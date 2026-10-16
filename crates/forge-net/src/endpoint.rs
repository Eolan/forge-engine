//! Servers and clients: QUIC endpoints, TLS, and the handshake that makes a [`Session`].
//!
//! **TLS.** QUIC's handshake is TLS 1.3 (RFC 9001), here `rustls` with `ring`'s cryptography.
//! A server shows a certificate; a client trusts exactly the one it was given
//! ([`ClientConfig::server_certificate`]), as SSH pins a host key on first use (D-010's TOFU).
//! [`ServerIdentity::generate`] makes a self-signed one at run time; nothing is stored.
//!
//! **The handshake.** Once TLS is done, the client opens a bidirectional stream and sends its
//! [`Hello`]: the protocol version, its channels and its connection token. The server checks
//! them in that order and answers with a [`Welcome`] carrying the client's id, or closes the
//! connection with the [`Refusal`]'s code. Only then does either side open channels. A server
//! refuses a client that sends no hello within [`TransportSettings::handshake_timeout`].
//!
//! **Liveness.** Each end pings when it has sent nothing for
//! [`TransportSettings::keep_alive`], and closes the connection when it has heard nothing for
//! [`TransportSettings::idle_timeout`]: a peer that died or a link that was cut ends the session
//! with [`ConnectionError::TimedOut`] (QUIC's idle timeout, RFC 9000, 10.1).
//!
//! Endpoints are made inside a tokio runtime; their tasks run on it.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{ConnectionError, Endpoint, EndpointConfig, IdleTimeout, Runtime, TokioRuntime};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::sync::mpsc;

use crate::channel::Channels;
use crate::conditioner::ConditionerConfig;
use crate::protocol::{ALPN, PROTOCOL_VERSION, Refusal};
use crate::session::{Session, SessionSettings};
use crate::socket::{ConditionedSocket, Conditioner};
use crate::token::TokenVerifier;
use crate::wire::{Hello, Welcome};

/// Bytes a handshake message has at most.
const HANDSHAKE_LIMIT: usize = 4096;

/// A server's certificate and its private key.
pub struct ServerIdentity {
    certificate: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
}

impl Clone for ServerIdentity {
    fn clone(&self) -> Self {
        Self {
            certificate: self.certificate.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl std::fmt::Debug for ServerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerIdentity")
            .field("certificate", &self.certificate.len())
            .finish_non_exhaustive()
    }
}

/// Something in the network's setup failed.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// A certificate could not be made.
    #[error("certificate: {0}")]
    Certificate(#[from] rcgen::Error),
    /// TLS refused the configuration.
    #[error("TLS: {0}")]
    Tls(#[from] rustls::Error),
    /// QUIC refused the TLS configuration.
    #[error("QUIC's TLS: {0}")]
    QuicTls(String),
    /// A socket could not be bound.
    #[error("socket: {0}")]
    Io(#[from] std::io::Error),
    /// A setting is out of its range.
    #[error("setting out of range: {0}")]
    Setting(&'static str),
}

impl ServerIdentity {
    /// A self-signed certificate for `names` (as a client will ask for them, `localhost` on
    /// loopback) and its key, made now.
    pub fn generate(names: &[&str]) -> Result<Self, SetupError> {
        let names: Vec<String> = names.iter().map(|s| (*s).to_owned()).collect();
        let made = rcgen::generate_simple_self_signed(names)?;
        Ok(Self {
            certificate: made.cert.der().clone(),
            key: PrivatePkcs8KeyDer::from(made.signing_key.serialize_der()),
        })
    }

    /// The certificate's DER bytes: what a client pins.
    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }
}

/// QUIC's timers and buffers, the same on both ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportSettings {
    /// A connection that heard nothing for this long is closed.
    pub idle_timeout: Duration,
    /// A connection that sent nothing for this long pings.
    pub keep_alive: Duration,
    /// How long a server waits for a client's hello.
    pub handshake_timeout: Duration,
    /// Datagrams waiting to be sent, in bytes; past it the oldest are dropped.
    pub datagram_send_buffer: usize,
    /// The largest datagram sent, at most what the path allows (RFC 9221's datagrams are never
    /// fragmented, so 1200 bytes is the size every path takes).
    pub datagram_limit: usize,
    /// The most bytes a message may have.
    pub max_message: usize,
    /// How long the fragments of an unreliable message wait for the rest.
    pub fragment_timeout: Duration,
    /// Streams the peer may have open at once (each reliable unordered message is one).
    pub max_streams: u32,
    /// The UDP socket's send and receive buffers, in bytes.
    pub socket_buffer: usize,
}

impl Default for TransportSettings {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(10),
            keep_alive: Duration::from_secs(2),
            handshake_timeout: Duration::from_secs(5),
            datagram_send_buffer: 1 << 20,
            datagram_limit: 1200,
            max_message: 1 << 20,
            fragment_timeout: Duration::from_secs(1),
            max_streams: 256,
            socket_buffer: 4 << 20,
        }
    }
}

