//! The mixer: the audio thread's side.
//!
//! Each block (256 frames) it:
//! 1. takes the commands that arrived and those whose frame falls in this block (a command
//!    for a later block waits in a list sorted by frame);
//! 2. places every voice for the listener (distance curve, air, occlusion, Doppler, how loud
//!    it is at the listener) and lets the loudest virtual voices back into the budget;
//! 3. renders each voice into its bus and sends;
//! 4. runs the buses from the last added to the master, each feeding its parent and sends;
//! 5. runs the master's fader and the limiter, and writes the block out.
//!
//! It never allocates, frees, locks or blocks: every buffer is sized when it is made, a
//! sound or stream it has finished with goes back to the game thread through a ring, and
//! the counters it publishes are atomics.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use rtrb::{Consumer, Producer};

use crate::bus::{Bus, BusLayout, apply_gain, mix_into};
use crate::clock::Clock;
use crate::dsp::{Limiter, LimiterParams};
use crate::engine::{Command, Control, EndReason, PlayCommand, Returned, Timed, VoiceId};
use crate::spatial::{DirectPath, Listener, PanSpatializer, SPEED_OF_SOUND, Spatializer, place};
use crate::voice::{MAX_SENDS, NO_BUS, NO_SLOT, Scratch, Source, State, Voice, flat_gains};
use crate::{BLOCK, CHANNELS, DEFAULT_RATE};

/// How a mixer is sized.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerConfig {
    /// Frames a second.
    pub sample_rate: u32,
    /// Voices mixed at once: the budget.
    pub max_voices: usize,
    /// Voices kept running silently past the budget, to come back when they matter.
    pub max_virtual: usize,
    /// Stolen voices allowed to fade out beside the budget (a voice cut without a fade
    /// clicks).
    pub tail_voices: usize,
    /// Commands the ring between the game and the mixer holds; past it the engine keeps
    /// them until the mixer has room.
    pub command_capacity: usize,
    /// Commands waiting for a later block.
    pub pending_capacity: usize,
    /// Frames a stolen voice fades out over.
    pub steal_fade: u32,
    /// How much louder (as a gain ratio) a virtual voice must be than the quietest real
    /// voice of its priority to take its place: hysteresis, so two voices do not swap
    /// every block.
    pub promote_margin: f32,
    /// The master's limiter.
    pub limiter: LimiterParams,
    /// For Doppler, m/s.
    pub speed_of_sound: f32,
}

impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: DEFAULT_RATE,
            max_voices: 64,
            max_virtual: 256,
            tail_voices: 16,
            command_capacity: 1024,
            pending_capacity: 1024,
            steal_fade: 240,
            promote_margin: std::f32::consts::SQRT_2,
            limiter: LimiterParams::default(),
            speed_of_sound: SPEED_OF_SOUND,
        }
    }
}

/// The mixer's counters, as the game reads them ([`crate::AudioEngine::stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MixerStats {
    /// Frames rendered.
    pub frames: u64,
    /// Voices mixed in the last block (within the budget).
    pub real_voices: u32,
    /// Voices running silently.
    pub virtual_voices: u32,
    /// Stolen voices fading out.
    pub tail_voices: u32,
    /// Voices started (real or virtual).
    pub started: u64,
    /// Voices that lost their place to a stronger one.
    pub stolen: u64,
    /// Voices that never got a place.
    pub rejected: u64,
    /// Voices that went virtual (when stolen or when they could not get a place).
    pub virtualized: u64,
    /// Virtual voices that came back.
    pub promoted: u64,
    /// Commands applied after their frame had been rendered.
    pub late_commands: u64,
    /// Blocks in which a stream's ring ran dry.
    pub underruns: u64,
    /// Samples the limiter's final clamp caught (0 when it does its job).
    pub limiter_overs: u64,
    /// The limiter's deepest gain reduction in the last block, dB (0: none).
    pub limiter_reduction_db: f32,
    /// The deepest gain reduction so far, dB.
    pub limiter_reduction_max_db: f32,
    /// The last block's time on the audio thread, ns.
    pub block_ns: u64,
    /// The longest block so far, ns.
    pub block_ns_max: u64,
    /// The mean block time, ns.
    pub block_ns_mean: u64,
}

