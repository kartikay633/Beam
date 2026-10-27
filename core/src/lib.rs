//! beam-core: the single implementation of Beam's protocol, encryption and
//! transfer engine, shared by the CLI, the desktop app and the Android app.

pub mod connection;
pub mod frame;
pub mod identity;
pub mod messages;
pub mod receiver;
pub mod sanitize;
pub mod secure;
pub mod sender;
pub mod transfer;
pub mod trust;

pub use connection::{Connection, ConnectionError, PlainCipher, RecordCipher};
pub use frame::{Frame, FrameError};
pub use identity::{Identity, IdentityError};
pub use messages::{DeviceId, DeviceType, PROTOCOL_VERSION};
pub use receiver::{
    receive_transfer, Decision, IncomingOffer, ReceiveOutcome, Receiver, ReceiverConfig, SavedFile,
};
pub use sanitize::sanitize_file_name;
pub use secure::{secure_connection, HandshakeInfo, NoiseCipher, Role};
pub use sender::{send_files, send_to, SendEvent, SendOutcome};
pub use transfer::{LocalDevice, PeerInfo, Progress, TransferError, TransferSummary, CHUNK_SIZE};
pub use trust::{TrustStore, TrustStoreError, TrustedDevice};

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