impl TransportSettings {
    fn quinn(&self) -> Result<Arc<quinn::TransportConfig>, SetupError> {
        let mut t = quinn::TransportConfig::default();
        t.max_idle_timeout(Some(
            IdleTimeout::try_from(self.idle_timeout)
                .map_err(|_| SetupError::Setting("idle_timeout"))?,
        ))
        .keep_alive_interval(Some(self.keep_alive))
        .datagram_send_buffer_size(self.datagram_send_buffer)
        .max_concurrent_uni_streams(self.max_streams.into());
        Ok(Arc::new(t))
    }

    fn session(&self, channels: Channels) -> SessionSettings {
        SessionSettings {
            channels,
            max_message: self.max_message,
            datagram_limit: self.datagram_limit,
            fragment_timeout: self.fragment_timeout,
        }
    }
}

/// The loopback address with any port: where tests and the headless runs bind.
pub fn loopback() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

/// Binds a UDP socket at `address` as an endpoint, conditioned if `conditioner` is given.
fn endpoint(
    address: SocketAddr,
    server: Option<quinn::ServerConfig>,
    transport: &TransportSettings,
    conditioner: Option<ConditionerConfig>,
) -> Result<(Endpoint, Option<Conditioner>), SetupError> {
    let runtime: Arc<dyn Runtime> = Arc::new(TokioRuntime);
    let socket = std::net::UdpSocket::bind(address)?;
    // The system's default receive buffer (64 KB on Windows) overflows under a burst of
    // datagrams before the endpoint's task reads them; reliable data is sent again, datagrams
    // are lost.
    let state = quinn::udp::UdpSocketState::new((&socket).into())?;
    state.set_recv_buffer_size((&socket).into(), transport.socket_buffer)?;
    state.set_send_buffer_size((&socket).into(), transport.socket_buffer)?;
    let socket = runtime.wrap_udp_socket(socket)?;
    let (socket, handle): (Arc<dyn quinn::AsyncUdpSocket>, _) = match conditioner {
        Some(config) => {
            let (socket, handle) = ConditionedSocket::wrap(socket, config);
            (socket, Some(handle))
        }
        None => (socket, None),
    };
    let endpoint =
        Endpoint::new_with_abstract_socket(EndpointConfig::default(), server, socket, runtime)?;
    Ok((endpoint, handle))
}

/// What a server is.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Where it listens.
    pub address: SocketAddr,
    /// Its certificate.
    pub identity: ServerIdentity,
    /// Checks the clients' tokens; carries the server's id.
    pub tokens: TokenVerifier,
    /// The channels its clients must declare.
    pub channels: Channels,
    /// QUIC's timers and buffers.
    pub transport: TransportSettings,
    /// A conditioner on what it sends, for tests.
    pub conditioner: Option<ConditionerConfig>,
}

impl ServerConfig {
    /// A server on loopback with the standard channels and default timers.
    pub fn loopback(identity: ServerIdentity, tokens: TokenVerifier) -> Self {
        Self {
            address: loopback(),
            identity,
            tokens,
            channels: Channels::standard(),
            transport: TransportSettings::default(),
            conditioner: None,
        }
    }
}

/// What happened at a server.
#[derive(Debug)]
pub enum ServerEvent {
    /// A client is in.
    Connected(Session),
    /// A client was refused.
    Refused {
        /// Its address.
        address: SocketAddr,
        /// Why.
        refusal: Refusal,
    },
}

