//! beam-core: the single implementation of Beam's protocol, encryption and
//! transfer engine, shared by the CLI, the desktop app and the Android app.

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