/// The counters as atomics: the mixer writes them once a block.
#[derive(Debug, Default)]
pub(crate) struct SharedStats {
    frames: AtomicU64,
    real: AtomicU32,
    virtuals: AtomicU32,
    tails: AtomicU32,
    started: AtomicU64,
    stolen: AtomicU64,
    rejected: AtomicU64,
    virtualized: AtomicU64,
    promoted: AtomicU64,
    late: AtomicU64,
    underruns: AtomicU64,
    overs: AtomicU64,
    reduction_db: AtomicU32,
    reduction_max_db: AtomicU32,
    block_ns: AtomicU64,
    block_ns_max: AtomicU64,
    block_ns_total: AtomicU64,
    blocks: AtomicU64,
}

impl SharedStats {
    pub fn snapshot(&self) -> MixerStats {
        let r = Ordering::Relaxed;
        let blocks = self.blocks.load(r).max(1);
        MixerStats {
            frames: self.frames.load(r),
            real_voices: self.real.load(r),
            virtual_voices: self.virtuals.load(r),
            tail_voices: self.tails.load(r),
            started: self.started.load(r),
            stolen: self.stolen.load(r),
            rejected: self.rejected.load(r),
            virtualized: self.virtualized.load(r),
            promoted: self.promoted.load(r),
            late_commands: self.late.load(r),
            underruns: self.underruns.load(r),
            limiter_overs: self.overs.load(r),
            limiter_reduction_db: f32::from_bits(self.reduction_db.load(r)),
            limiter_reduction_max_db: f32::from_bits(self.reduction_max_db.load(r)),
            block_ns: self.block_ns.load(r),
            block_ns_max: self.block_ns_max.load(r),
            block_ns_mean: self.block_ns_total.load(r) / blocks,
        }
    }
}

/// Counts kept on the audio thread and published each block.
#[derive(Default)]
struct Counts {
    started: u64,
    stolen: u64,
    rejected: u64,
    virtualized: u64,
    promoted: u64,
    late: u64,
    underruns: u64,
    block_ns_max: u64,
    block_ns_total: u64,
    blocks: u64,
}

/// The audio thread's mixer. Made by [`crate::create`]; render it from a device callback,
/// a [`crate::MixThread`] or an [`crate::OfflineRenderer`].
pub struct Mixer {
    config: MixerConfig,
    commands: Consumer<Timed>,
    returns: Producer<Returned>,
    /// What the return ring could not take yet; capacity reserved so pushing never
    /// allocates.
    overflow: Vec<Returned>,
    /// Commands for later blocks, sorted by frame, latest first.
    pending: Vec<Timed>,
    clock: Clock,
    stats: Arc<SharedStats>,
    counts: Counts,
    now: u64,
    voices: Vec<Voice>,
    virtuals: Vec<Voice>,
    real: usize,
    tails: usize,
    /// The most voices of each family mixed at once (0: no cap).
    family_caps: [u16; 256],
    free_slots: Vec<u16>,
    buses: Vec<Bus>,
    limiter: Limiter,
    listener: Listener,
    spatializer: Box<dyn Spatializer>,
    scratch: Scratch,
    /// The last block, interleaved, and how much of it was handed out.
    out: Box<[f32]>,
    out_at: usize,
}

impl Mixer {
    pub(crate) fn new(
        config: MixerConfig,
        layout: &BusLayout,
        commands: Consumer<Timed>,
        returns: Producer<Returned>,
        clock: Clock,
        stats: Arc<SharedStats>,
    ) -> Self {
        let slots = config.max_voices + config.tail_voices;
        assert!(slots < NO_SLOT as usize, "too many voices");
        let return_room =
            config.command_capacity + config.pending_capacity + slots + config.max_virtual;
        Self {
            config,
            commands,
            returns,
            overflow: Vec::with_capacity(return_room),
            pending: Vec::with_capacity(config.pending_capacity),
            clock,
            stats,
            counts: Counts::default(),
            now: 0,
            voices: Vec::with_capacity(slots),
            virtuals: Vec::with_capacity(config.max_virtual),
            real: 0,
            tails: 0,
            family_caps: [0; 256],
            free_slots: (0..slots as u16).rev().collect(),
            buses: layout
                .buses()
                .iter()
                .map(|spec| Bus::new(spec, BLOCK, config.sample_rate))
                .collect(),
            limiter: Limiter::new(config.limiter, config.sample_rate),
            listener: Listener::default(),
            spatializer: Box::new(PanSpatializer),
            scratch: Scratch::new(),
            out: vec![0.0; BLOCK * CHANNELS].into_boxed_slice(),
            out_at: BLOCK * CHANNELS,
        }
    }

