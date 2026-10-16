//! A voice: one sound playing, read from memory or a stream, resampled, enveloped, filtered,
//! placed and added into its bus. Everything here runs on the audio thread: no allocation,
//! no lock, no I/O.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::BLOCK;
use crate::bus::Bus;
use crate::dsp::ThreeBand;
use crate::engine::{EndReason, VoiceId};
use crate::sound::SampleBuffer;
use crate::spatial::{DirectPath, Spatial, Spatializer, pan_gains, ramp_add};
use crate::stream::Stream;

/// Sends a voice may have besides its bus.
pub const MAX_SENDS: usize = 2;
/// The fastest a voice reads its source: two octaves up.
pub(crate) const MAX_STEP: f64 = 4.0;
/// The slowest: six octaves down.
pub(crate) const MIN_STEP: f64 = 1.0 / 64.0;
/// No bus.
pub(crate) const NO_BUS: u16 = u16::MAX;
/// No spatializer slot.
pub(crate) const NO_SLOT: u16 = u16::MAX;

/// Where a voice's samples come from.
pub(crate) enum Source {
    Memory(Arc<SampleBuffer>),
    Stream(Box<StreamReader>),
}

/// A stream's ring and the window of frames the interpolator reads from.
pub(crate) struct StreamReader {
    pub stream: Stream,
    /// Interleaved frames from `start` on; capacity reserved for a block at the fastest rate.
    window: Vec<f32>,
    start: i64,
}

impl StreamReader {
    pub fn new(stream: Stream) -> Self {
        let channels = stream.channels as usize;
        let capacity = (BLOCK as f64 * MAX_STEP) as usize + 8;
        Self {
            stream,
            window: Vec::with_capacity(capacity * channels),
            start: 0,
        }
    }

    fn frames(&self) -> i64 {
        (self.window.len() / self.stream.channels as usize) as i64
    }

    /// Makes the window cover frames `first..=last` as far as the ring allows. Returns
    /// whether it does (false: an underrun, or the stream's end).
    fn prepare(&mut self, first: i64, last: i64) -> bool {
        let ch = self.stream.channels as usize;
        // Drop what is behind.
        let behind = (first - self.start).clamp(0, self.frames());
        if behind > 0 {
            self.window.drain(..behind as usize * ch);
            self.start += behind;
        }
        // After an underrun the playhead ran ahead of the ring: skip the frames it passed.
        if self.window.is_empty() && first > self.start {
            let skip = ((first - self.start) as usize).min(self.stream.consumer.slots() / ch);
            if skip > 0
                && let Ok(chunk) = self.stream.consumer.read_chunk(skip * ch)
            {
                chunk.commit_all();
                self.start += skip as i64;
            }
        }
        // Pull up to `last`, within the window's capacity.
        let room = self.window.capacity() / ch - self.window.len() / ch;
        let need = (last + 1 - (self.start + self.frames())).max(0) as usize;
        let available = self.stream.consumer.slots() / ch;
        let take = need.min(room).min(available);
        if take > 0 {
            let old = self.window.len();
            self.window.resize(old + take * ch, 0.0);
            let (popped, _) = self
                .stream
                .consumer
                .pop_partial_slice(&mut self.window[old..]);
            let got = popped.len();
            self.window.truncate(old + got);
        }
        // Frames before the first are silence, not missing.
        self.start <= first.max(0) && self.start + self.frames() > last
    }

    #[inline]
    fn sample(&self, frame: i64, channel: usize) -> f32 {
        let ch = self.stream.channels as usize;
        let i = frame - self.start;
        if i >= 0 && i < self.frames() {
            self.window[i as usize * ch + channel]
        } else {
            0.0
        }
    }

    /// Frames in the whole stream, once its end has been decoded.
    fn total(&self) -> Option<u64> {
        let shared = &self.stream.shared;
        shared
            .eof
            .load(Ordering::Acquire)
            .then(|| shared.total_frames.load(Ordering::Acquire))
    }
}

/// A linear ramp of a gain toward a target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Envelope {
    pub value: f32,
    target: f32,
    step: f32,
    remaining: u32,
}

impl Envelope {
    pub fn constant(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            remaining: 0,
        }
    }

    pub fn ramp_to(&mut self, target: f32, frames: u32) {
        if frames == 0 {
            self.value = target;
            self.target = target;
            self.remaining = 0;
        } else {
            self.target = target;
            self.step = (target - self.value) / frames as f32;
            self.remaining = frames;
        }
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.value = if self.remaining == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }

    pub fn is_steady(&self) -> bool {
        self.remaining == 0
    }

    /// Frames until it reaches its target.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }
}

