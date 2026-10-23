//! `forge-sim` — the simulation's clock and its inputs (Phase 3's step 2, issue #137; D-010,
//! D-016).
//!
//! A world advances in fixed ticks of 1/60 s ([`TICK`]). What players do reaches it as
//! commands stamped with the tick they act on ([`Stamped`]), applied in a fixed order whatever
//! order they arrived in; the world can be saved, restored and digested ([`Simulation`]). On
//! that rest three things:
//!
//! - [`Recording`]: a run's commands and its digests, written to a file and replayed against a
//!   fresh world, which must reach the same digests at the same ticks; a server records what it
//!   applies ([`Server::record`]) with its whole state every so often, so a replay can start at
//!   any tick from the keyframe before it ([`Recording::seek`]);
//! - [`Server`] and [`Client`]: the server owns the world and applies everyone's commands; a
//!   client runs ahead of it by the link's delay, predicts its own commands at once, and checks
//!   each snapshot the server sends against what it predicted for that tick: a match costs
//!   nothing, a mismatch (another player acted, an input came late) takes the client back to
//!   the server's state and replays its own commands since (D-010's reconciliation);
//! - [`Link`]: an in-process link that delays, jitters and loses packets, from a seed, so the
//!   whole scheme is tested in one process before any socket exists.
//!
//! A single-player game runs the same server in its own process: there is no other machine to
//! run the physics on, and nothing changes when a second player joins.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use forge_core::SplitMix64;

/// Ticks a second.
pub const TICK_RATE: u32 = 60;
/// Seconds a tick.
pub const TICK: f32 = 1.0 / TICK_RATE as f32;

/// A player of a session.
pub type PlayerId = u16;

/// A command stamped with the tick it acts on, its player and its number among that player's
/// commands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stamped<C> {
    /// The tick it acts on: applied in the step from that tick's state to the next.
    pub tick: u64,
    /// Whose it is.
    pub player: PlayerId,
    /// Its number among its player's commands, from 0.
    pub seq: u32,
    /// What it asks.
    pub command: C,
}

/// A command's bytes, exactly: what files and packets carry.
pub trait Codec: Sized {
    /// Appends its bytes.
    fn encode(&self, out: &mut Vec<u8>);
    /// Reads it back from the front of `bytes`, advancing them; `None` when they are not one.
    fn decode(bytes: &mut &[u8]) -> Option<Self>;
}

/// A world driven by the clock.
pub trait Simulation {
    /// What a player may ask for in a tick.
    type Command: Copy + PartialEq + std::fmt::Debug + Codec;
    /// Advances one tick with the commands stamped for it, already in their fixed order (by
    /// player, then by number).
    fn tick(&mut self, commands: &[Stamped<Self::Command>]);
    /// Ticks done since the start: the tick the next [`Simulation::tick`] acts on.
    fn now(&self) -> u64;
    /// The whole state, to restore.
    fn save(&mut self) -> Vec<u8>;
    /// Back to a saved state.
    fn restore(&mut self, state: &[u8]);
    /// A digest of the state to the bit: equal states, equal digests.
    fn digest(&mut self) -> u64;
}

