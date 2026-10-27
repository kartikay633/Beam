//! Visibility modes (SPEC.md section 1), like AirDrop's:
//!
//! - **Off**: not discoverable, and every offer is declined, even from a
//!   device that already knows the address.
//! - **Trusted only**: only trusted devices can recognize this device on the
//!   network (see [`crate::discovery`]) or send to it.
//! - **Everyone**: anyone nearby can see it and ask to send (the user still
//!   decides). Falls back to trusted-only after 10 minutes.

use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

/// How long "everyone" lasts before falling back to trusted-only.
pub const EVERYONE_FOR: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityMode {
    Off,
    TrustedOnly,
    Everyone,
}

impl VisibilityMode {
    /// The value used in the mDNS `vis` TXT record and on the command line.
    pub fn as_str(self) -> &'static str {
        match self {
            VisibilityMode::Off => "off",
            VisibilityMode::TrustedOnly => "trusted",
            VisibilityMode::Everyone => "everyone",
        }
    }
}

impl FromStr for VisibilityMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "off" => Ok(VisibilityMode::Off),
            "trusted" | "trusted-only" => Ok(VisibilityMode::TrustedOnly),
            "everyone" => Ok(VisibilityMode::Everyone),
            other => Err(format!(
                "unknown visibility {other:?} (use off, trusted or everyone)"
            )),
        }
    }
}

/// The current visibility, shared by the receiver and the advertiser and
/// changeable at any time (a tray menu, say). Cheap to clone.
#[derive(Clone, Debug)]
pub struct Visibility {
    state: Arc<Mutex<State>>,
    changes: Arc<watch::Sender<u64>>,
}

#[derive(Debug)]
struct State {
    mode: VisibilityMode,
    everyone_until: Option<Instant>,
    everyone_for: Duration,
}

impl Visibility {
    pub fn new(mode: VisibilityMode) -> Visibility {
        Self::with_everyone_duration(mode, EVERYONE_FOR)
    }

    /// Like [`Visibility::new`], with a custom "everyone" duration (tests).
    pub fn with_everyone_duration(mode: VisibilityMode, everyone_for: Duration) -> Visibility {
        let visibility = Visibility {
            state: Arc::new(Mutex::new(State {
                mode: VisibilityMode::Off,
                everyone_until: None,
                everyone_for,
            })),
            changes: Arc::new(watch::channel(0).0),
        };
        visibility.set(mode);
        visibility
    }

    /// Switches mode. Choosing "everyone" (again) restarts its 10 minutes.
    pub fn set(&self, mode: VisibilityMode) {
        {
            let mut state = self.lock();
            state.mode = mode;
            state.everyone_until =
                (mode == VisibilityMode::Everyone).then(|| Instant::now() + state.everyone_for);
        }
        self.changes.send_modify(|generation| *generation += 1);
    }

    /// The mode in effect now: "everyone" turns into trusted-only once its
    /// time is up.
    pub fn mode(&self) -> VisibilityMode {
        let mut state = self.lock();
        if state.mode == VisibilityMode::Everyone
            && state
                .everyone_until
                .is_some_and(|until| Instant::now() >= until)
        {
            state.mode = VisibilityMode::TrustedOnly;
            state.everyone_until = None;
        }
        state.mode
    }

    /// When "everyone" ends, if it's the current mode.
    pub fn everyone_until(&self) -> Option<Instant> {
        let _ = self.mode();
        self.lock().everyone_until
    }

    /// Notified on every explicit [`Visibility::set`] (not on expiry; use
    /// [`Visibility::everyone_until`] for that).
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_from_the_command_line() {
        assert_eq!("Everyone".parse(), Ok(VisibilityMode::Everyone));
        assert_eq!("trusted".parse(), Ok(VisibilityMode::TrustedOnly));
        assert_eq!("off".parse(), Ok(VisibilityMode::Off));
        assert!("public".parse::<VisibilityMode>().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn everyone_expires_into_trusted_only() {
        let visibility = Visibility::new(VisibilityMode::Everyone);
        assert_eq!(visibility.mode(), VisibilityMode::Everyone);
        assert!(visibility.everyone_until().is_some());

        tokio::time::advance(EVERYONE_FOR - Duration::from_secs(1)).await;
        assert_eq!(visibility.mode(), VisibilityMode::Everyone);
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(visibility.mode(), VisibilityMode::TrustedOnly);
        assert_eq!(visibility.everyone_until(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn choosing_everyone_again_restarts_the_clock() {
        let visibility = Visibility::new(VisibilityMode::Everyone);
        tokio::time::advance(EVERYONE_FOR - Duration::from_secs(1)).await;
        visibility.set(VisibilityMode::Everyone);
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(visibility.mode(), VisibilityMode::Everyone);
    }

    #[tokio::test]
    async fn changes_are_announced() {
        let visibility = Visibility::new(VisibilityMode::Off);
        let mut changes = visibility.subscribe();
        visibility.set(VisibilityMode::TrustedOnly);
        changes.changed().await.unwrap();
        assert_eq!(visibility.mode(), VisibilityMode::TrustedOnly);
    }
}