    /// Replaces the panner (before the mixer starts: this is not a real-time call).
    pub fn set_spatializer(&mut self, spatializer: Box<dyn Spatializer>) {
        self.spatializer = spatializer;
    }

    /// Frames a second.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// The delay between a frame being mixed and leaving the mixer: the limiter's
    /// look-ahead, in frames.
    pub fn latency(&self) -> usize {
        self.limiter.latency()
    }

    /// The first frame of the next block.
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Fills `out` with interleaved stereo frames, rendering blocks as needed. Any length of
    /// whole frames.
    pub fn render(&mut self, out: &mut [f32]) {
        debug_assert_eq!(out.len() % CHANNELS, 0);
        let mut written = 0;
        while written < out.len() {
            if self.out_at == self.out.len() {
                self.render_block();
                self.out_at = 0;
            }
            let n = (out.len() - written).min(self.out.len() - self.out_at);
            out[written..written + n].copy_from_slice(&self.out[self.out_at..self.out_at + n]);
            written += n;
            self.out_at += n;
        }
    }

    fn render_block(&mut self) {
        let started = Instant::now();
        let now = self.now;
        let end = now + BLOCK as u64;
        self.flush_returns();

        // 1. Commands.
        while let Ok(timed) = self.commands.pop() {
            match timed.at {
                Some(at) if at >= end => self.defer(timed),
                _ => self.apply(timed),
            }
        }
        while self.pending.last().is_some_and(|t| t.at.unwrap_or(0) < end) {
            let timed = self.pending.pop().expect("a pending command");
            self.apply(timed);
        }

        // 2. Placement, then the virtual voices' turn.
        let speed = self.config.speed_of_sound;
        for v in &mut self.voices {
            let fresh = v.fresh;
            update_placement(v, &self.listener, speed, fresh);
        }
        for v in &mut self.virtuals {
            update_placement(v, &self.listener, speed, true);
        }
        self.promote();

        // 3. Voices.
        for bus in &mut self.buses {
            bus.clear();
        }
        // Voices render in the order they started, and leave in it (`Vec::remove` keeps the
        // order; a few voices end in a block), so their events come back in order.
        let mut underrun = false;
        for v in &mut self.voices {
            let r = v.render(
                now,
                &mut self.scratch,
                &mut self.buses,
                self.spatializer.as_mut(),
            );
            underrun |= r.underrun;
            v.done = r.done;
        }
        if underrun {
            self.counts.underruns += 1;
        }
        while let Some(i) = self.voices.iter().position(|v| v.done) {
            let v = self.voices.remove(i);
            let reason = v.end;
            self.release(v, reason.unwrap_or(EndReason::Finished), reason.is_some());
        }
        for v in &mut self.virtuals {
            v.done = v.skip(now);
        }
        while let Some(i) = self.virtuals.iter().position(|v| v.done) {
            let v = self.virtuals.remove(i);
            let reason = if v.stop_at.is_some() {
                EndReason::Stopped
            } else {
                EndReason::Finished
            };
            self.retire(v, reason, true);
        }

        // 4. Buses, the last added first.
        for b in (1..self.buses.len()).rev() {
            let (lower, upper) = self.buses.split_at_mut(b);
            let bus = &mut upper[0];
            if let Some(reverb) = &mut bus.reverb {
                if bus.active {
                    // Ring on for the decay time and a little more after the input stops.
                    let rt60 = reverb.params().rt60;
                    bus.tail =
                        (rt60 * 1.5 * self.config.sample_rate as f32 / BLOCK as f32) as u32 + 2;
                }
                if bus.tail > 0 {
                    reverb.process(&mut bus.left, &mut bus.right);
                    bus.tail -= 1;
                    bus.active = true;
                }
            }
            if let Some(filter) = &mut bus.filter {
                if bus.active {
                    filter.process(&mut bus.left, &mut bus.right);
                } else {
                    filter.skip(BLOCK);
                }
            }
            if !bus.active {
                bus.gain_prev = bus.gain;
                for send in &mut bus.sends {
                    send.2 = send.1;
                }
                continue;
            }
            apply_gain(&mut bus.left, &mut bus.right, bus.gain_prev, bus.gain);
            bus.gain_prev = bus.gain;
            if let Some(parent) = bus.parent {
                let parent = &mut lower[parent as usize];
                mix_into(
                    &bus.left,
                    &bus.right,
                    1.0,
                    1.0,
                    &mut parent.left,
                    &mut parent.right,
                );
                parent.active = true;
            }
            for send in &mut bus.sends {
                let target = &mut lower[send.0 as usize];
                mix_into(
                    &bus.left,
                    &bus.right,
                    send.2,
                    send.1,
                    &mut target.left,
                    &mut target.right,
                );
                target.active |= send.1 != 0.0 || send.2 != 0.0;
                send.2 = send.1;
            }
        }

        // 5. The master: fader, limiter, out.
        let master = &mut self.buses[0];
        apply_gain(
            &mut master.left,
            &mut master.right,
            master.gain_prev,
            master.gain,
        );
        master.gain_prev = master.gain;
        self.limiter.process(&mut master.left, &mut master.right);
        for (i, frame) in self
            .out
            .as_chunks_mut::<CHANNELS>()
            .0
            .iter_mut()
            .enumerate()
        {
            frame[0] = master.left[i];
            frame[1] = master.right[i];
        }

        self.now = end;
        self.clock.set(end);
        let ns = started.elapsed().as_nanos() as u64;
        self.counts.blocks += 1;
        self.counts.block_ns_total += ns;
        self.counts.block_ns_max = self.counts.block_ns_max.max(ns);
        self.publish(ns);
    }

