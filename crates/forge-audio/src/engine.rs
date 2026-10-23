//! The game thread's side: starting sounds, steering them, reading what ended.
//!
//! [`create`] makes an [`AudioEngine`] and its [`crate::Mixer`] joined by two rings: commands
//! go one way, finished voices (their sounds, to be dropped here) and their reasons come
//! back. The engine never waits on the mixer: when the command ring is full it keeps the
//! commands, in order, and sends them at the next call.
//!
//! A sound made in code for the moment (a footstep, an impact: [`AudioEngine::play_made`]) is
//! made on the engine's maker thread once [`AudioEngine::make_off_thread`] turns it on: its voice
//! is named at once, the controls sent to it wait with it, and the next [`AudioEngine::update`]
//! (the shell calls it at the start of each frame) waits for every sound asked for before it
//! and starts them in order. A frame's sounds are made while the frame goes on, and start one
//! frame later, the same in every run.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

use glam::{DVec3, Vec3};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::bus::{BusId, BusLayout};
use crate::clock::Clock;
use crate::dsp::{BusFilter, ReverbParams};
use crate::mixer::{Mixer, MixerConfig, MixerStats, SharedStats};
use crate::sound::SampleBuffer;
use crate::spatial::{Listener, Occlusion, Spatial};
use crate::stream::Stream;
use crate::voice::{MAX_SENDS, Source, StreamReader};

/// A voice, named by the engine when it starts it; never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VoiceId(pub u64);

/// How a voice starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayParams {
    /// The bus it plays into.
    pub bus: BusId,
    /// A linear gain.
    pub gain: f32,
    /// A playback rate factor (2: an octave up).
    pub pitch: f32,
    /// For a non-spatial voice: −1 left to 1 right (equal-power for mono, balance for
    /// stereo).
    pub pan: f32,
    /// What it keeps its place by when the budget is full: higher keeps.
    pub priority: u8,
    /// Its family (0: none), for the cap on voices of one kind at once that
    /// [`AudioEngine::set_family_cap`] sets (twenty growls at once, #318).
    pub family: u8,
    /// Plays again from the start at its end.
    pub looping: bool,
    /// When stolen or refused a place, keeps running silently and comes back when it is
    /// loud enough (sounds in memory only).
    pub virtualize: bool,
    /// Where it is, if it is placed in the world.
    pub spatial: Option<Spatial>,
    /// The frame it starts on (`None`: the next block).
    pub start: Option<u64>,
    /// Frames of fade-in (0: it starts on its first sample).
    pub fade_in: u32,
    /// Sends to other buses (an aux reverb): a bus and a linear level each.
    pub sends: [Option<(BusId, f32)>; MAX_SENDS],
}

impl Default for PlayParams {
    fn default() -> Self {
        Self {
            bus: BusId::MASTER,
            gain: 1.0,
            pitch: 1.0,
            pan: 0.0,
            priority: 128,
            family: 0,
            looping: false,
            virtualize: false,
            spatial: None,
            start: None,
            fade_in: 0,
            sends: [None; MAX_SENDS],
        }
    }
}

impl PlayParams {
    /// A spatial voice at `position`.
    pub fn at(position: DVec3) -> Self {
        Self {
            spatial: Some(Spatial::at(position)),
            ..Self::default()
        }
    }
}