/// Puts commands in the order a tick applies them: by player, then by number.
pub fn order<C>(commands: &mut [Stamped<C>]) {
    commands.sort_by_key(|c| (c.player, c.seq));
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn take<const N: usize>(bytes: &mut &[u8]) -> Option<[u8; N]> {
    let (head, rest) = bytes.split_first_chunk::<N>()?;
    *bytes = rest;
    Some(*head)
}

fn take_u64(bytes: &mut &[u8]) -> Option<u64> {
    take::<8>(bytes).map(u64::from_le_bytes)
}

impl<C: Codec> Codec for Stamped<C> {
    fn encode(&self, out: &mut Vec<u8>) {
        put_u64(out, self.tick);
        out.extend_from_slice(&self.player.to_le_bytes());
        out.extend_from_slice(&self.seq.to_le_bytes());
        self.command.encode(out);
    }

    fn decode(bytes: &mut &[u8]) -> Option<Self> {
        Some(Self {
            tick: take_u64(bytes)?,
            player: take::<2>(bytes).map(u16::from_le_bytes)?,
            seq: take::<4>(bytes).map(u32::from_le_bytes)?,
            command: C::decode(bytes)?,
        })
    }
}

/// A run as it happened: the commands, the digest every so many ticks and, for a replay that
/// starts part-way, the whole state every so many more.
#[derive(Clone, Debug, PartialEq)]
pub struct Recording<C> {
    /// Every command applied, in the order applied.
    pub commands: Vec<Stamped<C>>,
    /// `(tick, digest)`: the state's digest when that many ticks were done.
    pub digests: Vec<(u64, u64)>,
    /// Ticks the run lasted.
    pub ticks: u64,
    /// `(tick, state)`: the whole state ([`Simulation::save`]) when that many ticks were done,
    /// oldest first. A recording made as it runs ([`Recorder`], [`Server::record`]) starts with
    /// one; [`Recording::record`] keeps none.
    pub keyframes: Vec<(u64, Vec<u8>)>,
}

/// Why [`Recording::seek`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SeekError {
    /// The tick is before the recording's first keyframe or past its end.
    #[error("tick {0} is outside the recording (before its first keyframe or past its end)")]
    Outside(u64),
    /// The run from the keyframe left the recording.
    #[error(transparent)]
    Diverged(#[from] Divergence),
}

/// Where a replay left the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "the replay left the recording at tick {tick}: digest {got:#018x}, recorded {expected:#018x}"
)]
pub struct Divergence {
    /// The first tick whose digest differs.
    pub tick: u64,
    /// The recorded digest.
    pub expected: u64,
    /// The replay's.
    pub got: u64,
}

const RECORDING_MAGIC: &[u8; 8] = b"FRGREC02";
/// The first format, without keyframes: still read.
const RECORDING_MAGIC_V1: &[u8; 8] = b"FRGREC01";