    fn publish(&self, block_ns: u64) {
        let r = Ordering::Relaxed;
        let s = &self.stats;
        let c = &self.counts;
        s.frames.store(self.now, r);
        s.real.store(self.real as u32, r);
        s.virtuals.store(self.virtuals.len() as u32, r);
        s.tails.store(self.tails as u32, r);
        s.started.store(c.started, r);
        s.stolen.store(c.stolen, r);
        s.rejected.store(c.rejected, r);
        s.virtualized.store(c.virtualized, r);
        s.promoted.store(c.promoted, r);
        s.late.store(c.late, r);
        s.underruns.store(c.underruns, r);
        s.overs.store(self.limiter.overs(), r);
        let reduction = (-crate::gain_to_db(self.limiter.reduction())).max(0.0);
        s.reduction_db.store(reduction.to_bits(), r);
        let deepest = f32::from_bits(s.reduction_max_db.load(r)).max(reduction);
        s.reduction_max_db.store(deepest.to_bits(), r);
        s.block_ns.store(block_ns, r);
        s.block_ns_max.store(c.block_ns_max, r);
        s.block_ns_total.store(c.block_ns_total, r);
        s.blocks.store(c.blocks, r);
    }

    /// Keeps a command for its block.
    fn defer(&mut self, timed: Timed) {
        if self.pending.len() == self.pending.capacity() {
            // No room to wait: act now, late.
            self.counts.late += 1;
            self.apply(Timed { at: None, ..timed });
            return;
        }
        let at = timed.at.unwrap_or(0);
        // Latest first; a new command goes before (so after, in time) those of its frame.
        let index = self.pending.partition_point(|t| t.at.unwrap_or(0) > at);
        self.pending.insert(index, timed);
    }

