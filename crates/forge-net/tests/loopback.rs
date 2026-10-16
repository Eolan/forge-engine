//! The transport on loopback, through the seeded link conditioner (issue #268, step 1).
//!
//! Every test binds 127.0.0.1 only. Waits are bounded by generous deadlines and no assertion
//! depends on a tight wall-clock time: the conditioner's fates come from its seed, the timer's
//! granularity (15.6 ms on Windows) only stretches the delays.

use std::collections::BTreeSet;
use std::future::Future;
use std::time::{Duration, Instant};

use bytes::Bytes;
use forge_net::{
    ChannelId, Channels, Client, ClientConfig, ConditionerConfig, ConnectError, ConnectionError,
    Delivery, PROTOCOL_VERSION, Refusal, Server, ServerConfig, ServerEvent, ServerIdentity,
    Session, TokenClaims, TokenError, TokenIssuer, TokenVerifier, TransportSettings,
};

const ORDERED: ChannelId = ChannelId(0);
const UNORDERED: ChannelId = ChannelId(1);
const UNRELIABLE: ChannelId = ChannelId(2);
const SERVER_ID: u64 = 77;

/// Runs a test's body under a deadline, so that a fault fails it instead of hanging CI.
async fn within<T>(seconds: u64, body: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(seconds), body)
        .await
        .expect("the test ran past its deadline")
}

/// A server and what a client needs to reach it.
struct Rig {
    issuer: TokenIssuer,
    certificate: Vec<u8>,
    server: Server,
}

impl Rig {
    fn new(conditioner: Option<ConditionerConfig>, transport: TransportSettings) -> Self {
        let issuer = TokenIssuer::generate().unwrap();
        let identity = ServerIdentity::generate(&["localhost"]).unwrap();
        let certificate = identity.certificate().to_vec();
        let mut config =
            ServerConfig::loopback(identity, TokenVerifier::new(issuer.public_key(), SERVER_ID));
        config.transport = transport;
        config.conditioner = conditioner;
        let server = Server::bind(config).unwrap();
        Self {
            issuer,
            certificate,
            server,
        }
    }

    fn perfect() -> Self {
        Self::new(None, TransportSettings::default())
    }

    fn token(&self, client_id: u64) -> Vec<u8> {
        self.issuer
            .issue(&TokenClaims::new(client_id, SERVER_ID, 60))
    }

    fn client_config(&self) -> ClientConfig {
        ClientConfig::loopback(&self.certificate)
    }

    /// A client through `config`, and the server's session for it.
    async fn connect(&mut self, config: ClientConfig, client_id: u64) -> (Session, Session) {
        let token = self.token(client_id);
        let address = self.server.local_address();
        let (client, server) = tokio::join!(
            Client::connect(config, address, &token),
            self.server.accept()
        );
        (client.unwrap(), server.unwrap())
    }

    /// The server's next event, which must be a refusal.
    async fn refusal(&mut self) -> Refusal {
        match self.server.next_event().await.unwrap() {
            ServerEvent::Refused { refusal, .. } => refusal,
            ServerEvent::Connected(s) => panic!("{s:?} was let in"),
        }
    }
}

/// A message numbered `k`, `len` bytes long.
fn numbered(k: u32, len: usize) -> Bytes {
    let mut v = vec![(k % 251) as u8; len.max(4)];
    v[..4].copy_from_slice(&k.to_le_bytes());
    v.into()
}

fn number(payload: &[u8]) -> u32 {
    u32::from_le_bytes(payload[..4].try_into().unwrap())
}

/// A bad link: 20 ms each way, 10 ms of jitter, 5 % lost, 2 % duplicated, 10 % held back 15 ms.
fn bad_link(seed: u64) -> ConditionerConfig {
    ConditionerConfig::new(20, 0.05, seed)
        .with_jitter(Duration::from_millis(10))
        .with_duplicate(0.02)
        .with_reorder(0.1, Duration::from_millis(15))
}