impl<C: Copy + Codec> Recording<C> {
    /// An empty recording.
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            digests: Vec::new(),
            ticks: 0,
            keyframes: Vec::new(),
        }
    }

    /// Runs `sim` for `ticks` ticks from where it stands, applying `commands` (stamped from its
    /// current tick on), and records them with a digest every `every` ticks.
    pub fn record<S: Simulation<Command = C>>(
        sim: &mut S,
        mut commands: Vec<Stamped<C>>,
        ticks: u64,
        every: u64,
    ) -> Self {
        commands.sort_by_key(|c| (c.tick, c.player, c.seq));
        let start = sim.now();
        let mut recording = Self::new();
        let mut next = 0;
        for _ in 0..ticks {
            let now = sim.now();
            let first = next;
            while next < commands.len() && commands[next].tick <= now {
                next += 1;
            }
            let mut these: Vec<Stamped<C>> = commands[first..next]
                .iter()
                .map(|c| Stamped { tick: now, ..*c })
                .collect();
            order(&mut these);
            sim.tick(&these);
            recording.commands.extend(these);
            if (sim.now() - start).is_multiple_of(every.max(1)) {
                recording.digests.push((sim.now(), sim.digest()));
            }
        }
        recording.ticks = ticks;
        recording
    }

    /// Replays the recording on `sim`, which must stand where the recorded world stood when it
    /// began; the first digest that differs is the error.
    pub fn replay<S: Simulation<Command = C>>(&self, sim: &mut S) -> Result<(), Divergence> {
        let until = sim.now() + self.ticks;
        self.run(sim, until)
    }

    /// Sets `sim` to the recorded world at `tick`: the last keyframe at or before it restored,
    /// then the recorded commands of the ticks between run, the digests on the way checked. A
    /// replay can start anywhere so, without running from the start.
    ///
    /// For watching and checking a run, as [`Recording::replay`]: nothing here lets play go on
    /// from a recorded moment (Blizzard's US 9,089,777 family claims resuming play that way,
    /// `docs/research/netcode.md` §9).
    pub fn seek<S: Simulation<Command = C>>(
        &self,
        sim: &mut S,
        tick: u64,
    ) -> Result<(), SeekError> {
        let Some(&(start, _)) = self.keyframes.first() else {
            return Err(SeekError::Outside(tick));
        };
        if tick < start || tick > start + self.ticks {
            return Err(SeekError::Outside(tick));
        }
        let at = self.keyframes.partition_point(|k| k.0 <= tick) - 1;
        sim.restore(&self.keyframes[at].1);
        debug_assert_eq!(
            sim.now(),
            self.keyframes[at].0,
            "a keyframe restores its tick"
        );
        Ok(self.run(sim, tick)?)
    }

    /// The tick a recording made as it ran ([`Recorder`]) started at: its first keyframe's.
    pub fn start(&self) -> Option<u64> {
        self.keyframes.first().map(|k| k.0)
    }

    /// Runs `sim` from where it stands up to tick `until` with the recorded commands of those
    /// ticks, checking the digests recorded for them; the first that differs is the error.
    fn run<S: Simulation<Command = C>>(&self, sim: &mut S, until: u64) -> Result<(), Divergence> {
        let mut next = self.commands.partition_point(|c| c.tick < sim.now());
        let from = self.digests.partition_point(|d| d.0 <= sim.now());
        let mut checks = self.digests[from..].iter().peekable();
        while sim.now() < until {
            let now = sim.now();
            let first = next;
            while next < self.commands.len() && self.commands[next].tick == now {
                next += 1;
            }
            sim.tick(&self.commands[first..next]);
            if let Some(&&(tick, expected)) = checks.peek()
                && tick == sim.now()
            {
                checks.next();
                let got = sim.digest();
                if got != expected {
                    return Err(Divergence {
                        tick,
                        expected,
                        got,
                    });
                }
            }
        }
        Ok(())
    }

    /// The recording's bytes: a magic, the ticks, the commands, the digests, the keyframes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = RECORDING_MAGIC.to_vec();
        put_u64(&mut out, self.ticks);
        put_u64(&mut out, self.commands.len() as u64);
        for c in &self.commands {
            c.encode(&mut out);
        }
        put_u64(&mut out, self.digests.len() as u64);
        for &(tick, digest) in &self.digests {
            put_u64(&mut out, tick);
            put_u64(&mut out, digest);
        }
        put_u64(&mut out, self.keyframes.len() as u64);
        for (tick, state) in &self.keyframes {
            put_u64(&mut out, *tick);
            put_u64(&mut out, state.len() as u64);
            out.extend_from_slice(state);
        }
        out
    }

    /// A recording back from its bytes (either format); `None` when they are not one.
    pub fn from_bytes(mut bytes: &[u8]) -> Option<Self> {
        let bytes = &mut bytes;
        let magic = take::<8>(bytes)?;
        let keyframed = magic == *RECORDING_MAGIC;
        if !keyframed && magic != *RECORDING_MAGIC_V1 {
            return None;
        }
        let ticks = take_u64(bytes)?;
        let n = take_u64(bytes)?;
        let commands = (0..n)
            .map(|_| Stamped::decode(bytes))
            .collect::<Option<Vec<_>>>()?;
        let n = take_u64(bytes)?;
        let digests = (0..n)
            .map(|_| Some((take_u64(bytes)?, take_u64(bytes)?)))
            .collect::<Option<Vec<_>>>()?;
        let keyframes = if keyframed {
            let n = take_u64(bytes)?;
            (0..n)
                .map(|_| {
                    let tick = take_u64(bytes)?;
                    let len = usize::try_from(take_u64(bytes)?).ok()?;
                    let (state, rest) = bytes.split_at_checked(len)?;
                    *bytes = rest;
                    Some((tick, state.to_vec()))
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            Vec::new()
        };
        bytes.is_empty().then_some(Self {
            commands,
            digests,
            ticks,
            keyframes,
        })
    }
}

/// A recording made as a run goes (a server's, [`Server::record`]): the commands each tick
/// applied, a digest every `every` ticks and the whole state every `keyframe_every`, the first
/// at its start. The research's "a full snapshot every 10 s" is 600 ticks at 60 Hz.
#[derive(Debug)]
pub struct Recorder<C> {
    recording: Recording<C>,
    start: u64,
    every: u64,
    keyframe_every: u64,
}

impl<C: Copy + Codec> Recorder<C> {
    /// Starts recording `sim` from where it stands: a keyframe of it now.
    pub fn start<S: Simulation<Command = C>>(sim: &mut S, every: u64, keyframe_every: u64) -> Self {
        let mut recording = Recording::new();
        recording.keyframes.push((sim.now(), sim.save()));
        Self {
            recording,
            start: sim.now(),
            every: every.max(1),
            keyframe_every: keyframe_every.max(1),
        }
    }

    /// Takes the tick `sim` has just run with `commands`.
    pub fn ticked<S: Simulation<Command = C>>(&mut self, sim: &mut S, commands: &[Stamped<C>]) {
        let r = &mut self.recording;
        r.commands.extend_from_slice(commands);
        r.ticks += 1;
        let done = sim.now() - self.start;
        if done.is_multiple_of(self.every) {
            r.digests.push((sim.now(), sim.digest()));
        }
        if done.is_multiple_of(self.keyframe_every) {
            r.keyframes.push((sim.now(), sim.save()));
        }
    }

    /// What it has recorded so far.
    pub fn recording(&self) -> &Recording<C> {
        &self.recording
    }

    /// The recording, done.
    pub fn finish(self) -> Recording<C> {
        self.recording
    }
}

impl<C: Copy + Codec> Default for Recording<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// A link's delay and losses (D-010's link conditioner).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkParams {
    /// One way, seconds.
    pub delay: f64,
    /// Added to each packet's delay, uniform in 0..jitter, seconds.
    pub jitter: f64,
    /// The share of packets lost.
    pub loss: f64,
    /// The generator's seed: the same seed, the same delays and losses.
    pub seed: u64,
}

