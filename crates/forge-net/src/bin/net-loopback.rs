//! `net-loopback`: a server and headless clients on loopback through the link conditioner,
//! measuring what the transport costs (issue #268; `docs/demos/net.md`).
//!
//! The server sends each client a message of `--payload` bytes `--rate` times a second on the
//! unreliable channel (a snapshot's stand-in); each client sends `--input` bytes `--input-rate`
//! times a second (its inputs). With `--replicate N`, the messages are real snapshots of a
//! crowd of N people (`forge_net::crowd`) under the replicator's byte budget, and each input
//! starts with the client's acknowledgement. Both directions go through a conditioner of
//! `--latency` ms one way, a tenth of it as jitter, and `--loss` of the packets lost. At the end
//! it prints, per client, the bytes on the wire each way (QUIC's headers, acks and encryption
//! included), the messages delivered and the round trip QUIC measured.
//!
//! ```text
//! cargo run --release -p forge-net --bin net-loopback -- --clients 8 --seconds 10 \
//!     --rate 30 --payload 800 --latency 50 --loss 0.02
//! cargo run --release -p forge-net --bin net-loopback -- --clients 100 --replicate 3000
//! ```
//!
//! It binds 127.0.0.1 only.

use std::time::{Duration, Instant};

use glam::DVec3;

use forge_core::SplitMix64;
use forge_net::crowd::{Crowd, Person};
use forge_net::layer::{self, LayerHost};
use forge_net::{
    Ack, ChannelId, Client, ClientConfig, ConditionerConfig, ReplicationConfig, Replicator, Server,
    ServerConfig, ServerIdentity, Session, SnapshotReceiver, TokenClaims, TokenIssuer,
    TokenVerifier,
};
use forge_sim::authority::{Layer, Strips, WorkerId};
use tokio::time::MissedTickBehavior;

const UNRELIABLE: ChannelId = ChannelId(2);
const SERVER_ID: u64 = 1;
/// The replication layer's endpoint for its workers (`--workers`).
const LAYER_ID: u64 = 2;

#[derive(Clone, Copy, Debug)]
struct Options {
    clients: usize,
    seconds: f64,
    rate: f64,
    payload: usize,
    input_rate: f64,
    input: usize,
    latency: u64,
    loss: f64,
    seed: u64,
    /// People in the replicated crowd; 0 for fixed payloads.
    replicate: usize,
    /// The crowd's square, metres.
    side: f64,
    /// A snapshot's budget, bytes.
    budget: usize,
    /// With `--replicate`: the crowd stepped at 60 Hz by this many workers, each owning a strip
    /// of the square, through the replication layer (`forge_net::layer`); 0 steps it here.
    workers: usize,
}

fn num<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} {value}: not a number"))
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut o = Self {
            clients: 8,
            seconds: 10.0,
            rate: 30.0,
            payload: 800,
            input_rate: 60.0,
            input: 64,
            latency: 50,
            loss: 0.02,
            seed: 1,
            replicate: 0,
            side: 600.0,
            budget: ReplicationConfig::default().packet_budget,
            workers: 0,
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut k = 0;
        while k < args.len() {
            let value = args
                .get(k + 1)
                .ok_or_else(|| format!("{} needs a value", args[k]))?;
            let flag = args[k].as_str();
            match flag {
                "--clients" => o.clients = num(flag, value)?,
                "--seconds" => o.seconds = num(flag, value)?,
                "--rate" => o.rate = num(flag, value)?,
                "--payload" => o.payload = num(flag, value)?,
                "--input-rate" => o.input_rate = num(flag, value)?,
                "--input" => o.input = num(flag, value)?,
                "--latency" => o.latency = num(flag, value)?,
                "--loss" => o.loss = num(flag, value)?,
                "--seed" => o.seed = num(flag, value)?,
                "--replicate" => o.replicate = num(flag, value)?,
                "--side" => o.side = num(flag, value)?,
                "--budget" => o.budget = num(flag, value)?,
                "--workers" => o.workers = num(flag, value)?,
                other => return Err(format!("unknown option {other}")),
            }
            k += 2;
        }
        Ok(o)
    }
}

/// What one client saw.
#[derive(Clone, Copy, Debug, Default)]
struct Report {
    received: u64,
    rtt: Duration,
    /// Wire bytes received and sent.
    wire_in: u64,
    wire_out: u64,
    /// UDP datagrams received and sent.
    datagrams_in: u64,
    datagrams_out: u64,
    payload_in: u64,
    payload_out: u64,
    /// Snapshots that did not decode (none expected).
    bad: u64,
    /// Entities the client held at the end.
    held: usize,
}

