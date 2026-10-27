//! The receiving side: a server that handles any number of connections at
//! once. After the Noise handshake, the first frame says what a connection is:
//!
//! - **HELLO**: a control connection. HELLO → OFFER → ask the user (unless
//!   the sender is trusted with auto-accept, or this is a resumed transfer) →
//!   ACCEPT → receive → TRANSFER_COMPLETE. See [`session`].
//! - **STREAM_OPEN**: a data connection for a running transfer. It must carry
//!   that session's token and prove the same sender key as the control
//!   connection. Its chunks go through [`stream`] and [`writer`].

mod manifest;
mod session;

pub(crate) use manifest::now_unix;
mod stream;
mod writer;

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

use crate::connection::{Connection, ConnectionError};
use crate::frame::Frame;
use crate::messages::{FileEntry, StreamOpen};
use crate::secure::{secure_connection, HandshakeInfo, Role};
use crate::transfer::{LocalDevice, PeerInfo, Progress, TransferError, TransferSummary};
use crate::trust::TrustStore;
use crate::visibility::Visibility;

/// A new connection must say what it is within this long.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a resumed transfer waits for the old session to wind down.
const TAKEOVER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    pub device: LocalDevice,
    pub download_dir: PathBuf,
    pub trust: TrustStore,
    /// Who may send right now; checked for every offer.
    pub visibility: Visibility,
}

/// What the user is asked to accept. File names are already sanitized.
#[derive(Debug, Clone)]
pub struct IncomingOffer {
    pub transfer_id: Uuid,
    pub from: PeerInfo,
    pub files: Vec<FileEntry>,
    pub total_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject(String),
}

#[derive(Debug, Clone)]
pub struct SavedFile {
    pub file_id: u32,
    pub path: PathBuf,
    pub size: u64,
}

#[derive(Debug)]
pub enum ReceiveOutcome {
    Completed {
        from: PeerInfo,
        /// True when the sender is trusted with auto-accept, so nobody was asked.
        auto_accepted: bool,
        /// True when this session continued an earlier, interrupted one.
        resumed: bool,
        files: Vec<SavedFile>,
        summary: TransferSummary,
    },
    Rejected {
        from: PeerInfo,
        /// The user's reason, or why the visibility mode refused the offer.
        reason: String,
    },
}

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// How a front-end (CLI, desktop, Android) takes part in receiving.
pub trait ReceiveHandler: Send + Sync + 'static {
    /// Asked for every offer except resumed ones and those from a trusted
    /// device with auto-accept on. Unknown devices are never accepted
    /// automatically.
    fn decide(&self, offer: IncomingOffer) -> BoxFuture<Decision>;

    fn progress(&self, _transfer_id: Uuid, _progress: Progress) {}

    /// Every offer that got as far as a transfer id ends here exactly once.
    fn finished(&self, _transfer_id: Uuid, _result: &Result<ReceiveOutcome, TransferError>) {}

    /// A connection was dropped before it became a transfer: a failed
    /// handshake, an impersonation attempt, a bad data stream.
    fn connection_refused(&self, _error: &TransferError) {}
}

/// A TCP listener that receives transfers.
pub struct Receiver {
    listener: TcpListener,
    shared: Arc<Shared>,
}

