//! `forge-sim`'s server and two predicting, paced clients over QUIC (#268): the scheme the physics
//! lab runs in one process (stamped commands sent again until acknowledged, snapshots checked
//! by digest, the clock paced by the server's measure of the lead), here through the network
//! thread ([`NetThread`]) and the link conditioner both ways, every loop synchronous as a
//! simulation's tick is: nothing in it awaits.
//!
//! The clock runs four times fast (240 ticks a second), so the 40 ms links are about ten ticks.
//! Binds 127.0.0.1 only.

use std::time::{Duration, Instant};

use forge_core::SplitMix64;
use forge_net::{
    ChannelId, ClientConfig, ConditionerConfig, NetThread, ServerConfig, ServerIdentity, Session,
    TokenClaims, TokenIssuer, TokenVerifier,
};
use forge_sim::{Client, Codec, InputPacket, PaceClock, Server, Simulation, Snapshot, Stamped};

const UNRELIABLE: ChannelId = ChannelId(2);
const SERVER_ID: u64 = 41;
/// Ticks a wall-clock second.
const RATE: f64 = 240.0;
/// Ticks between snapshots.
const SNAPSHOT_EVERY: u64 = 6;
/// The clients' lead on arrival they aim at, ticks: the 40 ms links' ten and some margin.
const TARGET: i64 = 4;

/// Balls bouncing on a floor in integer steps (forge-sim's own test world): a push changes one's
/// speed.
#[derive(Clone)]
struct Balls {
    tick: u64,
    balls: Vec<(i64, i64)>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Push {
    ball: u16,
    by: i32,
}

impl Codec for Push {
    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.ball.to_le_bytes());
        out.extend_from_slice(&self.by.to_le_bytes());
    }

    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let (ball, rest) = bytes.split_first_chunk::<2>()?;
        let (by, rest) = rest.split_first_chunk::<4>()?;
        *bytes = rest;
        Some(Self {
            ball: u16::from_le_bytes(*ball),
            by: i32::from_le_bytes(*by),
        })
    }
}

impl Balls {
    fn new() -> Self {
        Self {
            tick: 0,
            balls: (0..8).map(|k| (1000 + 300 * k, 0)).collect(),
        }
    }
}

impl Simulation for Balls {
    type Command = Push;

    fn tick(&mut self, commands: &[Stamped<Push>]) {
        for c in commands {
            self.balls[c.command.ball as usize % 8].1 += i64::from(c.command.by);
        }
        for (pos, vel) in &mut self.balls {
            *vel -= 3;
            *pos += *vel;
            if *pos < 0 {
                *pos = -*pos;
                *vel = -*vel * 9 / 10;
            }
        }
        self.tick += 1;
    }

    fn now(&self) -> u64 {
        self.tick
    }

