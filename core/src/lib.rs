//! beam-core: the single implementation of Beam's protocol, encryption and
//! transfer engine, shared by the CLI, the desktop app and the Android app.

pub mod connection;
pub mod frame;
pub mod messages;
pub mod receiver;
pub mod sanitize;
pub mod sender;
pub mod transfer;

pub use connection::{Connection, ConnectionError, PlainCipher, RecordCipher};
pub use frame::{Frame, FrameError};
pub use messages::{DeviceId, DeviceType, PROTOCOL_VERSION};
pub use receiver::{
    receive_transfer, Decision, IncomingOffer, ReceiveOutcome, Receiver, ReceiverConfig, SavedFile,
};
pub use sanitize::sanitize_file_name;
pub use sender::{send_files, send_to, SendOutcome};
pub use transfer::{DeviceInfo, Progress, TransferError, TransferSummary, CHUNK_SIZE};

/// Version of this crate, used by front-ends to report what they are built on.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!VERSION.is_empty());
    }
}