impl Receiver {
    pub async fn bind(addr: impl ToSocketAddrs, config: ReceiverConfig) -> io::Result<Self> {
        Ok(Receiver {
            listener: TcpListener::bind(addr).await?,
            shared: Arc::new(Shared {
                config,
                routes: Mutex::default(),
                active: Mutex::default(),
                next_generation: AtomicU64::new(0),
            }),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn config(&self) -> &ReceiverConfig {
        &self.shared.config
    }

    /// Serves connections until the task is dropped.
    pub async fn run(self, handler: impl ReceiveHandler) -> io::Result<()> {
        let handler: Arc<dyn ReceiveHandler> = Arc::new(handler);
        manifest::remove_stale(&self.shared.config.download_dir, &[]);
        loop {
            let (stream, _) = match self.listener.accept().await {
                Ok(accepted) => accepted,
                // E.g. too many open files: wait a moment instead of spinning.
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let _ = stream.set_nodelay(true);
            tokio::spawn(serve_connection(
                stream,
                self.shared.clone(),
                handler.clone(),
            ));
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    shared: Arc<Shared>,
    handler: Arc<dyn ReceiveHandler>,
) {
    let identity = &shared.config.device.identity;
    let (mut conn, handshake) = match secure_connection(stream, identity, Role::Responder).await {
        Ok(secured) => secured,
        Err(e) => return handler.connection_refused(&e.into()),
    };
    let first = match tokio::time::timeout(FIRST_FRAME_TIMEOUT, conn.read_frame()).await {
        Ok(Ok(frame)) => frame,
        Ok(Err(e)) => return handler.connection_refused(&e.into()),
        Err(_) => {
            return handler.connection_refused(&ConnectionError::Timeout("first frame").into())
        }
    };
    match first {
        Frame::Hello(hello) => session::run_control(conn, hello, handshake, shared, handler).await,
        Frame::StreamOpen(open) => {
            if let Err(e) = shared.attach_data_stream(conn, open, &handshake).await {
                handler.connection_refused(&e);
            }
        }
        other => {
            handler.connection_refused(&TransferError::unexpected("HELLO or STREAM_OPEN", &other))
        }
    }
}

/// State shared by every connection of one receiver.
pub(crate) struct Shared {
    pub config: ReceiverConfig,
    /// Session token → where that session's data connections go.
    routes: Mutex<HashMap<[u8; 32], DataRoute>>,
    /// Transfers with a running session.
    active: Mutex<HashMap<Uuid, ActiveEntry>>,
    next_generation: AtomicU64,
}

#[derive(Clone)]
struct DataRoute {
    transfer_id: Uuid,
    sender_key: [u8; 32],
    connections: mpsc::Sender<Connection<TcpStream>>,
}

struct ActiveEntry {
    generation: u64,
    sender_key: [u8; 32],
    cancel: watch::Sender<bool>,
    finished: watch::Receiver<bool>,
}

impl Shared {
    fn lock_routes(&self) -> MutexGuard<'_, HashMap<[u8; 32], DataRoute>> {
        self.routes.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_active(&self) -> MutexGuard<'_, HashMap<Uuid, ActiveEntry>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn active_ids(&self) -> Vec<Uuid> {
        self.lock_active().keys().copied().collect()
    }

    async fn attach_data_stream(
        &self,
        conn: Connection<TcpStream>,
        open: StreamOpen,
        handshake: &HandshakeInfo,
    ) -> Result<(), TransferError> {
        let route = self.lock_routes().get(&open.session_token).cloned();
        let Some(route) = route else {
            return Err(TransferError::BadDataStream("unknown session token".into()));
        };
        if route.transfer_id != open.transfer_id || route.sender_key != handshake.remote_public_key
        {
            return Err(TransferError::BadDataStream(
                "data connection does not match its session".into(),
            ));
        }
        route
            .connections
            .send(conn)
            .await
            .map_err(|_| TransferError::BadDataStream("session already ended".into()))
    }

    /// Registers where data connections for a session go, until the guard drops.
    pub(crate) fn add_route(
        self: &Arc<Self>,
        token: [u8; 32],
        transfer_id: Uuid,
        sender_key: [u8; 32],
    ) -> (RouteGuard, mpsc::Receiver<Connection<TcpStream>>) {
        let (tx, rx) = mpsc::channel(crate::transfer::MAX_STREAMS);
        self.lock_routes().insert(
            token,
            DataRoute {
                transfer_id,
                sender_key,
                connections: tx,
            },
        );
        (
            RouteGuard {
                shared: self.clone(),
                token,
            },
            rx,
        )
    }

    /// Marks `transfer_id` as running in this session. If an older session
    /// of the same transfer is still around (the sender reconnected before we
    /// noticed the drop), it is told to stop, and we wait until it has saved
    /// its progress.
    pub(crate) async fn claim(
        self: &Arc<Self>,
        transfer_id: Uuid,
        sender_key: [u8; 32],
    ) -> Result<ActiveGuard, TransferError> {
        loop {
            let (generation, mut finished) = {
                let mut active = self.lock_active();
                match active.get(&transfer_id) {
                    Some(entry) if entry.sender_key != sender_key => {
                        return Err(TransferError::InvalidOffer(
                            "transfer id already in use by another device".into(),
                        ))
                    }
                    Some(entry) => {
                        let _ = entry.cancel.send(true);
                        (entry.generation, entry.finished.clone())
                    }
                    None => {
                        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
                        let (cancel_tx, cancel_rx) = watch::channel(false);
                        let (finished_tx, finished_rx) = watch::channel(false);
                        active.insert(
                            transfer_id,
                            ActiveEntry {
                                generation,
                                sender_key,
                                cancel: cancel_tx,
                                finished: finished_rx,
                            },
                        );
                        return Ok(ActiveGuard {
                            shared: self.clone(),
                            transfer_id,
                            generation,
                            finished: finished_tx,
                            cancel: cancel_rx,
                        });
                    }
                }
            };
            let _ = tokio::time::timeout(TAKEOVER_TIMEOUT, finished.wait_for(|done| *done)).await;
            // If the old session is stuck, stop waiting for it.
            let mut active = self.lock_active();
            if active
                .get(&transfer_id)
                .is_some_and(|e| e.generation == generation)
            {
                active.remove(&transfer_id);
            }
        }
    }
}

pub(crate) struct RouteGuard {
    shared: Arc<Shared>,
    token: [u8; 32],
}

impl Drop for RouteGuard {
    fn drop(&mut self) {
        self.shared.lock_routes().remove(&self.token);
    }
}

pub(crate) struct ActiveGuard {
    shared: Arc<Shared>,
    transfer_id: Uuid,
    generation: u64,
    finished: watch::Sender<bool>,
    /// Becomes `true` when a newer session takes over.
    pub cancel: watch::Receiver<bool>,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut active = self.shared.lock_active();
        if active
            .get(&self.transfer_id)
            .is_some_and(|e| e.generation == self.generation)
        {
            active.remove(&self.transfer_id);
        }
        drop(active);
        let _ = self.finished.send(true);
    }
}
