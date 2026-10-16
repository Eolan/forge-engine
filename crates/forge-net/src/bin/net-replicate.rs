//! `net-replicate`: replication without sockets, measured (issue #268, step 2;
//! `docs/demos/net.md`).
//!
//! A synthetic crowd ([`forge_net::crowd`]) moves at 60 Hz; the server's replicator sends each
//! client a snapshot at `--rate` Hz under a `--budget` of bytes, through a seeded conditioner of
//! `--latency` ms and `--loss` each way on a virtual clock, and each client acknowledges at
//! 60 Hz. It prints the bytes a snapshot takes, the bandwidth a client needs, how often entities
//! are updated by distance, and the CPU each part takes on one thread.
//!
//! ```text
//! cargo run --release -p forge-net --bin net-replicate -- --entities 3000 --clients 100
//! cargo run --release -p forge-net --bin net-replicate -- --entities 1000 --everyone
//! ```
//!
//! `--everyone` sets an unbounded budget and radius: every entity in every snapshot, the size a
//! snapshot of N entities needs.

use std::time::{Duration, Instant};

use glam::DVec3;

use forge_core::SplitMix64;
use forge_net::crowd::Crowd;
use forge_net::{
    Ack, ConditionerConfig, LinkConditioner, ReplicationConfig, Replicator, SnapshotReceiver, Taken,
};

#[derive(Clone, Copy, Debug)]
struct Options {
    entities: usize,
    side: f64,
    clients: usize,
    seconds: f64,
    rate: f64,
    budget: usize,
    radius: f64,
    latency: u64,
    loss: f64,
    seed: u64,
    everyone: bool,
}

fn num<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{flag} {value}: not a number"))
}

impl Options {
    fn parse() -> Result<Self, String> {
        let mut o = Self {
            entities: 3000,
            side: 600.0,
            clients: 1,
            seconds: 20.0,
            rate: 30.0,
            budget: 720,
            radius: 150.0,
            latency: 50,
            loss: 0.02,
            seed: 1,
            everyone: false,
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut k = 0;
        while k < args.len() {
            if args[k] == "--everyone" {
                o.everyone = true;
                k += 1;
                continue;
            }
            let value = args
                .get(k + 1)
                .ok_or_else(|| format!("{} needs a value", args[k]))?;
            let flag = args[k].as_str();
            match flag {
                "--entities" => o.entities = num(flag, value)?,
                "--side" => o.side = num(flag, value)?,
                "--clients" => o.clients = num(flag, value)?,
                "--seconds" => o.seconds = num(flag, value)?,
                "--rate" => o.rate = num(flag, value)?,
                "--budget" => o.budget = num(flag, value)?,
                "--radius" => o.radius = num(flag, value)?,
                "--latency" => o.latency = num(flag, value)?,
                "--loss" => o.loss = num(flag, value)?,
                "--seed" => o.seed = num(flag, value)?,
                other => return Err(format!("unknown option {other}")),
            }
            k += 2;
        }
        Ok(o)
    }
}

/// One direction of a link on the virtual clock.
struct Link {
    conditioner: LinkConditioner,
    in_flight: Vec<(Duration, u64, Vec<u8>)>,
    order: u64,
}

impl Link {
    fn new(config: ConditionerConfig) -> Self {
        Self {
            conditioner: LinkConditioner::new(config),
            in_flight: Vec::new(),
            order: 0,
        }
    }

    fn send(&mut self, now: Duration, packet: Vec<u8>) {
        for at in self
            .conditioner
            .submit(now, packet.len())
            .into_iter()
            .flatten()
        {
            self.in_flight.push((at, self.order, packet.clone()));
            self.order += 1;
        }
    }