impl LinkParams {
    /// A link of `delay_ms` one way, a tenth of it as jitter, and `loss` of the packets lost.
    pub fn new(delay_ms: f64, loss: f64, seed: u64) -> Self {
        Self {
            delay: delay_ms / 1e3,
            jitter: delay_ms / 1e4,
            loss,
            seed,
        }
    }
}

/// One direction of an in-process link: packets go in with the time they are sent and come out
/// once their delay has passed, unless lost; a jittered packet may overtake an earlier one.
#[derive(Debug)]
pub struct Link<P> {
    params: LinkParams,
    random: SplitMix64,
    /// `(arrival, sent order, packet)`.
    in_flight: Vec<(f64, u64, P)>,
    sent: u64,
    /// Packets sent and lost so far.
    pub stats: LinkStats,
}

/// What a link carried.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    /// Packets sent.
    pub sent: u64,
    /// Packets lost.
    pub lost: u64,
    /// Bytes sent.
    pub bytes: u64,
}

impl<P> Link<P> {
    /// An empty link.
    pub fn new(params: LinkParams) -> Self {
        Self {
            params,
            random: SplitMix64::new(params.seed),
            in_flight: Vec::new(),
            sent: 0,
            stats: LinkStats::default(),
        }
    }

    fn unit(&mut self) -> f64 {
        self.random.next_f64()
    }

    /// Its delay from now on, seconds one way (a route that changes); what is in flight keeps
    /// its own.
    pub fn set_delay(&mut self, delay: f64) {
        self.params.delay = delay;
    }

    /// Sends `packet` of `bytes` bytes at `now` seconds.
    pub fn send(&mut self, now: f64, packet: P, bytes: usize) {
        self.stats.sent += 1;
        self.stats.bytes += bytes as u64;
        let (lost, jitter) = (self.unit() < self.params.loss, self.unit());
        if lost {
            self.stats.lost += 1;
            return;
        }
        let arrival = now + self.params.delay + jitter * self.params.jitter;
        self.in_flight.push((arrival, self.sent, packet));
        self.sent += 1;
    }

    /// The packets arrived by `now`, in their order of arrival.
    pub fn receive(&mut self, now: f64) -> Vec<P> {
        let mut arrived: Vec<(f64, u64, P)> = Vec::new();
        let mut k = 0;
        while k < self.in_flight.len() {
            if self.in_flight[k].0 <= now {
                arrived.push(self.in_flight.swap_remove(k));
            } else {
                k += 1;
            }
        }
        arrived.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        arrived.into_iter().map(|(_, _, p)| p).collect()
    }
}

/// What a client sends: its commands the server has not acknowledged, oldest first, so each is
/// sent again every tick until it is (D-010 repeats them; a lost packet loses none).
#[derive(Clone, Debug, PartialEq)]
pub struct InputPacket<C> {
    /// The sender.
    pub player: PlayerId,
    /// The sender's tick when it sent it (its world's `now`): how far ahead of the server it
    /// runs, which the server measures on arrival ([`Snapshot::leads`], D-010's pacing).
    pub tick: u64,
    /// Its unacknowledged commands, at most [`MAX_SENT`].
    pub commands: Vec<Stamped<C>>,
}