#[tokio::test]
async fn a_client_connects_and_each_channel_carries_messages_both_ways() {
    within(30, async {
        let mut rig = Rig::perfect();
        let (mut client, mut server) = rig.connect(rig.client_config(), 1234).await;
        assert_eq!(client.client_id(), 1234);
        assert_eq!(server.client_id(), 1234);
        assert_eq!(client.server_id(), SERVER_ID);
        for (k, channel) in [ORDERED, UNORDERED, UNRELIABLE].into_iter().enumerate() {
            client.send(channel, numbered(k as u32, 100)).unwrap();
            let m = server.recv().await.unwrap();
            assert_eq!((m.channel, number(&m.payload)), (channel, k as u32));
            server.send(channel, numbered(10 + k as u32, 50)).unwrap();
            let m = client.recv().await.unwrap();
            assert_eq!((m.channel, number(&m.payload)), (channel, 10 + k as u32));
        }
        assert!(client.send(ChannelId(9), Bytes::new()).is_err());
    })
    .await;
}

#[tokio::test]
async fn the_ordered_channel_keeps_its_order_under_loss_and_reordering() {
    within(60, async {
        let mut rig = Rig::new(Some(bad_link(11)), TransportSettings::default());
        let mut config = rig.client_config();
        config.conditioner = Some(bad_link(12));
        let (client, mut server) = rig.connect(config, 1).await;
        let n = 2000;
        for k in 0..n {
            client
                .send(ORDERED, numbered(k, 64 + (k as usize * 37) % 400))
                .unwrap();
        }
        for k in 0..n {
            let m = server.recv().await.unwrap();
            assert_eq!((m.channel, number(&m.payload)), (ORDERED, k));
        }
        let sent = client.conditioner().unwrap().stats();
        let echoed = rig.server.conditioner().unwrap().stats();
        assert!(
            sent.lost > 0 && sent.reordered > 0 && sent.duplicated > 0,
            "{sent:?}"
        );
        assert!(echoed.lost > 0, "{echoed:?}");
        assert!(client.stats().lost_packets > 0);
    })
    .await;
}

#[tokio::test]
async fn the_unordered_channel_delivers_everything_under_loss() {
    within(60, async {
        let mut rig = Rig::new(Some(bad_link(21)), TransportSettings::default());
        let mut config = rig.client_config();
        config.conditioner = Some(bad_link(22));
        let (client, mut server) = rig.connect(config, 1).await;
        let n = 300;
        for k in 0..n {
            client.send(UNORDERED, numbered(k, 200)).unwrap();
        }
        let mut seen = BTreeSet::new();
        while seen.len() < n as usize {
            let m = server.recv().await.unwrap();
            assert_eq!(m.channel, UNORDERED);
            assert!(seen.insert(number(&m.payload)), "a message came twice");
        }
        assert_eq!(seen, (0..n).collect());
    })
    .await;
}

/// Receives until nothing came for `quiet`.
async fn drain(session: &mut Session, quiet: Duration) -> Vec<Bytes> {
    let mut got = Vec::new();
    while let Ok(Some(m)) = tokio::time::timeout(quiet, session.recv()).await {
        got.push(m.payload);
    }
    got
}

#[tokio::test]
async fn unreliable_messages_drop_at_the_links_rate() {
    within(90, async {
        // Lost one way only, so that the acks all come back; a large send buffer, so that
        // QUIC's congestion control delays datagrams rather than dropping them, and the only
        // losses are the link's. 1000-byte messages: one a packet.
        let transport = TransportSettings {
            datagram_send_buffer: 8 << 20,
            ..TransportSettings::default()
        };
        let mut rig = Rig::new(None, transport);
        let mut config = rig.client_config();
        config.transport = transport;
        let loss = 0.1;
        config.conditioner = Some(ConditionerConfig::new(5, loss, 31));
        let (client, mut server) = rig.connect(config, 1).await;
        let n = 1000;
        for k in 0..n {
            client.send(UNRELIABLE, numbered(k, 1000)).unwrap();
        }
        let got = drain(&mut server, Duration::from_secs(3)).await;
        let numbers: BTreeSet<u32> = got.iter().map(|p| number(p)).collect();
        assert_eq!(numbers.len(), got.len(), "a datagram came twice");
        let delivered = got.len() as f64 / n as f64;
        let link = client.conditioner().unwrap().stats();
        let link_loss = link.lost as f64 / link.sent as f64;
        println!(
            "delivered {delivered:.3} of {n}; the link lost {link_loss:.3} of {} packets",
            link.sent
        );
        assert!(
            (delivered - (1.0 - loss)).abs() < 0.04,
            "delivered {delivered}, expected {}",
            1.0 - loss
        );
    })
    .await;
}

