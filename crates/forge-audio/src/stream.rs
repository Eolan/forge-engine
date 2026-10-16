//! Streaming: long sounds decoded while they play.
//!
//! [`Streams::open`] takes a decoder, makes a ring of a quarter of a second (by default)
//! between it and the voice that will play it, fills the ring at once on the caller's
//! thread, and hands the decoder to the [`StreamPump`]. The pump keeps every open stream's
//! ring full: on a decode thread of its own ([`StreamPump::spawn`]), or called before each
//! block when rendering offline, which keeps an offline render the same to the bit however
//! fast the machine is. The audio thread only pops samples from the ring. A ring that runs
//! dry plays silence for the missing frames and counts an underrun
//! ([`crate::MixerStats::underruns`]); it never waits.
//!
//! When the voice ends, its end of the ring goes back to the game thread and is dropped
//! there; the pump sees the ring abandoned and drops the decoder.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::decode::{DecodeError, Decoder};

/// What the decode side tells the audio side about a stream.
#[derive(Debug, Default)]
pub(crate) struct StreamShared {
    /// Set after the last frame was pushed.
    pub eof: AtomicBool,
    /// Frames pushed in all, valid once `eof` is set.
    pub total_frames: AtomicU64,
    /// The decoder failed; the stream ended there.
    pub failed: AtomicBool,
}

/// The playing end of a stream: give it to [`crate::AudioEngine::play_stream`].
pub struct Stream {
    pub(crate) consumer: Consumer<f32>,
    pub(crate) channels: u16,
    pub(crate) rate: u32,
    pub(crate) shared: Arc<StreamShared>,
}

impl Stream {
    /// Channels in a frame.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Frames a second.
    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Frames waiting in the ring.
    pub fn buffered(&self) -> usize {
        self.consumer.slots() / self.channels as usize
    }

    /// Whether the decoder failed.
    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("channels", &self.channels)
            .field("rate", &self.rate)
            .field("buffered", &self.buffered())
            .finish()
    }
}

/// The decoding end of a stream.
struct Feed {
    decoder: Box<dyn Decoder>,
    producer: Producer<f32>,
    channels: usize,
    looping: bool,
    shared: Arc<StreamShared>,
    total: u64,
    done: bool,
}

/// Frames decoded at a time.
const CHUNK: usize = 2048;

impl Feed {
    /// Decodes until the ring is full or the stream ends; returns the frames decoded.
    fn fill(&mut self, scratch: &mut Vec<f32>) -> usize {
        let ch = self.channels;
        let mut decoded = 0;
        let mut rewound = false;
        while !self.done {
            let free = self.producer.slots() / ch;
            if free == 0 {
                break;
            }
            let want = free.min(CHUNK);
            scratch.resize(want * ch, 0.0);
            match self.decoder.read(&mut scratch[..want * ch]) {
                Ok(0) => {
                    // An empty file loops forever on nothing: end it instead.
                    if self.looping && !rewound && self.total > 0 {
                        rewound = true;
                        if self.decoder.rewind().is_err() {
                            self.finish(true);
                        }
                    } else {
                        self.finish(false);
                    }
                }
                Ok(n) => {
                    rewound = false;
                    let (pushed, rest) = self.producer.push_partial_slice(&scratch[..n * ch]);
                    debug_assert!(rest.is_empty() && pushed.len() == n * ch);
                    self.total += n as u64;
                    decoded += n;
                }
                Err(_) => self.finish(true),
            }
        }
        decoded
    }

    fn finish(&mut self, failed: bool) {
        self.shared
            .total_frames
            .store(self.total, Ordering::Release);
        if failed {
            self.shared.failed.store(true, Ordering::Release);
        }
        self.shared.eof.store(true, Ordering::Release);
        self.done = true;
    }
}

/// Opens streams, from any thread; their decoders go to the [`StreamPump`].
#[derive(Clone, Debug)]
pub struct Streams {
    sender: mpsc::Sender<Feed>,
    buffer_seconds: f32,
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("total", &self.total)
            .field("done", &self.done)
            .finish()
    }
}

/// Keeps every open stream's ring full.
pub struct StreamPump {
    receiver: mpsc::Receiver<Feed>,
    feeds: Vec<Feed>,
    scratch: Vec<f32>,
}

/// A stream opener and its pump, with rings of `buffer_seconds` of sound (0.25 is a good
/// default: a decode thread that wakes every 10 ms has 25 chances to refill a ring before it
/// runs dry).
pub fn streams(buffer_seconds: f32) -> (Streams, StreamPump) {
    let (sender, receiver) = mpsc::channel();
    (
        Streams {
            sender,
            buffer_seconds: buffer_seconds.max(0.02),
        },
        StreamPump {
            receiver,
            feeds: Vec::new(),
            scratch: Vec::with_capacity(CHUNK * 2),
        },
    )
}

impl Streams {
    /// Opens a stream on `decoder`: makes its ring, fills it, and passes the decoder to the
    /// pump. A looping stream rewinds the decoder at its end and never ends.
    pub fn open(&self, decoder: Box<dyn Decoder>, looping: bool) -> Result<Stream, DecodeError> {
        let channels = decoder.channels();
        if channels == 0 || channels > 2 {
            return Err(DecodeError::Channels(channels));
        }
        let rate = decoder.sample_rate();
        let frames = ((self.buffer_seconds * rate as f32).ceil() as usize).max(crate::BLOCK * 8);
        let (producer, consumer) = RingBuffer::new(frames * channels as usize);
        let shared = Arc::new(StreamShared::default());
        let mut feed = Feed {
            decoder,
            producer,
            channels: channels as usize,
            looping,
            shared: shared.clone(),
            total: 0,
            done: false,
        };
        let mut scratch = Vec::new();
        feed.fill(&mut scratch);
        if shared.failed.load(Ordering::Acquire) && feed.total == 0 {
            return Err(DecodeError::Format(
                "the stream failed before its first frame".into(),
            ));
        }
        if !feed.done {
            // A pump that is gone only means the stream stops at what was filled.
            let _ = self.sender.send(feed);
        }
        Ok(Stream {
            consumer,
            channels,
            rate,
            shared,
        })
    }
}

impl StreamPump {
    /// Takes the newly opened streams, refills every ring, and drops the streams that ended
    /// or whose voice is gone. Returns the frames decoded.
    pub fn pump(&mut self) -> usize {
        while let Ok(feed) = self.receiver.try_recv() {
            self.feeds.push(feed);
        }
        let mut decoded = 0;
        for feed in &mut self.feeds {
            if !feed.producer.is_abandoned() {
                decoded += feed.fill(&mut self.scratch);
            }
        }
        self.feeds.retain(|f| !f.done && !f.producer.is_abandoned());
        decoded
    }

    /// Streams being fed.
    pub fn open_streams(&self) -> usize {
        self.feeds.len()
    }

    /// Runs the pump on a thread of its own, every `period`, until the returned handle is
    /// dropped.
    pub fn spawn(mut self, period: Duration) -> PumpThread {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("forge-audio decode".into())
            .spawn(move || {
                while !flag.load(Ordering::Acquire) {
                    self.pump();
                    std::thread::sleep(period);
                }
            })
            .expect("spawning the decode thread");
        PumpThread {
            stop,
            handle: Some(handle),
        }
    }
}

/// The decode thread; stops and joins when dropped.
pub struct PumpThread {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for PumpThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
