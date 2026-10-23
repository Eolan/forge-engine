//! The replication layer and two workers over QUIC on loopback (issue #268, step 2's handoff):
//! 200 bots patrol across the workers' boundary, and the layer's records match tick for tick, bit
//! for bit, those of one worker holding everything in one process, with a worker killed mid-run
//! and started again.

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use forge_core::SplitMix64;
use forge_net::layer::{self, LayerHost};
use forge_net::{
    Client, ClientConfig, Server, ServerConfig, ServerIdentity, Session, TokenClaims, TokenIssuer,
    TokenVerifier,
};
use forge_sim::Codec;
use forge_sim::authority::{EntityId, Layer, Placed, Strips, Worker, WorkerId};
use glam::DVec3;

const SERVER_ID: u64 = 77;
const TICK: f64 = 1.0 / 60.0;
const REACH: f64 = 1.5;
const TURN: f64 = 12.0;
const TICKS: u64 = 900;
/// The tick worker 1 is killed at (before it answers), and the tick it comes back at.
const KILLED: u64 = 400;
const BACK: u64 = 600;

/// Runs a test's body under a deadline, so that a fault fails it instead of hanging CI.
async fn within<T>(seconds: u64, body: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(seconds), body)
        .await
        .expect("the test ran past its deadline")
}

/// A bot, as `forge-sim`'s test of the same name.
#[derive(Clone, Debug, PartialEq)]
struct Bot {
    position: DVec3,
    velocity: DVec3,
    goal: f64,
}

impl Placed for Bot {
    fn position(&self) -> DVec3 {
        self.position
    }
}

impl Codec for Bot {
    fn encode(&self, out: &mut Vec<u8>) {
        for v in self
            .position
            .to_array()
            .into_iter()
            .chain(self.velocity.to_array())
            .chain([self.goal])
        {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        let mut v = [0.0_f64; 7];
        for x in &mut v {
            let (head, rest) = bytes.split_first_chunk::<8>()?;
            *x = f64::from_le_bytes(*head);
            *bytes = rest;
        }
        Some(Self {
            position: DVec3::new(v[0], v[1], v[2]),
            velocity: DVec3::new(v[3], v[4], v[5]),
            goal: v[6],
        })
    }
}

/// One tick of a bot from its picture: towards its side, pushed off its neighbours.
fn step(id: EntityId, bot: &Bot, world: &BTreeMap<EntityId, &Bot>) -> Bot {
    let mut push = DVec3::ZERO;
    for (&other, b) in world {
        if other == id {
            continue;
        }
        let d = bot.position - b.position;
        let l = d.length();
        if l < REACH && l > 1e-9 {
            push += d / l * (REACH - l);
        }
    }
    let walk = 1.2 + f64::from(id % 5) * 0.1;
    let along = (bot.goal - bot.position.x).signum() * walk;
    let velocity = DVec3::new(along, 0.0, 0.0) + push * 2.0;
    let position = bot.position + velocity * TICK;
    let goal = if (position.x - bot.goal).abs() < 0.5 {
        -bot.goal
    } else {
        bot.goal
    };
    Bot {
        position,
        velocity,
        goal,
    }
}

fn bots() -> Vec<(EntityId, Bot)> {
    let mut r = SplitMix64::new(7);
    (0..200_u32)
        .map(|k| {
            let bot = Bot {
                position: DVec3::new(r.range_f64(-TURN, TURN), 0.0, r.range_f64(-10.0, 10.0)),
                velocity: DVec3::ZERO,
                goal: if k % 2 == 0 { TURN } else { -TURN },
            };
            (k * 3 + 1, bot)
        })
        .collect()
}

fn two() -> Strips {
    Strips {
        cuts: vec![0.0],
        margin: 2.0,
        band: 2.0 + REACH + 0.01,
    }
}

/// Each tick's records as one worker holding everything steps them, in this process.
fn reference() -> Vec<Vec<(EntityId, Bot)>> {
    let strips = Strips {
        cuts: Vec::new(),
        margin: 2.0,
        band: 0.0,
    };
    let mut layer = Layer::new(&strips, bots());
    let mut worker = Worker::new(0);
    (0..TICKS)
        .map(|_| {
            worker.sync(&layer, &strips);
            for (id, epoch, state) in worker.step(step) {
                layer.write(0, id, epoch, state).unwrap();
            }
            layer.rebalance(&strips);
            snapshot(&layer)
        })
        .collect()
}

fn snapshot(layer: &Layer<Bot>) -> Vec<(EntityId, Bot)> {
    layer
        .records()
        .iter()
        .map(|(&id, r)| (id, r.state.clone()))
        .collect()
}

/// The layer's server and what a worker needs to reach it.
struct Rig {
    issuer: TokenIssuer,
    certificate: Vec<u8>,
    server: Server,
}

impl Rig {
    fn new() -> Self {
        let issuer = TokenIssuer::generate().unwrap();
        let identity = ServerIdentity::generate(&["localhost"]).unwrap();
        let certificate = identity.certificate().to_vec();
        let config =
            ServerConfig::loopback(identity, TokenVerifier::new(issuer.public_key(), SERVER_ID));
        let server = Server::bind(config).unwrap();
        Self {
            issuer,
            certificate,
            server,
        }
    }