async fn run_client(
    o: Options,
    k: usize,
    certificate: Vec<u8>,
    token: Vec<u8>,
    server: std::net::SocketAddr,
    start: Instant,
) -> Report {
    let mut config = ClientConfig::loopback(&certificate);
    config.conditioner = Some(ConditionerConfig::new(
        o.latency,
        o.loss,
        o.seed.wrapping_mul(1000) + k as u64,
    ));
    let mut session = Client::connect(config, server, &token)
        .await
        .expect("the client could not connect");
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / o.input_rate));
    tick.set_missed_tick_behavior(MissedTickBehavior::Burst);
    let mut input = vec![0x5Au8; o.input.max(6)];
    let mut receiver = SnapshotReceiver::new(ReplicationConfig::default().quantization);
    let end = start + Duration::from_secs_f64(o.seconds);
    // The bytes are counted from `start` to `end`; the messages the server sent before `end`
    // are counted until they have had time to arrive.
    let tail = end + Duration::from_millis(3 * o.latency + 200);
    let (mut received, mut bad, mut first, mut last) = (0, 0, None, None);
    loop {
        let now = Instant::now();
        if first.is_none() && now >= start {
            first = Some(session.stats());
        }
        if last.is_none() && now >= end {
            last = Some(session.stats());
        }
        if now >= tail {
            break;
        }
        tokio::select! {
            _ = tick.tick() => {
                if now < end {
                    if o.replicate > 0 && let Some(ack) = receiver.ack() {
                        input[..6].copy_from_slice(&ack.encode());
                    }
                    let _ = session.send(UNRELIABLE, input.clone());
                }
            }
            m = session.recv() => match m {
                Some(m) => {
                    received += 1;
                    if o.replicate > 0 && receiver.receive(&m.payload).is_err() {
                        bad += 1;
                    }
                }
                None => break,
            },
            _ = tokio::time::sleep_until(tail.into()) => {}
        }
    }
    let (a, b) = (first.unwrap_or_default(), last.unwrap_or_default());
    Report {
        received,
        rtt: b.rtt,
        wire_in: b.udp_received.1 - a.udp_received.1,
        wire_out: b.udp_sent.1 - a.udp_sent.1,
        datagrams_in: b.udp_received.0 - a.udp_received.0,
        datagrams_out: b.udp_sent.0 - a.udp_sent.0,
        payload_in: b.messages_received.1 - a.messages_received.1,
        payload_out: b.messages_sent.1 - a.messages_sent.1,
        bad,
        held: receiver.view().len(),
    }
}

/// The server's crowd and replicator, in `--replicate` mode.
struct Replication {
    crowd: Crowd,
    /// With `--workers`: the replication layer and its workers stepping the crowd, the handoffs
    /// they made and the time the layer's ticks took.
    layer: Option<LayerHost<Person>>,
    handoffs: usize,
    layer_time: Duration,
    layer_ticks: u64,
    replicator: Replicator,
    random: SplitMix64,
    half: f64,
    /// Encoding time and snapshots, for the CPU line.
    encode: Duration,
    world: Duration,
    snapshots: u64,
    ticks: u64,
}

