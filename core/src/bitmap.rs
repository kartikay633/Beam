//! A set of chunk indices, stored as the bitmap format used on the wire
//! (ACCEPT `chunk_bitmap`) and in progress files: bit `i` is byte `i / 8`,
//! bit `i % 8`, least significant bit first.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkBitmap {
    bits: Vec<u8>,
    len: u64,
    count: u64,
}

impl ChunkBitmap {
    /// An empty bitmap for `len` chunks.
    pub fn new(len: u64) -> ChunkBitmap {
        ChunkBitmap {
            bits: vec![0; len.div_ceil(8) as usize],
            len,
            count: 0,
        }
    }

    /// Rebuilds a bitmap for `len` chunks from its bytes. Returns `None` if
    /// the byte count is wrong or bits beyond `len` are set.
    pub fn from_bytes(len: u64, bytes: &[u8]) -> Option<ChunkBitmap> {
        if bytes.len() as u64 != len.div_ceil(8) {
            return None;
        }
        let mut bitmap = ChunkBitmap {
            bits: bytes.to_vec(),
            len,
            count: 0,
        };
        let tail_bits = len % 8;
        if tail_bits != 0 && bytes[bytes.len() - 1] >> tail_bits != 0 {
            return None;
        }
        bitmap.count = bitmap.bits.iter().map(|b| b.count_ones() as u64).sum();
        Some(bitmap)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bits
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Number of chunks marked present.
    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn is_complete(&self) -> bool {
        self.count == self.len
    }

    pub fn contains(&self, index: u64) -> bool {
        index < self.len && self.bits[(index / 8) as usize] & (1 << (index % 8)) != 0
    }

    /// Marks a chunk present. Returns `true` if it wasn't already.
    pub fn insert(&mut self, index: u64) -> bool {
        assert!(index < self.len, "chunk {index} out of range {}", self.len);
        let byte = &mut self.bits[(index / 8) as usize];
        let mask = 1 << (index % 8);
        if *byte & mask != 0 {
            return false;
        }
        *byte |= mask;
        self.count += 1;
        true
    }

    /// Marks a chunk missing again. Returns `true` if it was present.
    pub fn remove(&mut self, index: u64) -> bool {
        if !self.contains(index) {
            return false;
        }
        self.bits[(index / 8) as usize] &= !(1 << (index % 8));
        self.count -= 1;
        true
    }

    /// Chunk indices not yet present, in order.
    pub fn missing(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.len).filter(|&i| !self.contains(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_contains_count() {
        let mut bitmap = ChunkBitmap::new(10);
        assert!(!bitmap.is_complete());
        assert!(bitmap.insert(0));
        assert!(bitmap.insert(9));
        assert!(!bitmap.insert(9), "already present");
        assert!(bitmap.contains(9) && !bitmap.contains(5));
        assert!(!bitmap.contains(10), "out of range is never present");
        assert_eq!(bitmap.count(), 2);
        assert_eq!(bitmap.as_bytes(), [0b0000_0001, 0b0000_0010]);
        assert_eq!(
            bitmap.missing().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6, 7, 8]
        );
        assert!(bitmap.remove(9));
        assert!(!bitmap.remove(9), "already missing");
        assert_eq!(bitmap.count(), 1);
    }

    #[test]
    fn bytes_round_trip_and_are_validated() {
        let mut bitmap = ChunkBitmap::new(12);
        for i in [1, 3, 11] {
            bitmap.insert(i);
        }
        let copy = ChunkBitmap::from_bytes(12, bitmap.as_bytes()).unwrap();
        assert_eq!(copy, bitmap);

        assert!(ChunkBitmap::from_bytes(12, &[0]).is_none(), "too short");
        assert!(
            ChunkBitmap::from_bytes(12, &[0, 0b0001_0000]).is_none(),
            "bit 12 is past the end"
        );
    }

    #[test]
    fn empty_file_has_a_complete_empty_bitmap() {
        let bitmap = ChunkBitmap::new(0);
        assert!(bitmap.is_complete());
        assert!(bitmap.as_bytes().is_empty());
        assert_eq!(ChunkBitmap::from_bytes(0, &[]), Some(bitmap));
    }
}