    /// Worker `worker`'s session, and the layer's for it.
    async fn connect(&mut self, worker: WorkerId) -> (Session, Session) {
        let token = self
            .issuer
            .issue(&TokenClaims::new(u64::from(worker), SERVER_ID, 60));
        let address = self.server.local_address();
        let (client, server) = tokio::join!(
            Client::connect(ClientConfig::loopback(&self.certificate), address, &token),
            self.server.accept()
        );
        (client.unwrap(), server.unwrap())
    }
}

/// A worker that steps as `layer::serve` does until the picture of tick `until`, which it does
/// not answer: it closes its session there, as a process killed would drop its connection.
async fn dying(mut session: Session, until: u64) {
    let mut worker = Worker::new(1);
    while let Some(message) = session.recv().await {
        let (frame, tick, picture) =
            layer::decode_picture::<Bot>(&message.payload).expect("a picture");
        if tick >= until {
            session.close();
            return;
        }
        worker.take(picture);
        let writes = worker.step(step);
        session
            .send(layer::CHANNEL, layer::encode_writes(frame, tick, &writes))
            .unwrap();
    }
}

/// What a run over the transport made.
struct Run {
    states: Vec<Vec<(EntityId, Bot)>>,
    handoffs: usize,
    reassigned: usize,
    covered: Vec<(u64, WorkerId, WorkerId)>,
}

/// The layer and two workers over loopback, worker 1 killed at [`KILLED`] and back at [`BACK`]
/// when `kill`.
async fn over_the_transport(kill: bool) -> Run {
    let mut rig = Rig::new();
    let mut host = LayerHost::new(Layer::new(&two(), bots()), two());
    for w in 0..2 {
        let (mut client, server) = rig.connect(w).await;
        host.attach(w, server);
        if w == 1 && kill {
            tokio::spawn(dying(client, KILLED));
        } else {
            tokio::spawn(async move { layer::serve(w, &mut client, step).await });
        }
    }
    let mut run = Run {
        states: Vec::new(),
        handoffs: 0,
        reassigned: 0,
        covered: Vec::new(),
    };
    for tick in 0..TICKS {
        if kill && tick == BACK {
            let (mut client, server) = rig.connect(1).await;
            run.reassigned += host.attach(1, server).len();
            tokio::spawn(async move { layer::serve(1, &mut client, step).await });
        }
        let report = host.tick(Duration::from_secs(10)).await;
        run.handoffs += report.handoffs.len();
        run.reassigned += report.reassigned;
        run.covered
            .extend(report.covered.iter().map(|&(from, to)| (tick, from, to)));
        run.states.push(snapshot(host.layer()));
    }
    run
}

fn first_difference(a: &[Vec<(EntityId, Bot)>], b: &[Vec<(EntityId, Bot)>]) -> Option<usize> {
    a.iter().zip(b).position(|(x, y)| x != y)
}

#[tokio::test]
async fn two_workers_over_quic_step_the_bots_as_one_does() {
    within(120, async {
        let run = over_the_transport(false).await;
        eprintln!("over QUIC: {} handoffs in {TICKS} ticks", run.handoffs);
        assert!(run.handoffs > 100, "{} handoffs", run.handoffs);
        assert!(run.covered.is_empty());
        assert_eq!(first_difference(&run.states, &reference()), None);
    })
    .await;
}

#[tokio::test]
async fn a_worker_killed_is_covered_without_losing_a_tick() {
    within(120, async {
        let run = over_the_transport(true).await;
        eprintln!(
            "worker killed: covered {:?}, {} reassigned, {} handoffs besides",
            run.covered, run.reassigned, run.handoffs
        );
        assert_eq!(run.covered, vec![(KILLED, 1, 0)]);
        assert!(run.reassigned > 150, "{} reassigned", run.reassigned);
        assert_eq!(first_difference(&run.states, &reference()), None);
    })
    .await;
}
