//! `net-loopback`: a server and headless clients on loopback through the link conditioner,
//! measuring what the transport costs (issue #268; `docs/demos/net.md`).
//!
//! The server sends each client a message of `--payload` bytes `--rate` times a second on the
//! unreliable channel (a snapshot's stand-in); each client sends `--input` bytes `--input-rate`
//! times a second (its inputs). Both directions go through a conditioner of `--latency` ms one
//! way, a tenth of it as jitter, and `--loss` of the packets lost. At the end it prints, per
//! client, the bytes on the wire each way (QUIC's headers, acks and encryption included), the
//! messages delivered and the round trip QUIC measured.
//!
//! ```text
//! cargo run --release -p forge-net --bin net-loopback -- --clients 8 --seconds 10 \
//!     --rate 30 --payload 800 --latency 50 --loss 0.02
//! ```
//!
//! It binds 127.0.0.1 only.

use std::time::{Duration, Instant};

use forge_net::{
    ChannelId, Client, ClientConfig, ConditionerConfig, Server, ServerConfig, ServerIdentity,
    Session, TokenClaims, TokenIssuer, TokenVerifier,
};
use tokio::time::MissedTickBehavior;

const UNRELIABLE: ChannelId = ChannelId(2);
const SERVER_ID: u64 = 1;

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
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut k = 0;
        while k < args.len() {
            let value = args
                .get(k + 1)
                .ok_or_else(|| format!("{} needs a value", args[k]))?;
            match args[k].as_str() {
                "--clients" => o.clients = num(&args[k], value)?,
                "--seconds" => o.seconds = num(&args[k], value)?,
                "--rate" => o.rate = num(&args[k], value)?,
                "--payload" => o.payload = num(&args[k], value)?,
                "--input-rate" => o.input_rate = num(&args[k], value)?,
                "--input" => o.input = num(&args[k], value)?,
                "--latency" => o.latency = num(&args[k], value)?,
                "--loss" => o.loss = num(&args[k], value)?,
                "--seed" => o.seed = num(&args[k], value)?,
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
    let input = vec![0x5Au8; o.input];
    let end = start + Duration::from_secs_f64(o.seconds);
    // The bytes are counted from `start` to `end`; the messages the server sent before `end`
    // are counted until they have had time to arrive.
    let tail = end + Duration::from_millis(3 * o.latency + 200);
    let (mut received, mut first, mut last) = (0, None, None);
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
                    let _ = session.send(UNRELIABLE, input.clone());
                }
            }
            m = session.recv() => match m {
                Some(_) => received += 1,
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
    }
}

async fn serve(o: Options, mut server: Server, start: Instant) -> u64 {
    let mut sessions: Vec<Session> = Vec::with_capacity(o.clients);
    while sessions.len() < o.clients {
        sessions.push(server.accept().await.expect("the server closed"));
    }
    let period = Duration::from_secs_f64(1.0 / o.rate);
    let mut tick = tokio::time::interval_at(start.into(), period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Burst);
    let payload = vec![0xA5u8; o.payload];
    let end = start + Duration::from_secs_f64(o.seconds);
    let mut sent = 0;
    loop {
        tick.tick().await;
        if Instant::now() >= end {
            break;
        }
        for s in &mut sessions {
            while s.try_recv().is_some() {}
            if s.send(UNRELIABLE, payload.clone()).is_ok() {
                sent += 1;
            }
        }
    }
    // Keep the sessions until the clients have stopped counting.
    tokio::time::sleep(Duration::from_millis(3 * o.latency + 700)).await;
    sent
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
        let sent = serve(o, server, start).await;
        let mut reports = Vec::new();
        for c in clients {
            reports.push(c.await.expect("a client failed"));
        }
        print(&o, sent, &reports);
    });
}

fn print(o: &Options, sent: u64, reports: &[Report]) {
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
    println!(
        "{} clients, {:.0} s; down {} B at {:.0} Hz, up {} B at {:.0} Hz; {} ms one way, {:.1} % lost each way",
        o.clients,
        o.seconds,
        o.payload,
        o.rate,
        o.input,
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
}
