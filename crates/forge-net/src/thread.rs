//! The network thread (ARCHITECTURE.md, "Threads"; #268): a tokio runtime on a thread of its
//! own drives the sessions, so a simulation running its fixed tick never awaits.
//!
//! The simulation's thread starts it ([`NetThread::start`]), connects through it
//! ([`NetThread::connect`], which waits for the handshake: at start-up, never in a tick) or
//! serves through it ([`NetThread::serve`]: the listener takes each new client's session with
//! [`Listener::try_accept`], which never waits). From then on it holds plain [`Session`]s:
//! [`Session::send`] queues and [`Session::try_recv`] takes what arrived, neither waiting, while
//! the network thread does the I/O.
//!
//! Keep the listener as long as the sessions it handed over: dropping it closes the server's
//! endpoint, and every connection on it. Drop the sessions and the listener before the thread:
//! dropping the thread stops the runtime, and the connections with it.

use std::net::SocketAddr;
use std::sync::mpsc;

use crate::{Client, ClientConfig, ConnectError, Server, ServerConfig, Session, SetupError};

/// The network's runtime, on a thread of its own named `forge-net`.
pub struct NetThread {
    runtime: tokio::runtime::Runtime,
}

impl NetThread {
    /// Starts the runtime: one worker thread, its I/O and timers.
    pub fn start() -> std::io::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("forge-net")
            .enable_all()
            .build()?;
        Ok(Self { runtime })
    }

    /// Connects to the server at `address` with `token`, waiting for its answer.
    pub fn connect(
        &self,
        config: ClientConfig,
        address: SocketAddr,
        token: &[u8],
    ) -> Result<Session, ConnectError> {
        self.runtime
            .block_on(Client::connect(config, address, token))
    }

    /// Binds a server; the network thread accepts its clients for [`Listener::try_accept`].
    pub fn serve(&self, config: ServerConfig) -> Result<Listener, SetupError> {
        let _inside = self.runtime.enter();
        let mut server = Server::bind(config)?;
        let address = server.local_address();
        let (sessions, accepted) = mpsc::channel();
        let task = self.runtime.spawn(async move {
            while let Some(session) = server.accept().await {
                if sessions.send(session).is_err() {
                    break;
                }
            }
        });
        Ok(Listener {
            accepted,
            address,
            task,
        })
    }

    /// Runs `future` to its end on the network's runtime, the caller waiting (set-up and tests).
    pub fn block_on<F: std::future::Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

/// A server served by the network thread. Dropping it closes the server, and the sessions it
/// handed over with it.
pub struct Listener {
    accepted: mpsc::Receiver<Session>,
    address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Listener {
    /// Where it listens.
    pub fn local_address(&self) -> SocketAddr {
        self.address
    }

    /// The next client to have connected, if one has; never waits.
    pub fn try_accept(&self) -> Option<Session> {
        self.accepted.try_recv().ok()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // The server with it: its endpoint closes, and every connection on it.
        self.task.abort();
    }
}