/// A change to a voice, a bus or the listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    /// Stops a voice, fading out over `fade` frames from the command's frame.
    Stop {
        /// Which.
        voice: VoiceId,
        /// Frames of fade-out (0: silent from that frame).
        fade: u32,
    },
    /// A voice's gain.
    Gain {
        /// Which.
        voice: VoiceId,
        /// Linear.
        gain: f32,
    },
    /// A voice's playback rate factor.
    Pitch {
        /// Which.
        voice: VoiceId,
        /// 2: an octave up.
        pitch: f32,
    },
    /// A non-spatial voice's pan.
    Pan {
        /// Which.
        voice: VoiceId,
        /// −1 to 1.
        pan: f32,
    },
    /// A spatial voice's place and velocity.
    Position {
        /// Which.
        voice: VoiceId,
        /// Where.
        position: DVec3,
        /// Metres a second.
        velocity: Vec3,
    },
    /// What blocks a spatial voice's direct path.
    Occlusion {
        /// Which.
        voice: VoiceId,
        /// How much and through what.
        occlusion: Occlusion,
    },
    /// A voice's send level.
    VoiceSend {
        /// Which.
        voice: VoiceId,
        /// The send's slot in [`PlayParams::sends`].
        slot: usize,
        /// Linear.
        level: f32,
    },
    /// A bus's fader.
    BusGain {
        /// Which.
        bus: BusId,
        /// Linear.
        gain: f32,
    },
    /// A bus's send level.
    BusSend {
        /// Which.
        bus: BusId,
        /// The slot [`BusLayout::send`] returned.
        slot: usize,
        /// Linear.
        level: f32,
    },
    /// The listener.
    Listener(Listener),
    /// The parameters of the reverb on an aux bus.
    Reverb {
        /// Which.
        bus: BusId,
        /// The new parameters.
        params: ReverbParams,
    },
    /// The most voices of one family mixed at once (0: no cap). A new voice past the cap
    /// takes the place of its family's weakest when it beats it, and is refused (or kept
    /// virtual) otherwise.
    FamilyCap {
        /// Which (1 to 255).
        family: u8,
        /// Voices.
        cap: u16,
    },
    /// Where the filter of a bus that has one ([`BusLayout::set_filter`]) moves to.
    BusFilter {
        /// Which.
        bus: BusId,
        /// The new filter.
        filter: BusFilter,
    },
}

impl Control {
    /// The voice it acts on, if it acts on one.
    pub fn voice(&self) -> Option<VoiceId> {
        match *self {
            Control::Stop { voice, .. }
            | Control::Gain { voice, .. }
            | Control::Pitch { voice, .. }
            | Control::Pan { voice, .. }
            | Control::Position { voice, .. }
            | Control::Occlusion { voice, .. }
            | Control::VoiceSend { voice, .. } => Some(voice),
            _ => None,
        }
    }
}

/// Why a voice ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndReason {
    /// It played to its end.
    Finished,
    /// It was stopped.
    Stopped,
    /// A stronger voice took its place.
    Stolen,
    /// The budget was full of stronger voices when it started.
    Rejected,
}

/// What the mixer reports back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A voice ended.
    Ended {
        /// Which.
        voice: VoiceId,
        /// Why.
        reason: EndReason,
    },
}

pub(crate) struct PlayCommand {
    pub id: VoiceId,
    pub source: Source,
    pub params: PlayParams,
}

pub(crate) enum Command {
    Play(PlayCommand),
    Control(Control),
}

/// A command and the frame it acts on (`None`: the next block).
pub(crate) struct Timed {
    pub at: Option<u64>,
    pub cmd: Command,
}

/// A voice the mixer is done with: its sound, to be dropped on the game thread, and why it
/// ended (`None`: no event, the voice lives on elsewhere).
pub(crate) struct Returned {
    pub voice: VoiceId,
    pub reason: Option<EndReason>,
    pub source: Option<Source>,
}

/// The game thread's handle on the mixer.
pub struct AudioEngine {
    commands: Producer<Timed>,
    returns: Consumer<Returned>,
    backlog: VecDeque<Timed>,
    next_id: u64,
    clock: Clock,
    stats: Arc<SharedStats>,
    rate: u32,
    events: Vec<Event>,
    maker: Option<Maker>,
}

/// Something that makes a sound.
type Make = Box<dyn FnOnce() -> SampleBuffer + Send>;

/// The maker thread ([`AudioEngine::make_off_thread`]) and the sounds asked of it, in order:
/// each voice's parameters and the commands sent to it while it waits.
struct Maker {
    jobs: Option<Sender<Make>>,
    made: Receiver<SampleBuffer>,
    thread: Option<std::thread::JoinHandle<()>>,
    waiting: Vec<(VoiceId, PlayParams, Vec<Timed>)>,
}