/// A voice's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Playing,
    /// Fading to its end: stopped, or stolen.
    Stopping,
}

/// The scratch buffers voices render through, one set for the mixer.
pub(crate) struct Scratch {
    a: Box<[f32]>,
    b: Box<[f32]>,
    left: Box<[f32]>,
    right: Box<[f32]>,
}

impl Scratch {
    pub fn new() -> Self {
        let block = || vec![0.0; BLOCK].into_boxed_slice();
        Self {
            a: block(),
            b: block(),
            left: block(),
            right: block(),
        }
    }
}

/// One sound playing.
pub(crate) struct Voice {
    pub id: VoiceId,
    pub source: Source,
    pub channels: usize,
    /// Source frames per output frame before pitch and Doppler.
    pub base_step: f64,
    pub bus: u16,
    pub sends: [(u16, f32); MAX_SENDS],
    pub gain: f32,
    pub pitch: f32,
    pub pan: f32,
    pub priority: u8,
    pub looping: bool,
    pub virtualize: bool,
    pub spatial: Option<Spatial>,

    pub pos: f64,
    pub start_at: u64,
    pub stop_at: Option<u64>,
    pub stop_fade: u32,
    pub env: Envelope,
    pub state: State,
    /// A stolen voice fading out, outside the budget.
    pub tail: bool,
    /// Why it will end (reported when it does), or `None` to end without an event (the
    /// fading copy of a voice that lives on as a virtual one).
    pub end: Option<EndReason>,
    pub slot: u16,

    // Computed each block; the `prev_` values are the last block's, for the ramps.
    pub path: DirectPath,
    pub prev_path: DirectPath,
    pub bands: [f32; 3],
    pub gains: [f32; 2],
    pub prev_gains: [f32; 2],
    pub send_levels: [f32; MAX_SENDS],
    pub prev_send_levels: [f32; MAX_SENDS],
    pub doppler: f32,
    pub audibility: f32,
    /// No block rendered yet: start the ramps at their targets.
    pub fresh: bool,
    /// Ended this block (set by the mixer while it renders).
    pub done: bool,
    pub eq: ThreeBand,
}

/// What rendering a block did.
pub(crate) struct Rendered {
    /// The voice has ended: retire it.
    pub done: bool,
    /// Its stream ran dry this block.
    pub underrun: bool,
}

impl Voice {
    /// Whether it plays from memory (and so can be copied and virtualized).
    pub fn is_memory(&self) -> bool {
        matches!(self.source, Source::Memory(_))
    }

    /// A copy that shares the sound, for a voice going virtual while the original fades out.
    pub fn copy_for_virtual(&self) -> Option<Voice> {
        let Source::Memory(buffer) = &self.source else {
            return None;
        };
        Some(Voice {
            id: self.id,
            source: Source::Memory(buffer.clone()),
            channels: self.channels,
            base_step: self.base_step,
            bus: self.bus,
            sends: self.sends,
            gain: self.gain,
            pitch: self.pitch,
            pan: self.pan,
            priority: self.priority,
            looping: self.looping,
            virtualize: self.virtualize,
            spatial: self.spatial,
            pos: self.pos,
            start_at: self.start_at,
            stop_at: self.stop_at,
            stop_fade: self.stop_fade,
            env: Envelope::constant(1.0),
            state: self.state,
            tail: false,
            end: self.end,
            slot: NO_SLOT,
            path: self.path,
            prev_path: self.path,
            bands: self.bands,
            gains: self.gains,
            prev_gains: self.gains,
            send_levels: self.send_levels,
            prev_send_levels: self.send_levels,
            doppler: self.doppler,
            audibility: self.audibility,
            fresh: true,
            done: false,
            eq: self.eq,
        })
    }

    /// Starts a fade to silence over `frames`, after which the voice ends.
    pub fn fade_out(&mut self, frames: u32) {
        if self.state == State::Stopping && self.env.remaining() <= frames && !self.env.is_steady()
        {
            return;
        }
        self.state = State::Stopping;
        self.stop_at = None;
        self.env.ramp_to(0.0, frames.max(1));
    }

    /// The playback step this block.
    fn step(&self) -> f64 {
        (self.base_step * self.pitch as f64 * self.doppler as f64).clamp(MIN_STEP, MAX_STEP)
    }

    /// Advances a voice that is not mixed by a block; returns true when it has ended.
    pub fn skip(&mut self, now: u64) -> bool {
        let begin = self.start_at.saturating_sub(now);
        if begin >= BLOCK as u64 {
            return false;
        }
        if let Some(stop) = self.stop_at
            && stop < now + BLOCK as u64
        {
            return true;
        }
        self.pos += self.step() * (BLOCK as u64 - begin) as f64;
        if let Source::Memory(buffer) = &self.source {
            let len = buffer.frames() as f64;
            if self.looping && len > 0.0 {
                self.pos %= len;
            } else if self.pos >= len {
                return true;
            }
        }
        false
    }