/// Packets a player's lead is measured over ([`Snapshot::leads`]): a second's at 60 Hz.
pub const LEAD_WINDOW: usize = 60;

/// A client's clock (D-010's pacing): the ticks it runs from the time that passes, a little faster
/// when the server reports its packets arriving with less lead than `target`, a little slower
/// with more. The rate follows the error alone (target less the measure), within ±10 %, and only
/// the simulation's clock: the display and the input devices keep theirs. Nothing waits on a
/// late input but the server, which takes it at its next tick; nothing is repeated or guessed.
/// Patents: `docs/research/netcode.md` §9 (the search for pacing, 2026-10-23).
#[derive(Clone, Debug)]
pub struct PaceClock {
    /// The lead on arrival aimed at, ticks.
    target: i64,
    /// The error left alone either side of the target, ticks ([`PaceClock::tolerance`]).
    tolerance: i64,
    /// Ticks per tick of time: 1 on target.
    rate: f64,
    /// Ticks owed, a fraction.
    owed: f64,
}

/// The rate's change per tick of lead missed, and its bounds.
const PACE_GAIN: f64 = 0.02;
const PACE_LIMIT: f64 = 0.1;

impl PaceClock {
    /// A clock aiming for packets that arrive `target` ticks ahead of the server.
    pub fn new(target: i64) -> Self {
        Self {
            target,
            tolerance: 0,
            rate: 1.0,
            owed: 0.0,
        }
    }

    /// Leaves the rate at 1 while the lead is within `ticks` of the target: no hunting about it
    /// when the measure wavers by a tick.
    pub fn tolerance(mut self, ticks: i64) -> Self {
        self.tolerance = ticks.max(0);
        self
    }

    /// The server's measure of the lead on arrival ([`Snapshot::leads`]): the rate from now on.
    pub fn report(&mut self, lead: i64) {
        let missed = self.target - lead;
        let error = (missed.signum() * (missed.abs() - self.tolerance).max(0)) as f64;
        self.rate = 1.0 + (error * PACE_GAIN).clamp(-PACE_LIMIT, PACE_LIMIT);
    }

    /// Its rate, ticks per tick of time.
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// The ticks to run for `ticks` ticks of time passed (a frame's time over [`TICK`]).
    pub fn advance(&mut self, ticks: f64) -> u32 {
        self.owed += ticks * self.rate;
        let run = self.owed.floor();
        self.owed -= run;
        run as u32
    }
}

/// Commands in an input packet at most: the oldest unacknowledged ones (the rest wait their
/// turn).
pub const MAX_SENT: usize = 16;

/// What the server sends: the state when `tick` ticks were done, its digest, and how far it has
/// taken each player's commands.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    /// Ticks done.
    pub tick: u64,
    /// The state's digest.
    pub digest: u64,
    /// The state, as [`Simulation::save`] wrote it.
    pub state: Vec<u8>,
    /// `(player, n)`: the player's commands numbered under `n` are all taken.
    pub acked: Vec<(PlayerId, u32)>,
    /// `(player, ticks)`: how far ahead of the server each player's packets were when they
    /// arrived, the least over the last [`LEAD_WINDOW`] (a measure only: the client paces
    /// itself to it, [`PaceClock`]).
    pub leads: Vec<(PlayerId, i64)>,
}

/// The commands taken of a player: all those numbered under `below`, and some after.
#[derive(Debug, Default)]
struct Taken {
    below: u32,
    ahead: BTreeSet<u32>,
}

impl Taken {
    /// Takes `seq`; false when it was taken already.
    fn take(&mut self, seq: u32) -> bool {
        if seq < self.below || !self.ahead.insert(seq) {
            return false;
        }
        while self.ahead.remove(&self.below) {
            self.below += 1;
        }
        true
    }
}

/// The authority: the world, and the commands waiting for their tick.
pub struct Server<S: Simulation> {
    /// Its world.
    pub sim: S,
    /// Commands by the tick they act on.
    waiting: BTreeMap<u64, Vec<Stamped<S::Command>>>,
    /// The commands taken of each player (a repeated one is dropped).
    taken: BTreeMap<PlayerId, Taken>,
    /// Each player's recent leads on arrival, newest last (at most [`LEAD_WINDOW`]).
    leads: BTreeMap<PlayerId, VecDeque<i64>>,
    /// With [`Server::record`]: what it applies, for a replay.
    recorder: Option<Recorder<S::Command>>,
    /// What it did.
    pub stats: ServerStats,
}