#[tokio::test]
async fn fragmented_messages_arrive_whole_or_not_at_all() {
    within(60, async {
        let mut rig = Rig::perfect();
        let (client, mut server) = rig.connect(rig.client_config(), 1).await;
        let len = 5000;
        assert!(len > client.datagram_payload());
        // In bursts of 25 kB: a lossless link lets QUIC's window grow past what a system with
        // small socket buffers (Linux caps them at `rmem_max`) holds before it is read.
        for k in 0..50 {
            client.send(UNRELIABLE, numbered(k, len)).unwrap();
            if k % 5 == 4 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        let got = drain(&mut server, Duration::from_secs(2)).await;
        assert_eq!(got.len(), 50, "{:?}", client.stats());
        for p in &got {
            assert_eq!(p, &numbered(number(p), len));
        }

        // Over a lossy link a message of five fragments arrives with them all: 0.9⁵ ≈ 0.59.
        let transport = TransportSettings {
            datagram_send_buffer: 8 << 20,
            ..TransportSettings::default()
        };
        let mut rig = Rig::new(None, transport);
        let mut config = rig.client_config();
        config.transport = transport;
        config.conditioner = Some(ConditionerConfig::new(5, 0.1, 41));
        let (client, mut server) = rig.connect(config, 1).await;
        let n = 300;
        for k in 0..n {
            client.send(UNRELIABLE, numbered(k, len)).unwrap();
        }
        let got = drain(&mut server, Duration::from_secs(3)).await;
        for p in &got {
            assert_eq!(p, &numbered(number(p), len), "a message arrived damaged");
        }
        let delivered = got.len() as f64 / n as f64;
        let fragments = len.div_ceil(client.datagram_payload() - 4) as i32;
        let expected = 0.9f64.powi(fragments);
        println!(
            "{fragments} fragments a message; delivered {delivered:.3}, expected {expected:.3}"
        );
        assert!((delivered - expected).abs() < 0.1, "delivered {delivered}");
        assert!(
            server.stats().dropped > 0,
            "no unfinished message was dropped"
        );
    })
    .await;
}

#[tokio::test]
async fn a_dead_peer_times_out() {
    within(60, async {
        let transport = TransportSettings {
            idle_timeout: Duration::from_millis(1000),
            keep_alive: Duration::from_millis(200),
            ..TransportSettings::default()
        };
        let link = ConditionerConfig::perfect(51);
        let mut rig = Rig::new(Some(link), transport);
        let mut config = rig.client_config();
        config.transport = transport;
        config.conditioner = Some(link);
        let (client, server) = rig.connect(config, 1).await;
        // The server goes silent: everything it sends is lost, and so is everything sent to it.
        rig.server.conditioner().unwrap().cut(true, link);
        client.conditioner().unwrap().cut(true, link);
        let cut = Instant::now();
        let (on_client, on_server) = tokio::join!(client.closed(), server.closed());
        let waited = cut.elapsed();
        assert_eq!(on_client, ConnectionError::TimedOut);
        assert_eq!(on_server, ConnectionError::TimedOut);
        assert!(
            waited >= Duration::from_millis(900),
            "closed after {waited:?}"
        );
        assert!(client.send(ORDERED, Bytes::from_static(b"late")).is_err());
    })
    .await;
}

#[tokio::test]
async fn keep_alives_hold_a_quiet_session_open() {
    within(60, async {
        let transport = TransportSettings {
            idle_timeout: Duration::from_millis(1200),
            keep_alive: Duration::from_millis(250),
            ..TransportSettings::default()
        };
        let mut rig = Rig::new(None, transport);
        let mut config = rig.client_config();
        config.transport = transport;
        let (client, mut server) = rig.connect(config, 1).await;
        // Three idle timeouts without a message.
        tokio::time::sleep(Duration::from_millis(3600)).await;
        assert!(client.close_reason().is_none());
        client.send(ORDERED, numbered(7, 10)).unwrap();
        assert_eq!(number(&server.recv().await.unwrap().payload), 7);
    })
    .await;
}

#[tokio::test]
async fn a_client_of_another_version_is_refused_and_told_why() {
    within(30, async {
        let mut rig = Rig::perfect();
        let mut config = rig.client_config();
        config.version = PROTOCOL_VERSION + 1;
        let token = rig.token(1);
        let address = rig.server.local_address();
        let (result, refusal) =
            tokio::join!(Client::connect(config, address, &token), rig.refusal());
        let expected = Refusal::Version {
            server: PROTOCOL_VERSION,
            client: PROTOCOL_VERSION + 1,
        };
        assert_eq!(refusal, expected);
        match result {
            Err(ConnectError::Refused(r)) => assert_eq!(r, expected),
            other => panic!("expected a refusal, got {other:?}"),
        }
    })
    .await;
}

#[tokio::test]
async fn bad_tokens_and_other_channels_are_refused() {
    within(30, async {
        let mut rig = Rig::perfect();
        let address = rig.server.local_address();
        let forger = TokenIssuer::generate().unwrap();
        let expired = rig.issuer.issue(&TokenClaims {
            client_id: 1,
            issued: 1000,
            expires: 2000,
            servers: vec![SERVER_ID],
            user_data: Vec::new(),
        });
        let cases = [
            (
                forger.issue(&TokenClaims::new(1, SERVER_ID, 60)),
                TokenError::BadSignature,
            ),
            (expired, TokenError::Expired),
            (
                rig.issuer.issue(&TokenClaims::new(1, SERVER_ID + 1, 60)),
                TokenError::WrongServer,
            ),
            (b"garbage".to_vec(), TokenError::Malformed),
        ];
        for (token, error) in cases {
            let (result, refusal) = tokio::join!(
                Client::connect(rig.client_config(), address, &token),
                rig.refusal()
            );
            assert_eq!(refusal, Refusal::Token(error));
            match result {
                Err(ConnectError::Refused(r)) => assert_eq!(r, Refusal::Token(error)),
                other => panic!("expected a refusal, got {other:?}"),
            }
        }
        let mut config = rig.client_config();
        config.channels = Channels::new(&[Delivery::Unreliable]);
        let token = rig.token(1);
        let (result, refusal) =
            tokio::join!(Client::connect(config, address, &token), rig.refusal());
        assert_eq!(refusal, Refusal::Channels);
        assert!(matches!(
            result,
            Err(ConnectError::Refused(Refusal::Channels))
        ));
        // The server still takes a good client after all that.
        let (client, _server) = rig.connect(rig.client_config(), 9).await;
        assert_eq!(client.client_id(), 9);
    })
    .await;
}

#[tokio::test]
async fn a_client_that_does_not_trust_the_certificate_never_reaches_the_handshake() {
    within(30, async {
        let mut rig = Rig::perfect();
        let other = ServerIdentity::generate(&["localhost"]).unwrap();
        let config = ClientConfig::loopback(other.certificate());
        let token = rig.token(1);
        let result = Client::connect(config, rig.server.local_address(), &token).await;
        assert!(
            matches!(result, Err(ConnectError::Connection(_))),
            "{result:?}"
        );
        // The server saw no hello: nothing to accept or refuse.
        let next = tokio::time::timeout(Duration::from_millis(500), rig.server.next_event()).await;
        assert!(next.is_err(), "{next:?}");
    })
    .await;
}