impl Maker {
    fn start() -> std::io::Result<Self> {
        let (jobs, inbox) = std::sync::mpsc::channel::<Make>();
        let (outbox, made) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("forge-audio-maker".into())
            .spawn(move || {
                for make in inbox {
                    if outbox.send(make()).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            made,
            thread: Some(thread),
            waiting: Vec::new(),
        })
    }
}

impl Drop for Maker {
    fn drop(&mut self) {
        // Its inbox closed, the thread ends after the sound it is making.
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Makes a mixer for `layout` and the engine that drives it.
pub fn create(config: MixerConfig, layout: &BusLayout) -> (AudioEngine, Mixer) {
    let (command_tx, command_rx) = RingBuffer::new(config.command_capacity.max(1));
    let return_capacity = config.command_capacity
        + config.pending_capacity
        + config.max_voices
        + config.tail_voices
        + config.max_virtual;
    let (return_tx, return_rx) = RingBuffer::new(return_capacity);
    let clock = Clock::new();
    let stats = Arc::new(SharedStats::default());
    let mixer = Mixer::new(
        config,
        layout,
        command_rx,
        return_tx,
        clock.clone(),
        stats.clone(),
    );
    let engine = AudioEngine {
        commands: command_tx,
        returns: return_rx,
        backlog: VecDeque::new(),
        next_id: 1,
        clock,
        stats,
        rate: config.sample_rate,
        events: Vec::new(),
        maker: None,
    };
    (engine, mixer)
}

impl AudioEngine {
    /// Plays a sound held in memory.
    pub fn play(&mut self, sound: &Arc<SampleBuffer>, params: PlayParams) -> VoiceId {
        self.start(Source::Memory(sound.clone()), params)
    }

    /// Plays a stream ([`crate::Streams::open`]). Streams are never virtual: a stream that
    /// loses its place fades out and ends.
    pub fn play_stream(&mut self, stream: Stream, params: PlayParams) -> VoiceId {
        self.start(Source::Stream(Box::new(StreamReader::new(stream))), params)
    }

    fn start(&mut self, source: Source, params: PlayParams) -> VoiceId {
        let id = VoiceId(self.next_id);
        self.next_id += 1;
        self.send(Timed {
            at: params.start,
            cmd: Command::Play(PlayCommand { id, source, params }),
        });
        id
    }

    /// From now on [`AudioEngine::play_made`] makes its sounds on a thread of the engine's (the
    /// module's documentation), until the engine is dropped. A shell that calls
    /// [`AudioEngine::update`] each frame turns it on; without it each sound is made in the
    /// call. Errs when the thread cannot start, and the sounds are made in the call then.
    pub fn make_off_thread(&mut self) -> std::io::Result<()> {
        if self.maker.is_none() {
            self.maker = Some(Maker::start()?);
        }
        Ok(())
    }

    /// Plays the sound `make` makes (a footstep, an impact made in code for the moment): on the
    /// maker thread when it runs, started by the next [`AudioEngine::update`]; in the call
    /// otherwise. The voice is named at once, and the controls sent to it wait with it.
    pub fn play_made(
        &mut self,
        params: PlayParams,
        make: impl FnOnce() -> SampleBuffer + Send + 'static,
    ) -> VoiceId {
        let make: Make = Box::new(make);
        let sent = match self.maker.as_ref().and_then(|m| m.jobs.as_ref()) {
            Some(jobs) => jobs.send(make).map_err(|e| e.0),
            None => Err(make),
        };
        match (sent, &mut self.maker) {
            (Ok(()), Some(maker)) => {
                let id = VoiceId(self.next_id);
                self.next_id += 1;
                maker.waiting.push((id, params, Vec::new()));
                id
            }
            // No thread, or it ended: made here.
            (Err(make), _) => self.play(&Arc::new(make()), params),
            (Ok(()), None) => unreachable!("sent to a maker that is not there"),
        }
    }

    /// Starts every sound the maker thread was asked for, in order, waiting for those not made
    /// yet, each followed by the controls sent to it meanwhile.
    fn start_made(&mut self) {
        let Some(maker) = &mut self.maker else {
            return;
        };
        if maker.waiting.is_empty() {
            return;
        }
        let mut started = Vec::with_capacity(maker.waiting.len());
        for (id, params, held) in std::mem::take(&mut maker.waiting) {
            // The thread makes them in the order asked; a closed thread made none of the rest.
            let Ok(sound) = maker.made.recv() else {
                break;
            };
            started.push((id, params, sound, held));
        }
        for (id, params, sound, held) in started {
            self.send(Timed {
                at: params.start,
                cmd: Command::Play(PlayCommand {
                    id,
                    source: Source::Memory(Arc::new(sound)),
                    params,
                }),
            });
            for timed in held {
                self.send(timed);
            }
        }
    }

    /// Sends a control, or keeps it with its voice while that voice is being made.
    fn send_control(&mut self, timed: Timed) {
        if let (Some(maker), Command::Control(control)) = (&mut self.maker, &timed.cmd)
            && let Some(voice) = control.voice()
            && let Some(waiting) = maker.waiting.iter_mut().find(|w| w.0 == voice)
        {
            waiting.2.push(timed);
            return;
        }
        self.send(timed);
    }

    /// Applies a change at the next block.
    pub fn control(&mut self, control: Control) {
        self.send_control(Timed {
            at: None,
            cmd: Command::Control(control),
        });
    }

    /// Applies a change at `frame`: a stop on that very frame, any other change at the start
    /// of the block holding it (gains and positions are ramped across a block).
    pub fn control_at(&mut self, frame: u64, control: Control) {
        self.send_control(Timed {
            at: Some(frame),
            cmd: Command::Control(control),
        });
    }

    /// Stops a voice with a fade of `fade` frames.
    pub fn stop(&mut self, voice: VoiceId, fade: u32) {
        self.control(Control::Stop { voice, fade });
    }

    /// Caps the voices of `family` mixed at once (0: no cap), from the next block.
    pub fn set_family_cap(&mut self, family: u8, cap: u16) {
        self.control(Control::FamilyCap { family, cap });
    }

    /// Moves the listener.
    pub fn set_listener(&mut self, listener: Listener) {
        self.control(Control::Listener(listener));
    }

    fn send(&mut self, timed: Timed) {
        self.flush();
        if !self.backlog.is_empty() {
            self.backlog.push_back(timed);
            return;
        }
        if let Err(rtrb::PushError::Full(timed)) = self.commands.push(timed) {
            self.backlog.push_back(timed);
        }
    }

    fn flush(&mut self) {
        while let Some(timed) = self.backlog.pop_front() {
            if let Err(rtrb::PushError::Full(timed)) = self.commands.push(timed) {
                self.backlog.push_front(timed);
                break;
            }
        }
    }

    /// Starts the sounds the maker thread was asked for since the last call (waiting for them),
    /// sends what waited, drops what the mixer returned, and gives the events since the last
    /// call.
    pub fn update(&mut self) -> &[Event] {
        self.start_made();
        self.flush();
        self.events.clear();
        while let Ok(returned) = self.returns.pop() {
            if let Some(reason) = returned.reason {
                self.events.push(Event::Ended {
                    voice: returned.voice,
                    reason,
                });
            }
            // The sound or the stream's reader is dropped here, off the audio thread.
            drop(returned.source);
        }
        &self.events
    }

    /// Commands kept because the ring was full.
    pub fn backlog(&self) -> usize {
        self.backlog.len()
    }

    /// The mixer's clock.
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// The first frame the mixer has not rendered: the earliest a command can act on time.
    pub fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Frames a second.
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Frames in `seconds`.
    pub fn frames(&self, seconds: f64) -> u64 {
        (seconds * self.rate as f64).round() as u64
    }

    /// The mixer's counters.
    pub fn stats(&self) -> MixerStats {
        self.stats.snapshot()
    }
}