/// What a server did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// Commands applied.
    pub applied: u64,
    /// Commands that came after their tick, applied at the next tick instead.
    pub late: u64,
    /// Repeats dropped.
    pub repeats: u64,
}

impl<S: Simulation> Server<S> {
    /// A server over `sim`.
    pub fn new(sim: S) -> Self {
        Self {
            sim,
            waiting: BTreeMap::new(),
            taken: BTreeMap::new(),
            leads: BTreeMap::new(),
            recorder: None,
            stats: ServerStats::default(),
        }
    }

    /// Records from now on what it applies (D-010's replays): every player's commands at the
    /// ticks it applied them (a late one at the tick it was taken), a digest every `every`
    /// ticks and its whole state every `keyframe_every`, the first now. A replay of the
    /// recording ([`Recording::replay`], [`Recording::seek`]) is the session as the server ran it.
    pub fn record(&mut self, every: u64, keyframe_every: u64) {
        self.recorder = Some(Recorder::start(&mut self.sim, every, keyframe_every));
    }

    /// What it has recorded since [`Server::record`].
    pub fn recording(&self) -> Option<&Recording<S::Command>> {
        self.recorder.as_ref().map(Recorder::recording)
    }

    /// Takes an input packet: each command not seen yet waits for its tick, or for the next
    /// tick when its own has passed.
    /// How far ahead of it the sender ran is measured too ([`Snapshot::leads`]).
    pub fn receive(&mut self, packet: &InputPacket<S::Command>) {
        let leads = self.leads.entry(packet.player).or_default();
        if leads.len() == LEAD_WINDOW {
            leads.pop_front();
        }
        leads.push_back(packet.tick as i64 - self.sim.now() as i64);
        for c in &packet.commands {
            if !self.taken.entry(c.player).or_default().take(c.seq) {
                self.stats.repeats += 1;
                continue;
            }
            let now = self.sim.now();
            let tick = if c.tick < now {
                self.stats.late += 1;
                now
            } else {
                c.tick
            };
            self.waiting
                .entry(tick)
                .or_default()
                .push(Stamped { tick, ..*c });
        }
    }

    /// Runs one tick with the commands waiting for it.
    pub fn step(&mut self) {
        let now = self.sim.now();
        let mut commands = self.waiting.remove(&now).unwrap_or_default();
        order(&mut commands);
        self.stats.applied += commands.len() as u64;
        self.sim.tick(&commands);
        if let Some(r) = &mut self.recorder {
            r.ticked(&mut self.sim, &commands);
        }
    }

    /// The world as it stands, for the clients.
    pub fn snapshot(&mut self) -> Snapshot {
        Snapshot {
            tick: self.sim.now(),
            digest: self.sim.digest(),
            state: self.sim.save(),
            acked: self.taken.iter().map(|(&p, t)| (p, t.below)).collect(),
            leads: self
                .leads
                .iter()
                .filter_map(|(&p, l)| Some((p, *l.iter().min()?)))
                .collect(),
        }
    }
}

/// A player's view: its world run ahead of the server's, its own commands applied at once.
pub struct Client<S: Simulation> {
    /// Its predicted world.
    pub sim: S,
    /// Whose view.
    pub player: PlayerId,
    next_seq: u32,
    /// Its commands the server has not acknowledged, oldest first.
    unacked: VecDeque<Stamped<S::Command>>,
    /// Its commands for the coming ticks (made this frame, applied at the next tick).
    queued: Vec<Stamped<S::Command>>,
    /// The digest it predicted for each recent tick, to check the snapshots against.
    predicted: VecDeque<(u64, u64)>,
    /// The newest snapshot taken.
    last_snapshot: u64,
    /// What it did.
    pub stats: ClientStats,
}

/// What a client did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientStats {
    /// Snapshots taken.
    pub snapshots: u64,
    /// Those whose digest was the one predicted: nothing to do.
    pub matched: u64,
    /// Those it was taken back by.
    pub corrected: u64,
    /// Ticks run again after the corrections.
    pub replayed_ticks: u64,
    /// Snapshots older than one already taken, dropped.
    pub stale: u64,
}