    /// Renders a block into the voice's bus and sends.
    pub fn render(
        &mut self,
        now: u64,
        scratch: &mut Scratch,
        buses: &mut [Bus],
        spatializer: &mut dyn Spatializer,
    ) -> Rendered {
        let mut rendered = Rendered {
            done: false,
            underrun: false,
        };
        let begin = self.start_at.saturating_sub(now);
        if begin >= BLOCK as u64 {
            return rendered;
        }
        let begin = begin as usize;
        if self.fresh {
            self.prev_path = self.path;
            self.prev_gains = self.gains;
            self.prev_send_levels = self.send_levels;
            self.fresh = false;
        }
        let spatial = self.spatial.is_some();
        let stereo = self.channels == 2 && !spatial;
        let step = self.step();

        // 1. The source, resampled.
        let n = BLOCK - begin;
        let produced = {
            let a = &mut scratch.a[begin..];
            let b = &mut scratch.b[begin..];
            match &mut self.source {
                Source::Memory(buffer) => {
                    read_memory(buffer, &mut self.pos, step, self.looping, a, b, stereo)
                }
                Source::Stream(reader) => {
                    let (produced, underrun) =
                        read_stream(reader, &mut self.pos, step, a, b, stereo);
                    rendered.underrun = underrun;
                    produced
                }
            }
        };
        scratch.a[begin + produced..].fill(0.0);
        if stereo {
            scratch.b[begin + produced..].fill(0.0);
        }
        if produced < n {
            rendered.done = true;
        }

        // 2. The envelope: fades in, stops (on their frame), fades out.
        let stop_offset = self
            .stop_at
            .filter(|&t| t < now + BLOCK as u64)
            .map(|t| t.saturating_sub(now) as usize);
        if !(self.env.is_steady() && self.env.value == 1.0 && stop_offset.is_none()) {
            for i in begin..BLOCK {
                if Some(i) == stop_offset.map(|s| s.max(begin)) {
                    self.state = State::Stopping;
                    self.stop_at = None;
                    self.env.ramp_to(0.0, self.stop_fade);
                }
                let g = self.env.next();
                scratch.a[i] *= g;
                if stereo {
                    scratch.b[i] *= g;
                }
            }
        }
        if self.state == State::Stopping && self.env.is_steady() && self.env.value == 0.0 {
            rendered.done = true;
        }

        // 3. Into the two channels.
        scratch.left[begin..].fill(0.0);
        scratch.right[begin..].fill(0.0);
        let (left, right) = (&mut scratch.left[begin..], &mut scratch.right[begin..]);
        if spatial {
            self.eq.process(&mut scratch.a[begin..], self.bands);
            spatializer.render(
                self.slot as usize,
                &self.prev_path,
                &self.path,
                &scratch.a[begin..],
                left,
                right,
            );
        } else if stereo {
            let (from, to) = (self.prev_gains, self.gains);
            let step_l = (to[0] - from[0]) / n as f32;
            let step_r = (to[1] - from[1]) / n as f32;
            for i in 0..n {
                let t = (i + 1) as f32;
                left[i] = scratch.a[begin + i] * (from[0] + step_l * t);
                right[i] = scratch.b[begin + i] * (from[1] + step_r * t);
            }
        } else {
            ramp_add(
                &scratch.a[begin..],
                self.prev_gains,
                self.gains,
                left,
                right,
            );
        }

        // 4. Into the bus, and the sends.
        let bus = &mut buses[self.bus as usize];
        bus.active = true;
        for i in 0..n {
            bus.left[begin + i] += scratch.left[begin + i];
            bus.right[begin + i] += scratch.right[begin + i];
        }
        for s in 0..MAX_SENDS {
            let (target, _) = self.sends[s];
            if target == NO_BUS {
                continue;
            }
            let (from, to) = (self.prev_send_levels[s], self.send_levels[s]);
            if from == 0.0 && to == 0.0 {
                continue;
            }
            let bus = &mut buses[target as usize];
            bus.active = true;
            if spatial {
                // The filtered mono signal, centred: the reverb folds it to mono anyway.
                let c = std::f32::consts::FRAC_1_SQRT_2;
                ramp_add(
                    &scratch.a[begin..],
                    [from * c, from * c],
                    [to * c, to * c],
                    &mut bus.left[begin..],
                    &mut bus.right[begin..],
                );
            } else {
                let b = if stereo { &scratch.b } else { &scratch.a };
                let step = (to - from) / n as f32;
                for i in 0..n {
                    let g = from + step * (i + 1) as f32;
                    bus.left[begin + i] += scratch.a[begin + i] * g;
                    bus.right[begin + i] += b[begin + i] * g;
                }
            }
        }

        self.prev_path = self.path;
        self.prev_gains = self.gains;
        self.prev_send_levels = self.send_levels;
        rendered
    }