    fn deliver(&mut self, now: Duration, out: &mut Vec<Vec<u8>>) {
        out.clear();
        let mut due = Vec::new();
        let mut k = 0;
        while k < self.in_flight.len() {
            if self.in_flight[k].0 <= now {
                due.push(self.in_flight.swap_remove(k));
            } else {
                k += 1;
            }
        }
        due.sort_by_key(|d| (d.0, d.1));
        out.extend(due.into_iter().map(|d| d.2));
    }
}

struct Client {
    focus: DVec3,
    receiver: SnapshotReceiver,
    down: Link,
    up: Link,
}

#[derive(Default)]
struct Tally {
    /// Snapshot sizes after the join.
    bytes: Vec<usize>,
    /// Client 0's first snapshot: its bytes and full states.
    first: (usize, usize),
    join_packets: usize,
    join_bytes: usize,
    full: usize,
    deltas: usize,
    left_out: usize,
    interest: usize,
    snapshots: usize,
    world: Duration,
    encode: Duration,
    decode: Duration,
    acks: Duration,
    world_ticks: usize,
}

fn main() {
    let o = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("net-replicate: {e}");
            std::process::exit(2);
        }
    };
    let mut config = ReplicationConfig {
        radius: o.radius,
        packet_budget: o.budget,
        ..ReplicationConfig::default()
    };
    if o.everyone {
        config.radius = 1e9;
        config.packet_budget = 1 << 26;
    }
    let mut crowd = Crowd::new(o.entities, o.side, o.seed);
    let mut replicator = Replicator::new(config);
    let mut random = SplitMix64::new(o.seed ^ 0x5EED);
    let half = o.side / 2.0;
    let link = |seed| ConditionerConfig::new(o.latency, o.loss, seed);
    let mut clients: Vec<Client> = (0..o.clients)
        .map(|k| {
            let focus = if k == 0 {
                DVec3::ZERO
            } else {
                DVec3::new(
                    random.range_f64(-half, half),
                    0.0,
                    random.range_f64(-half, half),
                )
            };
            replicator.add_client(k as u64, focus);
            Client {
                focus,
                receiver: SnapshotReceiver::new(config.quantization),
                down: Link::new(link(o.seed * 1000 + 2 * k as u64)),
                up: Link::new(link(o.seed * 1000 + 2 * k as u64 + 1)),
            }
        })
        .collect();
    let every = (60.0 / o.rate).round().max(1.0) as u64;
    let ticks = (o.seconds * 60.0).round() as u64;
    let joined_after = 120; // two seconds
    let mut tally = Tally::default();
    // Arrivals of client 0's entities by distance band: 0-30, 30-75, 75-150 m.
    let mut bands = [(0u64, 0u64); 3];
    let mut packets = Vec::new();
    for tick in 0..ticks {
        let now = Duration::from_secs_f64(tick as f64 / 60.0);
        crowd.step(1.0 / 60.0);
        if tick.is_multiple_of(every) {
            let t = Instant::now();
            replicator.set_world(crowd.states().iter().map(|(id, s)| (*id, s, 1.0)));
            tally.world += t.elapsed();
            tally.world_ticks += 1;
            for (k, c) in clients.iter_mut().enumerate() {
                let t = Instant::now();
                let (bytes, s) = replicator
                    .snapshot(k as u64, tick as u32)
                    .expect("a known client");
                tally.encode += t.elapsed();
                if k == 0 && tally.first.0 == 0 {
                    tally.first = (bytes.len(), s.full);
                }
                tally.snapshots += 1;
                if tick < joined_after {
                    tally.join_packets += usize::from(s.full > 0 && s.deltas == 0);
                    tally.join_bytes += bytes.len();
                } else {
                    tally.bytes.push(bytes.len());
                    tally.full += s.full;
                    tally.deltas += s.deltas;
                    tally.left_out += s.left_out;
                    tally.interest += s.interest;
                }
                c.down.send(now, bytes);
            }
        }
        for (k, c) in clients.iter_mut().enumerate() {
            c.down.deliver(now, &mut packets);
            for p in &packets {
                let t = Instant::now();
                let taken = c.receiver.receive(p).expect("a packet that decodes");
                tally.decode += t.elapsed();
                if k == 0 && tick >= joined_after && matches!(taken, Taken::Applied { .. }) {
                    for id in c.receiver.latest_entities() {
                        let s = crowd
                            .states()
                            .iter()
                            .find(|e| e.0 == id)
                            .map(|e| e.1.position);
                        if let Some(p) = s {
                            let d = p.distance(c.focus);
                            let band = if d < 30.0 {
                                0
                            } else if d < 75.0 {
                                1
                            } else {
                                2
                            };
                            bands[band].0 += 1;
                        }
                    }
                }
            }
            if let Some(ack) = c.receiver.ack() {
                c.up.send(now, ack.encode().to_vec());
            }
            c.up.deliver(now, &mut packets);
            for a in &packets {
                let t = Instant::now();
                replicator.acknowledge(k as u64, Ack::decode(a).expect("six bytes"));
                tally.acks += t.elapsed();
            }
        }
        if tick >= joined_after && tick.is_multiple_of(every) {
            // Client 0's interest by band, for the rates.
            for (_, s) in crowd.states() {
                let d = s.position.distance(clients[0].focus);
                if d <= config.radius.min(150.0) {
                    let band = if d < 30.0 {
                        0
                    } else if d < 75.0 {
                        1
                    } else {
                        2
                    };
                    bands[band].1 += 1;
                }
            }
        }
    }
    print(&o, &config, &tally, &bands, every);
}