async fn serve(
    o: Options,
    mut server: Server,
    start: Instant,
    layer: Option<LayerHost<Person>>,
) -> (u64, Option<Replication>) {
    let mut sessions: Vec<Session> = Vec::with_capacity(o.clients);
    let mut replication = (o.replicate > 0).then(|| Replication {
        crowd: Crowd::new(o.replicate, o.side, o.seed),
        layer,
        handoffs: 0,
        layer_time: Duration::ZERO,
        layer_ticks: 0,
        replicator: Replicator::new(ReplicationConfig {
            packet_budget: o.budget,
            ..ReplicationConfig::default()
        }),
        random: SplitMix64::new(o.seed ^ 0x5EED),
        half: o.side / 2.0,
        encode: Duration::ZERO,
        world: Duration::ZERO,
        snapshots: 0,
        ticks: 0,
    });
    while sessions.len() < o.clients {
        let s = server.accept().await.expect("the server closed");
        if let Some(r) = &mut replication {
            let focus = DVec3::new(
                r.random.range_f64(-r.half, r.half),
                0.0,
                r.random.range_f64(-r.half, r.half),
            );
            r.replicator.add_client(s.client_id(), focus);
        }
        sessions.push(s);
    }
    let period = Duration::from_secs_f64(1.0 / o.rate);
    let mut tick = tokio::time::interval_at(start.into(), period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Burst);
    let payload = vec![0xA5u8; o.payload];
    let end = start + Duration::from_secs_f64(o.seconds);
    let mut sent = 0;
    let mut server_tick = 0u32;
    loop {
        tick.tick().await;
        if Instant::now() >= end {
            break;
        }
        server_tick += 1;
        if let Some(r) = &mut replication {
            let t = Instant::now();
            match &mut r.layer {
                // The layer's ticks at 60 Hz, its workers stepping the crowd.
                Some(host) => {
                    let steps = (60.0 / o.rate).round().max(1.0) as usize;
                    for _ in 0..steps {
                        let l = Instant::now();
                        let report = host.tick(Duration::from_secs(5)).await;
                        r.layer_time += l.elapsed();
                        r.layer_ticks += 1;
                        r.handoffs += report.handoffs.len();
                    }
                    let states: Vec<_> = host
                        .layer()
                        .records()
                        .iter()
                        .map(|(&id, record)| (id, record.state.state(id)))
                        .collect();
                    r.replicator
                        .set_world(states.iter().map(|(id, s)| (*id, s, 1.0)));
                }
                None => {
                    r.crowd.step(1.0 / o.rate);
                    r.replicator
                        .set_world(r.crowd.states().iter().map(|(id, s)| (*id, s, 1.0)));
                }
            }
            r.world += t.elapsed();
            r.ticks += 1;
        }
        for s in &mut sessions {
            while let Some(m) = s.try_recv() {
                if let Some(r) = &mut replication
                    && let Some(ack) = Ack::decode(&m.payload)
                {
                    r.replicator.acknowledge(s.client_id(), ack);
                }
            }
            let message = match &mut replication {
                Some(r) => {
                    let t = Instant::now();
                    let (bytes, _) = r
                        .replicator
                        .snapshot(s.client_id(), server_tick)
                        .expect("a known client");
                    r.encode += t.elapsed();
                    r.snapshots += 1;
                    bytes
                }
                None => payload.clone(),
            };
            if s.send(UNRELIABLE, message).is_ok() {
                sent += 1;
            }
        }
    }
    // Keep the sessions until the clients have stopped counting.
    tokio::time::sleep(Duration::from_millis(3 * o.latency + 700)).await;
    (sent, replication)
}

/// The replication layer over the crowd's people and its `--workers` workers, each a client of an
/// endpoint of its own on loopback with no conditioner (a datacentre's link), owning a strip of the
/// square along x; the endpoint is returned to be kept as long as the host.
async fn start_layer(o: &Options, issuer: &TokenIssuer) -> (LayerHost<Person>, Server) {
    let identity = ServerIdentity::generate(&["localhost"]).expect("a certificate");
    let certificate = identity.certificate().to_vec();
    let config =
        ServerConfig::loopback(identity, TokenVerifier::new(issuer.public_key(), LAYER_ID));
    let mut server = Server::bind(config).expect("the layer could not bind");
    let address = server.local_address();
    let half = o.side / 2.0;
    let strips = Strips {
        cuts: (1..o.workers)
            .map(|k| -half + o.side * k as f64 / o.workers as f64)
            .collect(),
        margin: 2.0,
        band: 2.0,
    };
    let people = Crowd::new(o.replicate, o.side, o.seed).people();
    let mut host = LayerHost::new(Layer::new(&strips, people), strips);
    let side = o.side;
    for w in 0..o.workers as WorkerId {
        let token = issuer.issue(&TokenClaims::new(u64::from(w) + 1, LAYER_ID, 600));
        let certificate = certificate.clone();
        tokio::spawn(async move {
            let mut session =
                Client::connect(ClientConfig::loopback(&certificate), address, &token)
                    .await
                    .expect("a worker could not connect");
            layer::serve(w, &mut session, |id, p: &Person, _| {
                p.step(id, 1.0 / 60.0, side)
            })
            .await
        });
        let session = server.accept().await.expect("the layer closed");
        host.attach(w, session);
    }
    (host, server)
}