/// A listening server (see the module's documentation).
#[derive(Debug)]
pub struct Server {
    endpoint: Endpoint,
    events: mpsc::UnboundedReceiver<ServerEvent>,
    conditioner: Option<Conditioner>,
}

impl Server {
    /// Binds and starts taking connections.
    pub fn bind(config: ServerConfig) -> Result<Self, SetupError> {
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(
            vec![config.identity.certificate.clone()],
            PrivateKeyDer::Pkcs8(config.identity.key.clone_key()),
        )?;
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let quic =
            QuicServerConfig::try_from(tls).map_err(|e| SetupError::QuicTls(e.to_string()))?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(quic));
        server.transport_config(config.transport.quinn()?);
        let (endpoint, conditioner) = endpoint(
            config.address,
            Some(server),
            &config.transport,
            config.conditioner,
        )?;
        let (tx, events) = mpsc::unbounded_channel();
        tokio::spawn(accept(
            endpoint.clone(),
            Arc::new(config),
            conditioner.clone(),
            tx,
        ));
        Ok(Self {
            endpoint,
            events,
            conditioner,
        })
    }

    /// Where it listens.
    pub fn local_address(&self) -> SocketAddr {
        self.endpoint
            .local_addr()
            .expect("a bound endpoint has an address")
    }

    /// The next connection or refusal; `None` once the server is closed.
    pub async fn next_event(&mut self) -> Option<ServerEvent> {
        self.events.recv().await
    }

    /// The next client to connect, passing over refusals.
    pub async fn accept(&mut self) -> Option<Session> {
        loop {
            if let ServerEvent::Connected(session) = self.next_event().await? {
                return Some(session);
            }
        }
    }

    /// The conditioner on what the server sends, if it has one.
    pub fn conditioner(&self) -> Option<&Conditioner> {
        self.conditioner.as_ref()
    }

    /// Closes every connection and stops listening.
    pub fn close(&self) {
        self.endpoint
            .close(crate::protocol::CLOSE_NORMAL.into(), b"server closed");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.close();
    }
}

/// The server's accept loop: a task per incoming connection, so a slow client holds up no
/// other.
async fn accept(
    endpoint: Endpoint,
    config: Arc<ServerConfig>,
    conditioner: Option<Conditioner>,
    events: mpsc::UnboundedSender<ServerEvent>,
) {
    while let Some(incoming) = endpoint.accept().await {
        let (config, conditioner, events) = (config.clone(), conditioner.clone(), events.clone());
        tokio::spawn(async move {
            let Ok(connection) = incoming.await else {
                return;
            };
            let address = connection.remote_address();
            let event = match greet(&connection, &config).await {
                Ok(welcome) => ServerEvent::Connected(Session::start(
                    connection,
                    config.transport.session(config.channels.clone()),
                    welcome.client_id,
                    welcome.server_id,
                    conditioner,
                    None,
                )),
                Err(refusal) => {
                    Session::refuse(&connection, refusal);
                    ServerEvent::Refused { address, refusal }
                }
            };
            let _ = events.send(event);
        });
    }
}

/// The server's side of the handshake: reads the hello, checks it, sends the welcome.
async fn greet(connection: &quinn::Connection, config: &ServerConfig) -> Result<Welcome, Refusal> {
    let read = async {
        let (send, mut recv) = connection.accept_bi().await.ok()?;
        let hello = recv.read_to_end(HANDSHAKE_LIMIT).await.ok()?;
        Some((send, hello))
    };
    let (mut send, hello) = tokio::time::timeout(config.transport.handshake_timeout, read)
        .await
        .ok()
        .flatten()
        .ok_or(Refusal::Timeout)?;
    let version = Hello::version_of(&hello).ok_or(Refusal::Malformed)?;
    if version != PROTOCOL_VERSION {
        return Err(Refusal::Version {
            server: PROTOCOL_VERSION,
            client: version,
        });
    }
    let hello = Hello::decode(&hello).map_err(|_| Refusal::Malformed)?;
    if hello.channels != config.channels.kinds() {
        return Err(Refusal::Channels);
    }
    let claims = config.tokens.verify(&hello.token).map_err(Refusal::Token)?;
    let welcome = Welcome {
        version: PROTOCOL_VERSION,
        client_id: claims.client_id,
        server_id: config.tokens.server_id(),
    };
    send.write_all(&welcome.encode())
        .await
        .map_err(|_| Refusal::Timeout)?;
    send.finish().map_err(|_| Refusal::Timeout)?;
    Ok(welcome)
}