fn print(o: &Options, config: &ReplicationConfig, t: &Tally, bands: &[(u64, u64); 3], every: u64) {
    let mut sizes = t.bytes.clone();
    sizes.sort_unstable();
    let n = sizes.len().max(1);
    let mean = sizes.iter().sum::<usize>() as f64 / n as f64;
    let pick = |p: f64| sizes[((n - 1) as f64 * p) as usize];
    let rate = 60.0 / every as f64;
    let per_snapshot = |v: usize| v as f64 / n as f64;
    println!(
        "{} entities in {:.0} m, {} clients; snapshots at {:.0} Hz, budget {} B, radius {:.0} m; {} ms one way, {:.1} % lost; {:.0} s",
        o.entities,
        o.side,
        o.clients,
        rate,
        config.packet_budget,
        config.radius,
        o.latency,
        o.loss * 100.0,
        o.seconds
    );
    println!(
        "the first snapshot: {} B for {} full states ({:.1} B each with the header shared)",
        t.first.0,
        t.first.1,
        t.first.0 as f64 / t.first.1.max(1) as f64
    );
    println!(
        "join: {} snapshots of full states only before the first delta (a client's mean); the first 2 s averaged {:.0} B a snapshot",
        t.join_packets / o.clients.max(1),
        t.join_bytes as f64 / (t.snapshots - t.bytes.len()).max(1) as f64
    );
    println!(
        "after it: bytes a snapshot mean {mean:.0}, median {}, p99 {}, max {}; {:.1} entities in interest, {:.1} deltas and {:.2} full states a snapshot, {:.1} changed but left out",
        pick(0.5),
        pick(0.99),
        pick(1.0),
        per_snapshot(t.interest),
        per_snapshot(t.deltas),
        per_snapshot(t.full),
        per_snapshot(t.left_out)
    );
    let payload = mean * rate / 1000.0;
    // A datagram's cost measured on loopback (docs/demos/net.md): the channel byte, QUIC's
    // short header, datagram frame and tag (about 31 bytes), IPv4 and UDP (28).
    let wire = (mean + 60.0) * rate / 1000.0;
    println!(
        "per client: {payload:.2} kB/s of snapshots, about {wire:.2} kB/s with QUIC's and IPv4's headers; {:.2} B an entity in a delta",
        if t.deltas > 0 {
            (mean * n as f64) / (t.deltas + t.full) as f64
        } else {
            0.0
        }
    );
    let band = |b: (u64, u64)| {
        if b.1 == 0 {
            0.0
        } else {
            b.0 as f64 / b.1 as f64 * rate
        }
    };
    println!(
        "client 0's updates an entity a second: {:.1} within 30 m, {:.1} at 30-75 m, {:.1} at 75-150 m",
        band(bands[0]),
        band(bands[1]),
        band(bands[2])
    );
    let ticks = t.world_ticks.max(1) as f64;
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    println!(
        "CPU, one thread: the world {:.0} us a snapshot tick ({} entities quantised and bucketed); encoding {:.1} us a client; acks {:.2} us a client a tick; the server {:.2} ms a snapshot tick for {} clients; decoding {:.1} us a snapshot",
        us(t.world) / ticks,
        o.entities,
        us(t.encode) / t.snapshots.max(1) as f64,
        us(t.acks) / (ticks * every as f64 * o.clients as f64),
        (us(t.world) + us(t.encode) + us(t.acks)) / ticks / 1000.0,
        o.clients,
        us(t.decode) / t.snapshots.max(1) as f64
    );
}