    fn apply(&mut self, timed: Timed) {
        let at = match timed.at {
            Some(at) if at < self.now => {
                self.counts.late += 1;
                self.now
            }
            Some(at) => at,
            None => self.now,
        };
        match timed.cmd {
            Command::Play(play) => self.play(play, at),
            Command::Control(control) => self.control(control, at),
        }
    }

    fn play(&mut self, play: PlayCommand, at: u64) {
        let PlayCommand { id, source, params } = play;
        let (channels, rate) = match &source {
            Source::Memory(buffer) => (buffer.channels() as usize, buffer.rate()),
            Source::Stream(reader) => (reader.stream.channels as usize, reader.stream.rate),
        };
        let mut sends = [(NO_BUS, 0.0); MAX_SENDS];
        for (s, send) in params.sends.iter().enumerate() {
            if let Some((bus, level)) = send
                && bus.index() < self.buses.len()
            {
                sends[s] = (bus.0, *level);
            }
        }
        let bus = if params.bus.index() < self.buses.len() {
            params.bus.0
        } else {
            0
        };
        let mut voice = Voice {
            id,
            source,
            channels,
            base_step: rate as f64 / self.config.sample_rate as f64,
            bus,
            sends,
            gain: params.gain,
            pitch: params.pitch,
            pan: params.pan,
            priority: params.priority,
            family: params.family,
            looping: params.looping,
            virtualize: params.virtualize,
            spatial: params.spatial,
            pos: 0.0,
            start_at: at,
            stop_at: None,
            stop_fade: 0,
            env: crate::voice::Envelope::constant(if params.fade_in > 0 { 0.0 } else { 1.0 }),
            state: State::Playing,
            tail: false,
            end: Some(EndReason::Finished),
            slot: NO_SLOT,
            path: DirectPath::SILENT,
            prev_path: DirectPath::SILENT,
            bands: [1.0; 3],
            gains: [0.0; 2],
            prev_gains: [0.0; 2],
            send_levels: [0.0; MAX_SENDS],
            prev_send_levels: [0.0; MAX_SENDS],
            doppler: 1.0,
            audibility: 0.0,
            fresh: true,
            done: false,
            eq: crate::dsp::ThreeBand::new(self.config.sample_rate),
        };
        if params.fade_in > 0 {
            voice.env.ramp_to(1.0, params.fade_in);
        }
        self.counts.started += 1;
        update_placement(&mut voice, &self.listener, self.config.speed_of_sound, true);
        self.admit(voice);
    }

    /// Gives a new voice a place: a free one, the weakest voice's, a virtual one, or none.
    fn admit(&mut self, voice: Voice) {
        let family = voice.family;
        if at_cap(&self.family_caps, &self.family_counts(), family) {
            // Its family is at its cap: only in place of its own family's weakest.
            if let Some(w) = self.weakest_real(Some(family))
                && beats(score(&voice), score(&self.voices[w]), 1.0)
            {
                self.demote(w);
                self.start_real(voice);
            } else {
                self.park_or_reject(voice);
            }
            return;
        }
        if self.real < self.config.max_voices {
            self.start_real(voice);
            return;
        }
        let weakest = self.weakest_real(None);
        if let Some(w) = weakest
            && beats(score(&voice), score(&self.voices[w]), 1.0)
        {
            self.demote(w);
            self.start_real(voice);
            return;
        }
        self.park_or_reject(voice);
    }

    fn park_or_reject(&mut self, mut voice: Voice) {
        if voice.virtualize && voice.is_memory() && self.virtuals.len() < self.config.max_virtual {
            voice.slot = NO_SLOT;
            voice.fresh = true;
            self.counts.virtualized += 1;
            self.virtuals.push(voice);
        } else {
            self.counts.rejected += 1;
            self.retire(voice, EndReason::Rejected, true);
        }
    }

    fn start_real(&mut self, mut voice: Voice) {
        let slot = self.free_slots.pop().expect("a free voice slot");
        self.spatializer.reset(slot as usize);
        voice.slot = slot;
        voice.tail = false;
        self.real += 1;
        self.voices.push(voice);
    }