/// What a client is.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Where it binds its socket.
    pub address: SocketAddr,
    /// The server's certificate it trusts, and no other.
    pub server_certificate: Vec<u8>,
    /// The name the certificate is for.
    pub server_name: String,
    /// The channels it declares.
    pub channels: Channels,
    /// QUIC's timers and buffers.
    pub transport: TransportSettings,
    /// A conditioner on what it sends, for tests.
    pub conditioner: Option<ConditionerConfig>,
    /// The protocol version it claims: [`PROTOCOL_VERSION`] but in tests.
    pub version: u16,
}

impl ClientConfig {
    /// A client on loopback that trusts `server_certificate` for `localhost`.
    pub fn loopback(server_certificate: &[u8]) -> Self {
        Self {
            address: loopback(),
            server_certificate: server_certificate.to_vec(),
            server_name: "localhost".to_owned(),
            channels: Channels::standard(),
            transport: TransportSettings::default(),
            conditioner: None,
            version: PROTOCOL_VERSION,
        }
    }
}

/// Why a client could not connect.
#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    /// The setup failed.
    #[error(transparent)]
    Setup(#[from] SetupError),
    /// QUIC would not start the connection.
    #[error("connect: {0}")]
    Connect(#[from] quinn::ConnectError),
    /// The connection failed or closed.
    #[error("connection: {0}")]
    Connection(#[from] ConnectionError),
    /// The server refused the client.
    #[error("refused: {0}")]
    Refused(Refusal),
    /// The server's answer did not parse.
    #[error("the server's welcome did not parse")]
    Malformed,
}

/// The error a failed read means: the server's refusal if it closed with one.
fn refusal_or(e: ConnectionError) -> ConnectError {
    if let ConnectionError::ApplicationClosed(close) = &e
        && let Some(r) = Refusal::from_close(close.error_code.into_inner(), &close.reason)
    {
        return ConnectError::Refused(r);
    }
    ConnectError::Connection(e)
}

/// A client's way in.
#[derive(Debug)]
pub struct Client;

impl Client {
    /// Connects to `server` with `token`, through TLS and the handshake.
    pub async fn connect(
        config: ClientConfig,
        server: SocketAddr,
        token: &[u8],
    ) -> Result<Session, ConnectError> {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(CertificateDer::from(config.server_certificate.clone()))
            .map_err(SetupError::from)?;
        let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(SetupError::from)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        tls.alpn_protocols = vec![ALPN.to_vec()];
        let quic =
            QuicClientConfig::try_from(tls).map_err(|e| SetupError::QuicTls(e.to_string()))?;
        let mut client = quinn::ClientConfig::new(Arc::new(quic));
        client.transport_config(config.transport.quinn()?);
        let (endpoint, conditioner) =
            endpoint(config.address, None, &config.transport, config.conditioner)?;
        let connection = endpoint
            .connect_with(client, server, &config.server_name)?
            .await?;
        let hello = Hello {
            version: config.version,
            channels: config.channels.kinds().to_vec(),
            token: token.to_vec(),
        };
        let (mut send, mut recv) = connection.open_bi().await?;
        let answer = async {
            send.write_all(&hello.encode()).await.ok()?;
            send.finish().ok()?;
            recv.read_to_end(HANDSHAKE_LIMIT).await.ok()
        }
        .await;
        let Some(answer) = answer else {
            // The server closed the connection: with its refusal, or for another reason.
            return Err(refusal_or(connection.closed().await));
        };
        let welcome = Welcome::decode(&answer).map_err(|_| ConnectError::Malformed)?;
        Ok(Session::start(
            connection,
            config.transport.session(config.channels),
            welcome.client_id,
            welcome.server_id,
            conditioner,
            Some(endpoint),
        ))
    }
}