/// Ticks of predicted digests a client keeps.
const PREDICTED: usize = 600;

impl<S: Simulation> Client<S> {
    /// A client over `sim`, the world as the server's stands. Run it ahead with [`Client::step`]
    /// as many ticks as the link's delay and a margin: it then knows the digests of those ticks
    /// when their snapshots come.
    pub fn new(sim: S, player: PlayerId) -> Self {
        Self {
            sim,
            player,
            next_seq: 0,
            unacked: VecDeque::new(),
            queued: Vec::new(),
            predicted: VecDeque::new(),
            last_snapshot: 0,
            stats: ClientStats::default(),
        }
    }

    /// Asks for `command` at the next tick.
    pub fn command(&mut self, command: S::Command) {
        let c = Stamped {
            tick: self.sim.now(),
            player: self.player,
            seq: self.next_seq,
            command,
        };
        self.next_seq += 1;
        self.queued.push(c);
    }

    /// Runs one tick with its own commands for it, and remembers the digest it predicts.
    pub fn step(&mut self) {
        let now = self.sim.now();
        let mut these: Vec<_> = std::mem::take(&mut self.queued)
            .into_iter()
            .map(|c| Stamped { tick: now, ..c })
            .collect();
        order(&mut these);
        self.unacked.extend(these.iter().copied());
        self.sim.tick(&these);
        let digest = self.sim.digest();
        self.predicted.push_back((self.sim.now(), digest));
        if self.predicted.len() > PREDICTED {
            self.predicted.pop_front();
        }
    }

    /// The packet to send this tick: its tick, and the oldest [`MAX_SENT`] unacknowledged
    /// commands (none, often: a packet goes every tick, so the server measures the lead).
    pub fn outgoing(&self) -> Option<InputPacket<S::Command>> {
        Some(InputPacket {
            player: self.player,
            tick: self.sim.now(),
            commands: self.unacked.iter().take(MAX_SENT).copied().collect(),
        })
    }

    /// Commands the server has not acknowledged yet.
    pub fn unacknowledged(&self) -> usize {
        self.unacked.len()
    }

    /// Takes a snapshot: drops the commands it acknowledges; when its digest is not the one
    /// predicted for its tick, goes back to its state and runs the ticks since again with the
    /// commands not yet acknowledged.
    pub fn apply(&mut self, snapshot: &Snapshot) {
        if snapshot.tick <= self.last_snapshot && self.stats.snapshots > 0 {
            self.stats.stale += 1;
            return;
        }
        self.last_snapshot = snapshot.tick;
        self.stats.snapshots += 1;
        if let Some(&(_, below)) = snapshot.acked.iter().find(|(p, _)| *p == self.player) {
            while self.unacked.front().is_some_and(|c| c.seq < below) {
                self.unacked.pop_front();
            }
        }
        let predicted = self
            .predicted
            .iter()
            .find(|&&(t, _)| t == snapshot.tick)
            .map(|&(_, d)| d);
        if predicted == Some(snapshot.digest) {
            self.stats.matched += 1;
            return;
        }
        // Back to the server's state, and forward again with what it has not seen yet.
        self.stats.corrected += 1;
        let target = self.sim.now();
        self.sim.restore(&snapshot.state);
        self.predicted.retain(|&(t, _)| t <= snapshot.tick);
        // A command whose tick the snapshot has passed, not taken yet, will be taken late: the
        // best guess is the snapshot's tick.
        let pending: Vec<_> = self
            .unacked
            .iter()
            .map(|c| Stamped {
                tick: c.tick.max(snapshot.tick),
                ..*c
            })
            .collect();
        while self.sim.now() < target {
            let now = self.sim.now();
            let these: Vec<_> = pending.iter().filter(|c| c.tick == now).copied().collect();
            self.sim.tick(&these);
            self.stats.replayed_ticks += 1;
            let digest = self.sim.digest();
            self.predicted.push_back((self.sim.now(), digest));
        }
    }
}

pub mod authority;
pub mod population;
pub mod rewind;
pub mod taxi;
pub mod transit;

#[cfg(test)]
mod tests;