    /// Takes voice `w` out of the budget: it fades out as a tail (or is cut when there is
    /// no room for tails), and lives on as a virtual voice when it may.
    fn demote(&mut self, w: usize) {
        let mut old = self.voices.remove(w);
        self.real -= 1;
        let copy = if old.virtualize
            && old.state == State::Playing
            && old.stop_at.is_none()
            && self.virtuals.len() < self.config.max_virtual
        {
            old.copy_for_virtual()
        } else {
            None
        };
        if let Some(copy) = copy {
            self.counts.virtualized += 1;
            self.virtuals.push(copy);
            // The fading original ends quietly: the voice goes on.
            old.end = None;
        } else if old.state == State::Playing && old.stop_at.is_none() {
            self.counts.stolen += 1;
            old.end = Some(EndReason::Stolen);
        }
        // (A voice already stopping keeps its reason: it only fades out sooner.)
        if self.tails < self.config.tail_voices {
            old.tail = true;
            old.fade_out(self.config.steal_fade);
            self.tails += 1;
            self.voices.push(old);
        } else {
            let reason = old.end;
            self.release(old, reason.unwrap_or(EndReason::Stolen), reason.is_some());
        }
    }

    /// Lets the loudest virtual voices back, a few a block.
    fn promote(&mut self) {
        let mut counts = self.family_counts();
        // Families at their cap whose best virtual voice lost to their own weakest: the other
        // virtual voices still get their turn.
        let mut blocked = [false; 256];
        for _ in 0..4 {
            let now = self.now;
            let best = self
                .virtuals
                .iter()
                .enumerate()
                .filter(|(_, v)| v.start_at <= now && !blocked[v.family as usize])
                .max_by(|(_, a), (_, b)| cmp_score(score(a), score(b)))
                .map(|(i, _)| i);
            let Some(best) = best else { break };
            let family = self.virtuals[best].family;
            if at_cap(&self.family_caps, &counts, family) {
                if let Some(w) = self.weakest_real(Some(family))
                    && beats(
                        score(&self.virtuals[best]),
                        score(&self.voices[w]),
                        self.config.promote_margin,
                    )
                {
                    let v = self.virtuals.remove(best);
                    self.demote(w);
                    self.resume(v);
                } else {
                    blocked[family as usize] = true;
                }
                continue;
            }
            if self.real < self.config.max_voices {
                let v = self.virtuals.remove(best);
                counts[family as usize] += 1;
                self.resume(v);
                continue;
            }
            let Some(w) = self.weakest_real(None) else {
                break;
            };
            if !beats(
                score(&self.virtuals[best]),
                score(&self.voices[w]),
                self.config.promote_margin,
            ) {
                break;
            }
            let v = self.virtuals.remove(best);
            counts[self.voices[w].family as usize] -= 1;
            counts[family as usize] += 1;
            self.demote(w);
            self.resume(v);
        }
    }

    /// The voices of each family in the budget (fading tails left out).
    fn family_counts(&self) -> [u16; 256] {
        let mut counts = [0u16; 256];
        for v in self.voices.iter().filter(|v| !v.tail) {
            counts[v.family as usize] += 1;
        }
        counts
    }

    /// A virtual voice back in the budget, fading in where its time has reached.
    fn resume(&mut self, mut voice: Voice) {
        voice.env = crate::voice::Envelope::constant(0.0);
        voice.env.ramp_to(1.0, self.config.steal_fade);
        voice.fresh = true;
        voice.eq.reset();
        self.counts.promoted += 1;
        self.start_real(voice);
    }