fn main() {
    let o = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("net-loopback: {e}");
            std::process::exit(2);
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime");
    runtime.block_on(async move {
        let issuer = TokenIssuer::generate().expect("a signing key");
        let identity = ServerIdentity::generate(&["localhost"]).expect("a certificate");
        let certificate = identity.certificate().to_vec();
        let mut config =
            ServerConfig::loopback(identity, TokenVerifier::new(issuer.public_key(), SERVER_ID));
        config.conditioner = Some(ConditionerConfig::new(o.latency, o.loss, o.seed));
        let server = Server::bind(config).expect("the server could not bind");
        let address = server.local_address();
        // Every client connects first; the clock starts once they are all in.
        let start = Instant::now() + Duration::from_millis(500 + 20 * o.clients as u64);
        let clients: Vec<_> = (0..o.clients)
            .map(|k| {
                let token = issuer.issue(&TokenClaims::new(k as u64 + 1, SERVER_ID, 600));
                tokio::spawn(run_client(o, k, certificate.clone(), token, address, start))
            })
            .collect();
        let (layer, _layer_endpoint) = if o.replicate > 0 && o.workers > 0 {
            let (host, endpoint) = start_layer(&o, &issuer).await;
            (Some(host), Some(endpoint))
        } else {
            (None, None)
        };
        let (sent, replication) = serve(o, server, start, layer).await;
        let mut reports = Vec::new();
        for c in clients {
            reports.push(c.await.expect("a client failed"));
        }
        print(&o, sent, &reports, replication.as_ref());
    });
}

fn print(o: &Options, sent: u64, reports: &[Report], replication: Option<&Replication>) {
    let n = reports.len() as f64;
    let mean = |f: &dyn Fn(&Report) -> f64| reports.iter().map(f).sum::<f64>() / n;
    let per_second = |bytes: u64| bytes as f64 / o.seconds / 1000.0;
    let down = mean(&|r| per_second(r.wire_in));
    let up = mean(&|r| per_second(r.wire_out));
    let payload_down = mean(&|r| per_second(r.payload_in));
    let payload_up = mean(&|r| per_second(r.payload_out));
    // IPv4 and UDP add 28 bytes a datagram that QUIC's counts leave out.
    let ip_down = mean(&|r| per_second(r.wire_in + 28 * r.datagrams_in));
    let ip_up = mean(&|r| per_second(r.wire_out + 28 * r.datagrams_out));
    let rate_down = mean(&|r| r.datagrams_in as f64 / o.seconds);
    let rate_up = mean(&|r| r.datagrams_out as f64 / o.seconds);
    let received: u64 = reports.iter().map(|r| r.received).sum();
    let mut rtts: Vec<f64> = reports.iter().map(|r| r.rtt.as_secs_f64() * 1e3).collect();
    rtts.sort_by(f64::total_cmp);
    let what = if o.replicate > 0 {
        format!(
            "snapshots of {} people in {:.0} m (budget {} B)",
            o.replicate, o.side, o.budget
        )
    } else {
        format!("{} B", o.payload)
    };
    println!(
        "{} clients, {:.0} s; down {what} at {:.0} Hz, up {} B at {:.0} Hz; {} ms one way, {:.1} % lost each way",
        o.clients,
        o.seconds,
        o.rate,
        o.input.max(6),
        o.input_rate,
        o.latency,
        o.loss * 100.0
    );
    println!(
        "per client, down: {down:.2} kB/s of UDP payload ({ip_down:.2} with IPv4 headers) in {rate_down:.0} datagrams/s, for {payload_down:.2} kB/s of messages (+{:.1} %)",
        (down / payload_down - 1.0) * 100.0
    );
    println!(
        "per client, up:   {up:.2} kB/s of UDP payload ({ip_up:.2} with IPv4 headers) in {rate_up:.0} datagrams/s, for {payload_up:.2} kB/s of messages (+{:.1} %)",
        (up / payload_up - 1.0) * 100.0
    );
    println!(
        "server messages delivered: {received} of {sent} ({:.2} %)",
        received as f64 / sent.max(1) as f64 * 100.0
    );
    println!(
        "round trip (QUIC's smoothed): min {:.1} ms, median {:.1} ms, max {:.1} ms",
        rtts[0],
        rtts[rtts.len() / 2],
        rtts[rtts.len() - 1]
    );
    if let Some(r) = replication {
        let bad: u64 = reports.iter().map(|r| r.bad).sum();
        let held = mean(&|r| r.held as f64);
        println!(
            "replication: {bad} snapshots failed to decode; a client held {held:.0} people at the end; the server spent {:.0} us a tick on the crowd and its world, {:.1} us a client on its snapshot",
            r.world.as_secs_f64() * 1e6 / r.ticks.max(1) as f64,
            r.encode.as_secs_f64() * 1e6 / r.snapshots.max(1) as f64
        );
        if r.layer.is_some() {
            println!(
                "the layer: {} workers stepping the crowd at 60 Hz, {} handoffs, {:.0} us a tick (its pictures out and its workers' writes back, over QUIC)",
                o.workers,
                r.handoffs,
                r.layer_time.as_secs_f64() * 1e6 / r.layer_ticks.max(1) as f64
            );
        }
    }
}
