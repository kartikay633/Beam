//! Moves frames over a byte stream, through pluggable record encryption.
//!
//! Wire format: a sequence of *records*, each `len:u16 (big-endian) | bytes`.
//! Each record is one Noise transport message (at most 65,535 bytes), made by
//! the sealer/opener pair from [`crate::secure`].
//!
//! Inside the records, each frame is prefixed with `len:u32` and may span
//! several records; a record never holds parts of two frames. DATA frames are
//! sized to fit in a single record. See docs/DECISIONS.md for why frames can
//! be larger than one record.
//!
//! A [`Connection`] can be split into a [`FrameReader`] and a [`FrameWriter`]
//! so one task can read while another writes. Each direction has its own
//! encryption state.

use tokio::io::{
    AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter, ReadHalf, WriteHalf,
};

use crate::frame::{encode_data, Frame, FrameError, DATA_HEADER_LEN};

/// Largest record on the wire: the Noise transport message limit.
pub const MAX_RECORD_LEN: usize = 65_535;
/// Bytes per record taken by the ChaCha20-Poly1305 tag.
pub const RECORD_OVERHEAD: usize = 16;
/// Largest plaintext a record can carry.
pub const MAX_RECORD_PLAINTEXT: usize = MAX_RECORD_LEN - RECORD_OVERHEAD;
/// Largest frame accepted (type byte + body). Bounds memory for hostile peers
/// while still allowing an OFFER with thousands of files.
pub const MAX_FRAME_LEN: usize = 4 * 1024 * 1024;
/// Bytes of file data per DATA frame ("~64 KiB"): chosen so that the length
/// prefix, type byte, DATA header and payload fit in one record.
pub const DATA_PAYLOAD_LEN: usize = 63 * 1024;

const FRAME_LEN_PREFIX: usize = 4;
const IO_BUFFER: usize = 256 * 1024;

const _: () =
    assert!(FRAME_LEN_PREFIX + 1 + DATA_HEADER_LEN + DATA_PAYLOAD_LEN <= MAX_RECORD_PLAINTEXT);