    /// Takes the source out for retiring, leaving an empty sound behind.
    pub fn take_source(self) -> Source {
        self.source
    }
}

/// Non-spatial gains: equal-power pan for mono, balance for stereo.
pub(crate) fn flat_gains(channels: usize, gain: f32, pan: f32) -> [f32; 2] {
    if channels == 2 {
        let pan = pan.clamp(-1.0, 1.0);
        [gain * (1.0 - pan).min(1.0), gain * (1.0 + pan).min(1.0)]
    } else {
        let [l, r] = pan_gains(pan);
        [gain * l, gain * r]
    }
}

/// Catmull–Rom (third-order Hermite) interpolation between `x0` and `x1` at `t` in [0, 1):
/// exact on the samples (`t = 0` gives `x0`).
#[inline]
fn hermite(xm1: f32, x0: f32, x1: f32, x2: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x1 - xm1);
    let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// Reads from a sound in memory into `a` (and `b` for a stereo voice; a stereo sound into a
/// mono voice is folded down). Returns the frames written: fewer than asked at the end of a
/// sound that does not loop.
fn read_memory(
    buffer: &SampleBuffer,
    pos: &mut f64,
    step: f64,
    looping: bool,
    a: &mut [f32],
    b: &mut [f32],
    stereo: bool,
) -> usize {
    let len = buffer.frames() as i64;
    if len == 0 {
        return 0;
    }
    let ch = buffer.channels() as usize;
    let data = buffer.samples();
    let lenf = len as f64;
    let fetch = |k: i64, c: usize| -> f32 {
        if (0..len).contains(&k) {
            data[k as usize * ch + c]
        } else if looping {
            data[k.rem_euclid(len) as usize * ch + c]
        } else {
            0.0
        }
    };
    for i in 0..a.len() {
        if !looping && *pos >= lenf {
            return i;
        }
        let base = pos.floor();
        let t = (*pos - base) as f32;
        let k = base as i64;
        let value = |c: usize| -> f32 {
            if t == 0.0 {
                return fetch(k, c);
            }
            if k >= 1 && k + 2 < len {
                let at = k as usize * ch + c;
                hermite(data[at - ch], data[at], data[at + ch], data[at + 2 * ch], t)
            } else {
                hermite(
                    fetch(k - 1, c),
                    fetch(k, c),
                    fetch(k + 1, c),
                    fetch(k + 2, c),
                    t,
                )
            }
        };
        if ch == 1 {
            a[i] = value(0);
        } else if stereo {
            a[i] = value(0);
            b[i] = value(1);
        } else {
            a[i] = 0.5 * (value(0) + value(1));
        }
        *pos += step;
        if looping && *pos >= lenf {
            *pos -= lenf;
        }
    }
    a.len()
}

/// Reads from a stream. Returns the frames written (fewer at the stream's end) and whether
/// the ring ran dry.
fn read_stream(
    reader: &mut StreamReader,
    pos: &mut f64,
    step: f64,
    a: &mut [f32],
    b: &mut [f32],
    stereo: bool,
) -> (usize, bool) {
    let n = a.len();
    let first = pos.floor() as i64 - 1;
    let last = (*pos + step * n as f64).floor() as i64 + 2;
    let covered = reader.prepare(first, last);
    let total = reader.total();
    let ch = reader.stream.channels as usize;
    for i in 0..n {
        if let Some(total) = total
            && *pos >= total as f64
        {
            return (i, false);
        }
        let base = pos.floor();
        let t = (*pos - base) as f32;
        let k = base as i64;
        let value = |c: usize| -> f32 {
            if t == 0.0 {
                reader.sample(k, c)
            } else {
                hermite(
                    reader.sample(k - 1, c),
                    reader.sample(k, c),
                    reader.sample(k + 1, c),
                    reader.sample(k + 2, c),
                    t,
                )
            }
        };
        if ch == 1 {
            a[i] = value(0);
        } else if stereo {
            a[i] = value(0);
            b[i] = value(1);
        } else {
            a[i] = 0.5 * (value(0) + value(1));
        }
        *pos += step;
    }
    // Short of frames without being at the end: the ring ran dry.
    (n, !covered && total.is_none())
}