    /// The real voice that would give up its place first: stopping voices, then the lowest
    /// priority, then the quietest at the listener.
    /// `family`: among one family's voices only.
    fn weakest_real(&self, family: Option<u8>) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.tail && family.is_none_or(|f| v.family == f))
            .min_by(|(_, a), (_, b)| cmp_score(score(a), score(b)))
            .map(|(i, _)| i)
    }

    fn control(&mut self, control: Control, at: u64) {
        match control {
            Control::Listener(listener) => self.listener = listener,
            Control::FamilyCap { family, cap } => self.family_caps[family as usize] = cap,
            Control::BusGain { bus, gain } => {
                if let Some(b) = self.buses.get_mut(bus.index()) {
                    b.gain = gain;
                }
            }
            Control::BusSend { bus, slot, level } => {
                if let Some(send) = self
                    .buses
                    .get_mut(bus.index())
                    .and_then(|b| b.sends.get_mut(slot))
                {
                    send.1 = level;
                }
            }
            Control::Reverb { bus, params } => {
                if let Some(reverb) = self
                    .buses
                    .get_mut(bus.index())
                    .and_then(|b| b.reverb.as_mut())
                {
                    reverb.set_params(params);
                }
            }
            Control::BusFilter { bus, filter } => {
                if let Some(f) = self
                    .buses
                    .get_mut(bus.index())
                    .and_then(|b| b.filter.as_mut())
                {
                    f.set(filter);
                }
            }
            Control::Stop { voice, fade } => self.stop(voice, fade, at),
            other => {
                let voice = other.voice().expect("a voice control");
                if let Some(v) = self.find(voice) {
                    apply_to_voice(v, &other);
                } else if let Some(p) = self.pending_play(voice) {
                    apply_to_params(p, &other);
                }
            }
        }
    }

    fn stop(&mut self, id: VoiceId, fade: u32, at: u64) {
        if let Some(i) = self.voices.iter().position(|v| v.id == id && !v.tail) {
            if at <= self.voices[i].start_at {
                // Stopped before it started: it never sounds.
                let v = self.voices.remove(i);
                self.release(v, EndReason::Stopped, true);
            } else {
                let v = &mut self.voices[i];
                v.stop_at = Some(at);
                v.stop_fade = fade;
                v.end = Some(EndReason::Stopped);
            }
        } else if let Some(v) = self.virtuals.iter_mut().find(|v| v.id == id) {
            // Ends on its frame (`Voice::skip`), or fades out if it comes back first.
            v.stop_at = Some(at);
            v.stop_fade = fade;
            v.end = Some(EndReason::Stopped);
        } else if let Some(i) = self
            .pending
            .iter()
            .position(|t| matches!(&t.cmd, Command::Play(p) if p.id == id))
        {
            let timed = self.pending.remove(i);
            if let Command::Play(play) = timed.cmd {
                self.push_return(Returned {
                    voice: id,
                    reason: Some(EndReason::Stopped),
                    source: Some(play.source),
                });
            }
        }
    }

    fn find(&mut self, id: VoiceId) -> Option<&mut Voice> {
        if let Some(i) = self.voices.iter().position(|v| v.id == id && !v.tail) {
            return Some(&mut self.voices[i]);
        }
        self.virtuals.iter_mut().find(|v| v.id == id)
    }

    fn pending_play(&mut self, id: VoiceId) -> Option<&mut crate::PlayParams> {
        self.pending.iter_mut().find_map(|t| match &mut t.cmd {
            Command::Play(p) if p.id == id => Some(&mut p.params),
            _ => None,
        })
    }

    /// A real or tail voice ends: free its slot and retire it.
    fn release(&mut self, voice: Voice, reason: EndReason, report: bool) {
        if voice.tail {
            self.tails -= 1;
        } else {
            self.real -= 1;
        }
        if voice.slot != NO_SLOT {
            self.free_slots.push(voice.slot);
        }
        self.retire(voice, reason, report);
    }

    /// Sends a voice's sound back to the game thread to be dropped there.
    fn retire(&mut self, voice: Voice, reason: EndReason, report: bool) {
        let id = voice.id;
        self.push_return(Returned {
            voice: id,
            reason: report.then_some(reason),
            source: Some(voice.take_source()),
        });
    }

    fn push_return(&mut self, returned: Returned) {
        if !self.overflow.is_empty() {
            self.flush_returns();
        }
        if let Err(rtrb::PushError::Full(returned)) = self.returns.push(returned) {
            if self.overflow.len() < self.overflow.capacity() {
                self.overflow.push(returned);
            } else {
                // Nowhere left: dropping here may free memory on the audio thread. The game
                // thread has stopped reading; this only happens when it is gone.
                drop(returned);
            }
        }
    }

    fn flush_returns(&mut self) {
        while let Some(returned) = self.overflow.pop() {
            if let Err(rtrb::PushError::Full(returned)) = self.returns.push(returned) {
                self.overflow.push(returned);
                break;
            }
        }
    }
}