/// Protects outgoing records. Must add at most [`RECORD_OVERHEAD`] bytes.
pub trait RecordSealer: Send {
    /// Appends the protected form of `plaintext` to `out`.
    fn seal(&mut self, plaintext: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError>;
}

/// Checks and decrypts incoming records.
pub trait RecordOpener: Send {
    /// Appends the plaintext of `record` to `out`.
    fn open(&mut self, record: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError>;
}

#[derive(Debug, thiserror::Error)]
#[error("record cipher failed: {0}")]
pub struct CipherError(pub String);

/// No encryption: records carry the plaintext unchanged. Only for testing the
/// framing layer; transfers always use Noise.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainCipher;

impl RecordSealer for PlainCipher {
    fn seal(&mut self, plaintext: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError> {
        out.extend_from_slice(plaintext);
        Ok(())
    }
}

impl RecordOpener for PlainCipher {
    fn open(&mut self, record: &[u8], out: &mut Vec<u8>) -> Result<(), CipherError> {
        out.extend_from_slice(record);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("connection closed by peer")]
    Closed,
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Cipher(#[from] CipherError),
    #[error("frame of {0} bytes exceeds the {MAX_FRAME_LEN}-byte limit")]
    FrameTooLarge(usize),
    #[error("malformed record stream: {0}")]
    BadRecord(&'static str),
    #[error("secure handshake failed: {0}")]
    Handshake(String),
    #[error("timed out waiting for the {0}")]
    Timeout(&'static str),
}

/// Both halves of a connection.
pub struct Connection<S> {
    reader: FrameReader<ReadHalf<S>>,
    writer: FrameWriter<WriteHalf<S>>,
}

impl<S: AsyncRead + AsyncWrite + Send + 'static> Connection<S> {
    pub fn new(stream: S, sealer: Box<dyn RecordSealer>, opener: Box<dyn RecordOpener>) -> Self {
        let (read, write) = tokio::io::split(stream);
        Connection {
            reader: FrameReader::new(read, opener),
            writer: FrameWriter::new(write, sealer),
        }
    }

    /// An unencrypted connection, for tests of the framing layer.
    pub fn plain(stream: S) -> Self {
        Self::new(stream, Box::new(PlainCipher), Box::new(PlainCipher))
    }

    pub fn into_split(self) -> (FrameReader<ReadHalf<S>>, FrameWriter<WriteHalf<S>>) {
        (self.reader, self.writer)
    }

    pub async fn send(&mut self, frame: &Frame) -> Result<(), ConnectionError> {
        self.writer.send(frame).await
    }

    pub async fn write_frame(&mut self, frame: &Frame) -> Result<(), ConnectionError> {
        self.writer.write_frame(frame).await
    }

    pub async fn write_data(
        &mut self,
        file_id: u32,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), ConnectionError> {
        self.writer.write_data(file_id, offset, bytes).await
    }

    pub async fn flush(&mut self) -> Result<(), ConnectionError> {
        self.writer.flush().await
    }

    pub async fn read_frame(&mut self) -> Result<Frame, ConnectionError> {
        self.reader.read_frame().await
    }

    pub async fn shutdown(&mut self) -> Result<(), ConnectionError> {
        self.writer.shutdown().await
    }

    #[cfg(test)]
    pub(crate) fn ciphers_mut(&mut self) -> (&mut dyn RecordSealer, &mut dyn RecordOpener) {
        (self.writer.sealer.as_mut(), self.reader.opener.as_mut())
    }
}

/// The sending half: encodes frames, seals records, writes them.
pub struct FrameWriter<W> {
    stream: BufWriter<W>,
    sealer: Box<dyn RecordSealer>,
    /// Plaintext of the frame being written, including its length prefix.
    frame_buf: Vec<u8>,
    record_buf: Vec<u8>,
}

impl<W: AsyncWrite + Unpin> FrameWriter<W> {
    pub fn new(stream: W, sealer: Box<dyn RecordSealer>) -> Self {
        FrameWriter {
            stream: BufWriter::with_capacity(IO_BUFFER, stream),
            sealer,
            frame_buf: Vec::new(),
            record_buf: Vec::new(),
        }
    }

    /// Writes a frame and flushes it to the network.
    pub async fn send(&mut self, frame: &Frame) -> Result<(), ConnectionError> {
        self.write_frame(frame).await?;
        self.flush().await
    }

    /// Writes a frame into the output buffer without flushing.
    pub async fn write_frame(&mut self, frame: &Frame) -> Result<(), ConnectionError> {
        self.frame_buf.clear();
        self.frame_buf.extend_from_slice(&[0; FRAME_LEN_PREFIX]);
        frame.encode(&mut self.frame_buf)?;
        self.write_buffered_frame().await
    }

    /// Writes a DATA frame straight from a borrowed buffer (no extra copy into
    /// a `Data` value), without flushing.
    pub async fn write_data(
        &mut self,
        file_id: u32,
        offset: u64,
        bytes: &[u8],
    ) -> Result<(), ConnectionError> {
        self.frame_buf.clear();
        self.frame_buf.extend_from_slice(&[0; FRAME_LEN_PREFIX]);
        encode_data(file_id, offset, bytes, &mut self.frame_buf)?;
        self.write_buffered_frame().await
    }

    pub async fn flush(&mut self) -> Result<(), ConnectionError> {
        self.stream.flush().await?;
        Ok(())
    }

    /// Flushes and shuts down the stream so the peer sees a clean close.
    pub async fn shutdown(&mut self) -> Result<(), ConnectionError> {
        self.stream.flush().await?;
        self.stream.shutdown().await?;
        Ok(())
    }

    async fn write_buffered_frame(&mut self) -> Result<(), ConnectionError> {
        let frame_len = self.frame_buf.len() - FRAME_LEN_PREFIX;
        if frame_len > MAX_FRAME_LEN {
            return Err(ConnectionError::FrameTooLarge(frame_len));
        }
        self.frame_buf[..FRAME_LEN_PREFIX].copy_from_slice(&(frame_len as u32).to_be_bytes());

        for piece in self.frame_buf.chunks(MAX_RECORD_PLAINTEXT) {
            self.record_buf.clear();
            self.sealer.seal(piece, &mut self.record_buf)?;
            let record_len = u16::try_from(self.record_buf.len())
                .map_err(|_| ConnectionError::BadRecord("cipher produced an oversized record"))?;
            self.stream.write_all(&record_len.to_be_bytes()).await?;
            self.stream.write_all(&self.record_buf).await?;
        }
        Ok(())
    }
}

/// The receiving half: reads records, opens them, decodes frames.
pub struct FrameReader<R> {
    stream: BufReader<R>,
    opener: Box<dyn RecordOpener>,
    /// Plaintext of the frame being read, including its length prefix.
    frame_buf: Vec<u8>,
    record_buf: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(stream: R, opener: Box<dyn RecordOpener>) -> Self {
        FrameReader {
            stream: BufReader::with_capacity(IO_BUFFER, stream),
            opener,
            frame_buf: Vec::new(),
            record_buf: Vec::new(),
        }
    }

    /// Reads the next frame. Returns [`ConnectionError::Closed`] if the peer
    /// closed the connection cleanly between frames.
    ///
    /// Not cancel-safe: if the future is dropped halfway, the stream is out
    /// of sync and the connection must be abandoned.
    pub async fn read_frame(&mut self) -> Result<Frame, ConnectionError> {
        self.frame_buf.clear();
        let mut expected_len: Option<usize> = None;
        loop {
            let record_len = match self.stream.read_u16().await {
                Ok(len) => len as usize,
                Err(e)
                    if e.kind() == std::io::ErrorKind::UnexpectedEof && expected_len.is_none() =>
                {
                    return Err(ConnectionError::Closed)
                }
                Err(e) => return Err(e.into()),
            };
            self.record_buf.resize(record_len, 0);
            self.stream.read_exact(&mut self.record_buf).await?;
            self.opener.open(&self.record_buf, &mut self.frame_buf)?;

            let expected = match expected_len {
                Some(len) => len,
                None => {
                    let Some(prefix) = self.frame_buf.get(..FRAME_LEN_PREFIX) else {
                        return Err(ConnectionError::BadRecord(
                            "first record of a frame is shorter than the length prefix",
                        ));
                    };
                    let frame_len =
                        u32::from_be_bytes(prefix.try_into().expect("4 bytes")) as usize;
                    if frame_len > MAX_FRAME_LEN {
                        return Err(ConnectionError::FrameTooLarge(frame_len));
                    }
                    let total = FRAME_LEN_PREFIX + frame_len;
                    expected_len = Some(total);
                    total
                }
            };

            if self.frame_buf.len() == expected {
                return Ok(Frame::decode(&self.frame_buf[FRAME_LEN_PREFIX..])?);
            }
            if self.frame_buf.len() > expected {
                return Err(ConnectionError::BadRecord(
                    "record extends past the end of its frame",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;
    use tokio::io::duplex;

    fn big_offer(files: usize) -> Frame {
        Frame::Offer(Offer {
            transfer_id: uuid::Uuid::from_u128(42),
            files: (0..files)
                .map(|i| FileEntry {
                    file_id: i as u32,
                    name: format!("IMG_20261027_{i:06}_with_a_fairly_long_name.jpg"),
                    size: 3_000_000,
                    mime: "image/jpeg".into(),
                    modified_unix: 1_790_000_000,
                })
                .collect(),
            total_size: 3_000_000 * files as u64,
        })
    }

    #[tokio::test]
    async fn frames_larger_than_one_record_round_trip() {
        let (a, b) = duplex(1 << 20);
        let (mut tx, mut rx) = (Connection::plain(a), Connection::plain(b));
        let offer = big_offer(5_000);
        assert!(offer.to_bytes().unwrap().len() > 4 * MAX_RECORD_PLAINTEXT);

        let sent = offer.clone();
        let writer = tokio::spawn(async move {
            tx.send(&sent).await.unwrap();
            tx.send(&Frame::Ping(Ping { nonce: 9 })).await.unwrap();
        });
        assert_eq!(rx.read_frame().await.unwrap(), offer);
        assert_eq!(
            rx.read_frame().await.unwrap(),
            Frame::Ping(Ping { nonce: 9 })
        );
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn split_halves_read_and_write_at_the_same_time() {
        let (a, b) = duplex(1 << 16);
        let (mut a_read, mut a_write) = Connection::plain(a).into_split();
        let (mut b_read, mut b_write) = Connection::plain(b).into_split();

        // Both sides write many frames before reading any: this only works
        // because reading and writing happen in separate tasks.
        let a_sends = tokio::spawn(async move {
            for nonce in 0..2_000 {
                a_write.send(&Frame::Ping(Ping { nonce })).await.unwrap();
            }
        });
        let b_sends = tokio::spawn(async move {
            for nonce in 0..2_000 {
                b_write.send(&Frame::Pong(Ping { nonce })).await.unwrap();
            }
        });
        for nonce in 0..2_000 {
            assert_eq!(
                b_read.read_frame().await.unwrap(),
                Frame::Ping(Ping { nonce })
            );
            assert_eq!(
                a_read.read_frame().await.unwrap(),
                Frame::Pong(Ping { nonce })
            );
        }
        a_sends.await.unwrap();
        b_sends.await.unwrap();
    }

    #[tokio::test]
    async fn full_size_data_frame_fits_one_record() {
        let (a, b) = duplex(1 << 20);
        let mut tx = Connection::plain(a);
        let payload = vec![0xAB; DATA_PAYLOAD_LEN];
        tx.write_data(1, 0, &payload).await.unwrap();
        tx.shutdown().await.unwrap();

        let mut raw = Vec::new();
        let mut b = b;
        b.read_to_end(&mut raw).await.unwrap();
        let record_len = u16::from_be_bytes([raw[0], raw[1]]) as usize;
        assert_eq!(raw.len(), 2 + record_len, "exactly one record");
    }

    #[tokio::test]
    async fn oversized_frame_length_is_rejected_before_allocating() {
        let (mut a, b) = duplex(1024);
        let mut rx = Connection::plain(b);
        let mut record = Vec::new();
        record.extend_from_slice(&6u16.to_be_bytes());
        record.extend_from_slice(&u32::MAX.to_be_bytes());
        record.extend_from_slice(&[crate::frame::TYPE_PING, 0]);
        a.write_all(&record).await.unwrap();
        assert!(matches!(
            rx.read_frame().await,
            Err(ConnectionError::FrameTooLarge(_))
        ));
    }

    #[tokio::test]
    async fn clean_close_between_frames_is_reported_as_closed() {
        let (a, b) = duplex(1024);
        drop(a);
        let mut rx = Connection::plain(b);
        assert!(matches!(
            rx.read_frame().await,
            Err(ConnectionError::Closed)
        ));
    }

    #[tokio::test]
    async fn close_in_the_middle_of_a_frame_is_an_error() {
        let (mut a, b) = duplex(1024);
        a.write_all(&100u16.to_be_bytes()).await.unwrap();
        a.write_all(&[1, 2, 3]).await.unwrap();
        drop(a);
        let mut rx = Connection::plain(b);
        assert!(matches!(rx.read_frame().await, Err(ConnectionError::Io(_))));
    }
}