    fn save(&mut self) -> Vec<u8> {
        let mut out = self.tick.to_le_bytes().to_vec();
        for (p, v) in &self.balls {
            out.extend_from_slice(&p.to_le_bytes());
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    fn restore(&mut self, state: &[u8]) {
        let words: Vec<i64> = state
            .as_chunks::<8>()
            .0
            .iter()
            .map(|&w| i64::from_le_bytes(w))
            .collect();
        self.tick = words[0] as u64;
        self.balls = words[1..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[p, v]| (p, v))
            .collect();
    }

    fn digest(&mut self) -> u64 {
        self.save().iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    }
}

fn bytes_of<T: Codec>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.encode(&mut out);
    out
}

fn decoded<T: Codec>(payload: &[u8]) -> T {
    let mut bytes = payload;
    let value = T::decode(&mut bytes).expect("a packet that decodes");
    assert!(bytes.is_empty(), "nothing after it");
    value
}

/// One player at its end of the link: its predicting client, its paced clock and its session.
struct Player {
    client: Client<Balls>,
    clock: PaceClock,
    session: Session,
    /// The ticks of its own clock it pushes at, and with what.
    pushes: Vec<(u64, Push)>,
    leads: Vec<i64>,
}

#[test]
fn a_server_and_two_predicting_clients_end_alike_over_quic_through_the_network_thread() {
    let net = NetThread::start().unwrap();
    let issuer = TokenIssuer::generate().unwrap();
    let identity = ServerIdentity::generate(&["localhost"]).unwrap();
    let certificate = identity.certificate().to_vec();
    let link = |seed| ConditionerConfig::new(40, 0.02, seed).with_jitter(Duration::from_millis(4));
    let mut config =
        ServerConfig::loopback(identity, TokenVerifier::new(issuer.public_key(), SERVER_ID));
    config.conditioner = Some(link(1));
    let listener = net.serve(config).unwrap();
    let mut players: Vec<Player> = (0..2u16)
        .map(|p| {
            let mut config = ClientConfig::loopback(&certificate);
            config.conditioner = Some(link(10 + u64::from(p)));
            let token = issuer.issue(&TokenClaims::new(u64::from(p) + 1, SERVER_ID, 60));
            let session = net
                .connect(config, listener.local_address(), &token)
                .unwrap();
            // It starts as far ahead as its target and the link's ten ticks.
            let mut client = Client::new(Balls::new(), p);
            for _ in 0..TARGET + 10 {
                client.step();
            }
            let mut random = SplitMix64::new(77 + u64::from(p));
            let mut pushes: Vec<(u64, Push)> = (0..20)
                .map(|_| {
                    (
                        40 + random.next_u64() % 900,
                        Push {
                            ball: (random.next_u64() % 8) as u16,
                            by: (random.next_u64() % 200) as i32 - 50,
                        },
                    )
                })
                .collect();
            pushes.sort_by_key(|&(t, _)| t);
            pushes.dedup_by_key(|&mut (t, _)| t);
            Player {
                client,
                clock: PaceClock::new(TARGET).tolerance(1),
                session,
                pushes,
                leads: Vec::new(),
            }
        })
        .collect();
    // The server's sessions, in the clients' order (each says who it is by its packets).
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut sessions = Vec::new();
    while sessions.len() < players.len() {
        assert!(Instant::now() < deadline, "the clients never arrived");
        if let Some(s) = listener.try_accept() {
            sessions.push(s);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut server = Server::new(Balls::new());
    let pushed: usize = players.iter().map(|p| p.pushes.len()).sum();
    let end = 1200;
    let start = Instant::now();
    let mut last = start;
    while server.sim.now() < end {
        let now = Instant::now();
        // The server: what arrived, then the ticks its clock owes, a snapshot every sixth.
        for s in &mut sessions {
            while let Some(m) = s.try_recv() {
                server.receive(&decoded::<InputPacket<Push>>(&m.payload));
            }
        }
        while (server.sim.now() as f64) < now.duration_since(start).as_secs_f64() * RATE
            && server.sim.now() < end
        {
            server.step();
            if server.sim.now().is_multiple_of(SNAPSHOT_EVERY) {
                let snapshot = bytes_of(&server.snapshot());
                for s in &sessions {
                    s.send(UNRELIABLE, snapshot.clone()).unwrap();
                }
            }
        }
        // The clients: what arrived, then the ticks their paced clocks owe, sending each tick.
        let elapsed = now.duration_since(last).as_secs_f64() * RATE;
        last = now;
        for p in &mut players {
            while let Some(m) = p.session.try_recv() {
                let snapshot: Snapshot = decoded(&m.payload);
                p.client.apply(&snapshot);
                if let Some(&(_, lead)) = snapshot
                    .leads
                    .iter()
                    .find(|(who, _)| *who == p.client.player)
                {
                    p.clock.report(lead);
                    p.leads.push(lead);
                }
            }
            for _ in 0..p.clock.advance(elapsed) {
                let here = p.client.sim.now();
                for &(_, push) in p.pushes.iter().filter(|&&(t, _)| t == here) {
                    p.client.command(push);
                }
                p.client.step();
                if let Some(packet) = p.client.outgoing() {
                    p.session.send(UNRELIABLE, bytes_of(&packet)).unwrap();
                }
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the session ran past its deadline"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Every command taken once, and the clients' last snapshots in: each client is where the
    // server is, its world run on to the client's tick with nothing more.
    assert_eq!(server.stats.applied as usize, pushed, "{:?}", server.stats);
    let quiet = Instant::now() + Duration::from_millis(300);
    while Instant::now() < quiet {
        for p in &mut players {
            while let Some(m) = p.session.try_recv() {
                p.client.apply(&decoded::<Snapshot>(&m.payload));
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    for (k, p) in players.iter_mut().enumerate() {
        let target = p.client.sim.now();
        let mut world = server.sim.clone();
        while world.now() < target {
            world.tick(&[]);
        }
        assert_eq!(world.digest(), p.client.sim.digest(), "client {k}");
        let s = p.client.stats;
        // Each learns of the other's pushes from the snapshots, and is taken back by them.
        assert!(s.matched > 0 && s.corrected > 0, "client {k}: {s:?}");
        // Paced to its target: the median of the second half's leads within a few ticks of it
        // (a machine's hiccup moves single ones; the median holds).
        let mut settled = p.leads[p.leads.len() / 2..].to_vec();
        settled.sort_unstable();
        let median = settled[settled.len() / 2];
        eprintln!(
            "client {k}: {s:?}, late {}, lead settled {}..{} (median {median}) ticks, pace {:.3}",
            server.stats.late,
            settled[0],
            settled[settled.len() - 1],
            p.clock.rate()
        );
        assert!(
            (0..=TARGET + 4).contains(&median),
            "client {k}: lead {median}"
        );
    }
    drop(sessions);
    drop(players);
    drop(listener);
}