/// A voice's claim to a place: priority first, then loudness at the listener; a voice
/// already stopping has none.
fn score(v: &Voice) -> (u8, f32) {
    if v.state == State::Stopping || v.stop_at.is_some() {
        (0, 0.0)
    } else {
        (v.priority, v.audibility)
    }
}

/// Whether `a` beats `b`: a higher priority, or the same and louder by more than `margin`.
fn beats(a: (u8, f32), b: (u8, f32), margin: f32) -> bool {
    a.0 > b.0 || (a.0 == b.0 && a.1 > b.1 * margin)
}

/// Whether `family` (not 0) has as many voices in the budget as its cap allows.
fn at_cap(caps: &[u16; 256], counts: &[u16; 256], family: u8) -> bool {
    let cap = caps[family as usize];
    family != 0 && cap > 0 && counts[family as usize] >= cap
}

fn cmp_score(a: (u8, f32), b: (u8, f32)) -> std::cmp::Ordering {
    a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
}

/// Recomputes a voice's path, bands, gains and send levels for the listener.
fn update_placement(v: &mut Voice, listener: &Listener, speed_of_sound: f32, fresh: bool) {
    match &v.spatial {
        Some(spatial) => {
            let p = place(listener, spatial, v.gain, speed_of_sound);
            v.path = p.path;
            v.bands = p.bands;
            v.audibility = p.audibility;
            // Doppler eases toward its target (about 25 ms) so a jump in a velocity does not
            // step the pitch.
            v.doppler = if fresh {
                p.doppler
            } else {
                v.doppler + (p.doppler - v.doppler) * 0.2
            };
            // The sends fall with the square root of the distance curve: the farther a
            // source, the more of it is its room's.
            let curve = if v.gain > 0.0 {
                p.path.gain / v.gain
            } else {
                0.0
            };
            let level = v.gain * curve.sqrt();
            for s in 0..MAX_SENDS {
                v.send_levels[s] = v.sends[s].1 * level;
            }
        }
        None => {
            v.gains = flat_gains(v.channels, v.gain, v.pan);
            v.audibility = v.gain;
            v.doppler = 1.0;
            for s in 0..MAX_SENDS {
                v.send_levels[s] = v.sends[s].1 * v.gain;
            }
        }
    }
}

fn apply_to_voice(v: &mut Voice, control: &Control) {
    match *control {
        Control::Gain { gain, .. } => v.gain = gain,
        Control::Pitch { pitch, .. } => v.pitch = pitch,
        Control::Pan { pan, .. } => v.pan = pan,
        Control::Position {
            position, velocity, ..
        } => {
            if let Some(s) = &mut v.spatial {
                s.position = position;
                s.velocity = velocity;
            }
        }
        Control::Occlusion { occlusion, .. } => {
            if let Some(s) = &mut v.spatial {
                s.occlusion = occlusion;
            }
        }
        Control::VoiceSend { slot, level, .. } => {
            if let Some(send) = v.sends.get_mut(slot) {
                send.1 = level;
            }
        }
        _ => {}
    }
}

fn apply_to_params(p: &mut crate::PlayParams, control: &Control) {
    match *control {
        Control::Gain { gain, .. } => p.gain = gain,
        Control::Pitch { pitch, .. } => p.pitch = pitch,
        Control::Pan { pan, .. } => p.pan = pan,
        Control::Position {
            position, velocity, ..
        } => {
            if let Some(s) = &mut p.spatial {
                s.position = position;
                s.velocity = velocity;
            }
        }
        Control::Occlusion { occlusion, .. } => {
            if let Some(s) = &mut p.spatial {
                s.occlusion = occlusion;
            }
        }
        Control::VoiceSend { slot, level, .. } => {
            if let Some(Some(send)) = p.sends.get_mut(slot) {
                send.1 = level;
            }
        }
        _ => {}
    }
}
