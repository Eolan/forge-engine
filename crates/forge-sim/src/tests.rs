//! The clock's checks on a toy world of integer balls: a recording replays to the same digests
//! and catches a change; a client predicting over a link of 100 ms with losses ends where the
//! server is, whether alone or with another player.

use super::*;

/// Balls bouncing on a floor in integer steps: a push changes one's speed.
#[derive(Clone)]
struct Toy {
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
        Some(Self {
            ball: take::<2>(bytes).map(u16::from_le_bytes)?,
            by: take::<4>(bytes).map(i32::from_le_bytes)?,
        })
    }
}

impl Toy {
    fn new() -> Self {
        Self {
            tick: 0,
            balls: (0..8).map(|k| (1000 + 300 * k, 0)).collect(),
        }
    }
}

impl Simulation for Toy {
    type Command = Push;

    fn tick(&mut self, commands: &[Stamped<Push>]) {
        for c in commands {
            assert_eq!(c.tick, self.tick, "a command applied at its tick");
            let ball = &mut self.balls[c.command.ball as usize % 8];
            ball.1 += i64::from(c.command.by);
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
        let state = self.save();
        state.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    }
}

/// `n` pushes by `player` at ticks from a seed, spread over `ticks`.
fn pushes(player: PlayerId, n: u32, ticks: u64, seed: u64) -> Vec<Stamped<Push>> {
    let mut random = SplitMix64::new(seed);
    let mut out: Vec<_> = (0..n)
        .map(|seq| Stamped {
            // After the clients' lead: a command is made at the tick it is stamped with.
            tick: 20 + random.next_u64() % ticks,
            player,
            seq,
            command: Push {
                ball: (random.next_u64() % 8) as u16,
                by: (random.next_u64() % 200) as i32 - 50,
            },
        })
        .collect();
    out.sort_by_key(|c| c.tick);
    for (seq, c) in out.iter_mut().enumerate() {
        c.seq = seq as u32;
    }
    out
}

#[test]
fn a_recording_replays_to_the_same_digests_and_survives_its_bytes() {
    let commands = pushes(0, 40, 600, 7);
    let recording = Recording::record(&mut Toy::new(), commands, 900, 60);
    assert_eq!(recording.digests.len(), 15);
    assert_eq!(recording.commands.len(), 40);
    let back = Recording::from_bytes(&recording.to_bytes()).expect("its bytes");
    assert_eq!(back, recording);
    back.replay(&mut Toy::new()).expect("the same digests");
}

#[test]
fn a_changed_command_shows_at_the_next_digest() {
    let mut recording = Recording::record(&mut Toy::new(), pushes(0, 40, 600, 7), 900, 60);
    let k = recording.commands.len() / 2;
    let tick = recording.commands[k].tick;
    recording.commands[k].command.by += 1;
    let divergence = recording.replay(&mut Toy::new()).expect_err("a divergence");
    assert_eq!(divergence.tick, (tick / 60 + 1) * 60);
}

#[test]
fn a_link_delays_and_loses_as_asked() {
    let mut link = Link::new(LinkParams::new(100.0, 0.02, 3));
    for k in 0..10_000 {
        link.send(f64::from(k) / 60.0, k, 40);
    }
    assert!(link.receive(0.099).is_empty(), "nothing before the delay");
    let arrived = link.receive(1e9);
    let lost = link.stats.lost as f64 / 10_000.0;
    assert!((0.015..0.025).contains(&lost), "{lost}");
    assert_eq!(arrived.len() as u64 + link.stats.lost, 10_000);
    assert!(
        arrived.windows(2).all(|w| w[0] < w[1]),
        "a 10 ms jitter keeps 17 ms apart in order"
    );
}

/// The session the tests run: a server and `players` clients over links of `delay_ms` with
/// `loss`, the clients `lead` ticks ahead, a snapshot every `every` ticks; each client makes its
/// commands at the ticks given (its own clock), then everyone runs on quiet for two seconds.
struct Session {
    server: Server<Toy>,
    clients: Vec<Client<Toy>>,
}

fn session(players: u16, delay_ms: f64, loss: f64, commands: &[Vec<Stamped<Push>>]) -> Session {
    let lead = (delay_ms / 1e3 / f64::from(TICK)).ceil() as u64 + 2;
    let every = 6;
    let mut server = Server::new(Toy::new());
    server.record(60, 300);
    let mut clients: Vec<Client<Toy>> = (0..players)
        .map(|p| {
            // Run ahead through the client, so it knows the digests of the ticks it skipped.
            let mut client = Client::new(Toy::new(), p);
            for _ in 0..lead {
                client.step();
            }
            client
        })
        .collect();
    let mut up: Vec<Link<InputPacket<Push>>> = (0..players)
        .map(|p| Link::new(LinkParams::new(delay_ms, loss, 100 + u64::from(p))))
        .collect();
    let mut down: Vec<Link<Snapshot>> = (0..players)
        .map(|p| Link::new(LinkParams::new(delay_ms, loss, 200 + u64::from(p))))
        .collect();
    let last = commands.iter().flatten().map(|c| c.tick).max().unwrap_or(0);
    for frame in 0..last + 120 {
        let now = frame as f64 / f64::from(TICK_RATE);
        for link in &mut up {
            for packet in link.receive(now) {
                server.receive(&packet);
            }
        }
        server.step();
        if server.sim.now().is_multiple_of(every) {
            let snapshot = server.snapshot();
            let bytes = snapshot.state.len();
            for link in &mut down {
                link.send(now, snapshot.clone(), bytes);
            }
        }
        for (p, client) in clients.iter_mut().enumerate() {
            for snapshot in down[p].receive(now) {
                client.apply(&snapshot);
            }
            let here = client.sim.now();
            for c in commands[p].iter().filter(|c| c.tick == here) {
                client.command(c.command);
            }
            client.step();
            if let Some(packet) = client.outgoing() {
                up[p].send(now, packet, 64);
            }
        }
    }
    // Quiet: the clients' last snapshots in, the server and the clients side by side.
    Session { server, clients }
}

/// The server's world and a client's at the same tick: the client's state when it was at the
/// server's tick is gone, so the server runs on to the client's tick with no command.
fn same_world(session: &mut Session, client: usize) -> bool {
    let target = session.clients[client].sim.now();
    let mut server = session.server.sim.clone();
    while server.now() < target {
        server.tick(&[]);
    }
    server.digest() == session.clients[client].sim.digest()
}

#[test]
fn the_servers_recording_replays_its_session_and_starts_anywhere() {
    let commands = vec![pushes(0, 30, 1200, 21), pushes(1, 30, 1200, 22)];
    let s = session(2, 100.0, 0.02, &commands);
    let recording = s.server.recording().expect("recorded").clone();
    let end = s.server.sim.now();
    assert_eq!((recording.start(), recording.ticks), (Some(0), end));
    assert_eq!(recording.commands.len(), 60, "every command, once");
    assert_eq!(recording.keyframes.len() as u64, end / 300 + 1);
    let digest_at = |tick: u64| {
        let mut toy = Toy::new();
        Recording {
            keyframes: recording.keyframes[..1].to_vec(),
            ..recording.clone()
        }
        .seek(&mut toy, tick)
        .expect("from the start");
        toy.digest()
    };
    // From its start, as the session went: every digest, and the server's world at the end.
    let mut toy = Toy::new();
    recording.replay(&mut toy).expect("the same digests");
    assert_eq!(toy.digest(), s.server.sim.clone().digest());
    // From the keyframe before a tick between keyframes: the world the run from the start has.
    for tick in [0, 299, 300, 777, end] {
        let mut toy = Toy::new();
        recording.seek(&mut toy, tick).expect("a seek");
        assert_eq!(toy.now(), tick);
        assert_eq!(toy.digest(), digest_at(tick), "tick {tick}");
    }
    assert_eq!(
        recording.seek(&mut Toy::new(), end + 1),
        Err(SeekError::Outside(end + 1))
    );
    // Its bytes, keyframes and all; the first format still read.
    let back = Recording::from_bytes(&recording.to_bytes()).expect("its bytes");
    assert_eq!(back, recording);
    let plain = Recording {
        keyframes: Vec::new(),
        ..recording.clone()
    };
    let mut first = plain.to_bytes();
    first.truncate(first.len() - 8); // no keyframe count
    first[..8].copy_from_slice(b"FRGREC01");
    assert_eq!(Recording::from_bytes(&first), Some(plain));
    // A command changed after a keyframe shows at the next digest, seeking from that keyframe.
    let mut changed = recording.clone();
    let k = changed
        .commands
        .iter()
        .position(|c| c.tick > 600 && c.tick % 300 < 240)
        .expect("a push well before a keyframe");
    let tick = changed.commands[k].tick;
    changed.commands[k].command.by += 1;
    let Err(SeekError::Diverged(d)) = changed.seek(&mut Toy::new(), tick / 300 * 300 + 299) else {
        panic!("a divergence")
    };
    assert_eq!(d.tick, (tick / 60 + 1) * 60);
}

#[test]
fn packets_and_snapshots_cross_a_transport_as_bytes() {
    let commands = vec![pushes(0, 30, 1200, 21), pushes(1, 30, 1200, 22)];
    let mut s = session(2, 100.0, 0.02, &commands);
    let snapshot = s.server.snapshot();
    assert!(!snapshot.acked.is_empty() && !snapshot.leads.is_empty());
    let packet = InputPacket {
        player: 1,
        tick: 77,
        commands: commands[1][..3].to_vec(),
    };
    let (mut a, mut b) = (Vec::new(), Vec::new());
    snapshot.encode(&mut a);
    packet.encode(&mut b);
    assert_eq!(Snapshot::decode(&mut a.as_slice()), Some(snapshot));
    assert_eq!(InputPacket::decode(&mut b.as_slice()), Some(packet));
    // Cut short: not a packet.
    assert_eq!(InputPacket::<Push>::decode(&mut &b[..b.len() - 1]), None);
}

#[test]
fn a_client_alone_predicts_what_the_server_does() {
    let commands = vec![pushes(0, 60, 1200, 11)];
    let mut s = session(1, 100.0, 0.02, &commands);
    assert_eq!(s.server.stats.applied, 60, "every command taken");
    assert_eq!(s.server.stats.late, 0, "none late with the lead");
    assert!(same_world(&mut s, 0));
    let stats = s.clients[0].stats;
    // Alone and on time, the prediction is the server's to the bit: no correction at all.
    assert!(stats.snapshots > 200, "{stats:?}");
    assert_eq!(stats.corrected, 0, "{stats:?}");
}

#[test]
fn two_clients_are_corrected_by_each_other_and_end_where_the_server_is() {
    let commands = vec![pushes(0, 30, 1200, 21), pushes(1, 30, 1200, 22)];
    let mut s = session(2, 100.0, 0.02, &commands);
    assert_eq!(s.server.stats.applied, 60);
    for client in 0..2 {
        assert!(same_world(&mut s, client), "client {client}");
        let stats = s.clients[client].stats;
        // Each learns of the other's pushes from the snapshots, and is taken back by them.
        assert!(
            stats.corrected > 0 && stats.corrected < stats.snapshots,
            "{stats:?}"
        );
        assert_eq!(s.clients[client].unacknowledged(), 0);
    }
}

#[test]
fn a_late_command_is_taken_at_the_next_tick_and_corrected() {
    // A client with no lead: its commands reach the server after their tick.
    let commands = [pushes(0, 20, 600, 31)];
    let mut server = Server::new(Toy::new());
    let mut client = Client::new(Toy::new(), 0);
    let mut up = Link::new(LinkParams::new(50.0, 0.0, 1));
    let mut down = Link::new(LinkParams::new(50.0, 0.0, 2));
    for frame in 0..720u64 {
        let now = frame as f64 / 60.0;
        for packet in up.receive(now) {
            server.receive(&packet);
        }
        server.step();
        if server.sim.now().is_multiple_of(6) {
            let snapshot = server.snapshot();
            down.send(now, snapshot, 0);
        }
        for snapshot in down.receive(now) {
            client.apply(&snapshot);
        }
        let here = client.sim.now();
        for c in commands[0].iter().filter(|c| c.tick == here) {
            client.command(c.command);
        }
        client.step();
        if let Some(packet) = client.outgoing() {
            up.send(now, packet, 0);
        }
    }
    assert_eq!(server.stats.late, 20);
    assert!(client.stats.corrected > 0);
    let mut s = Session {
        server,
        clients: vec![client],
    };
    assert!(same_world(&mut s, 0));
}

/// What a paced run made: the leads the server measured (at each snapshot the client took, with
/// the frame), and the commands taken late in each of its three phases' last two thirds.
struct Paced {
    leads: Vec<(u64, i64)>,
    late_settled: [u64; 3],
    late: u64,
    applied: u64,
}

/// One client over a link of 100 ms one way, starting with no lead; its route goes to 200 ms at
/// frame 1200 and to 50 ms at frame 2400. Its clock is paced to a lead of `target` ticks.
fn paced(target: i64) -> Paced {
    let mut server = Server::new(Toy::new());
    let mut client = Client::new(Toy::new(), 0);
    let mut clock = PaceClock::new(target);
    let mut up = Link::new(LinkParams::new(100.0, 0.02, 41));
    let mut down = Link::new(LinkParams::new(100.0, 0.02, 42));
    let commands = pushes(0, 300, 3500, 43);
    let mut out = Paced {
        leads: Vec::new(),
        late_settled: [0; 3],
        late: 0,
        applied: 0,
    };
    for frame in 0..3600u64 {
        let now = frame as f64 / f64::from(TICK_RATE);
        if frame == 1200 {
            up.set_delay(0.2);
            down.set_delay(0.2);
        }
        if frame == 2400 {
            up.set_delay(0.05);
            down.set_delay(0.05);
        }
        let late = server.stats.late;
        for packet in up.receive(now) {
            server.receive(&packet);
        }
        if frame % 1200 >= 400 {
            out.late_settled[(frame / 1200) as usize] += server.stats.late - late;
        }
        server.step();
        if server.sim.now().is_multiple_of(6) {
            down.send(now, server.snapshot(), 0);
        }
        for snapshot in down.receive(now) {
            if let Some(&(_, lead)) = snapshot.leads.iter().find(|(p, _)| *p == 0) {
                clock.report(lead);
                out.leads.push((frame, lead));
            }
            client.apply(&snapshot);
        }
        for _ in 0..clock.advance(1.0) {
            let here = client.sim.now();
            for c in commands.iter().filter(|c| c.tick == here) {
                client.command(c.command);
            }
            client.step();
            if let Some(packet) = client.outgoing() {
                up.send(now, packet, 16);
            }
        }
    }
    out.late = server.stats.late;
    out.applied = server.stats.applied;
    out
}

/// D-010's pacing: a client with no lead runs a little faster until its packets reach the server
/// `target` ticks ahead, and follows its route when the delay doubles and when it falls to a
/// quarter: once settled, no command is late, and the lead stays near the target.
#[test]
fn a_paced_client_keeps_its_lead_as_the_route_changes() {
    let run = paced(3);
    for phase in 0..3u64 {
        let leads: Vec<i64> = run
            .leads
            .iter()
            .filter(|(f, _)| f / 1200 == phase && f % 1200 >= 400)
            .map(|&(_, l)| l)
            .collect();
        eprintln!(
            "phase {phase}: lead {}..{} after settling, {} late",
            leads.iter().min().unwrap(),
            leads.iter().max().unwrap(),
            run.late_settled[phase as usize]
        );
    }
    eprintln!("{} late of {} applied", run.late, run.applied);
    assert_eq!(run.applied, 300, "every command taken, none repeated");
    assert_eq!(run.late_settled, [0, 0, 0], "late once settled");
    assert!(run.late < 20, "{} late in the changes", run.late);
    for phase in 0..3u64 {
        assert!(
            run.leads
                .iter()
                .filter(|(f, _)| f / 1200 == phase && f % 1200 >= 400)
                .all(|&(_, l)| (3..=4).contains(&l)),
            "phase {phase} settles at its target"
        );
    }
}

/// The clock's rate follows the lead's error, within ±10 %, and its ticks the rate.
#[test]
fn the_pace_follows_the_error_within_a_tenth() {
    let mut clock = PaceClock::new(3);
    assert_eq!(clock.advance(10.0), 10);
    clock.report(1);
    assert!((clock.rate() - 1.04).abs() < 1e-12);
    assert_eq!((0..100).map(|_| clock.advance(1.0)).sum::<u32>(), 104);
    clock.report(-20);
    assert!((clock.rate() - 1.1).abs() < 1e-12);
    clock.report(30);
    assert!((clock.rate() - 0.9).abs() < 1e-12);
    clock.report(3);
    assert_eq!(clock.rate(), 1.0);
}
