//! `cargo run --release -p forge-audio --example audio-render -- <command>`: forge-audio's
//! headless runs (issue #267). Nothing here opens an audio device.
//!
//! - `scenes`: renders the demo scenes into WAV files (`captures/audio/` by default) for the
//!   owner to listen to, and prints what each did (voices, steals, the limiter's work).
//! - `bench`: the mixer's cost for 32, 64 and 128 spatial voices: the time to render one
//!   second of sound, the best of three runs (`docs/PROFILE.md`).
//! - `tone`: writes a stereo test tone (the source of the codec fixtures in `tests/data/`).

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use forge_audio::ambience::{
    Ambience, AmbienceAssets, AmbienceBuses, AmbienceState, Bed, BedKind, Medium, Shore,
};
use forge_audio::decode::Decoder;
use forge_audio::dsp::ReverbParams;
use forge_audio::sound::{noise, sine, sine_loop, stereo};
use forge_audio::spatial::{AcousticMaterial, Attenuation, Occlusion};
use forge_audio::stream::streams;
use forge_audio::zones::{ReverbZone, ReverbZones, ZoneShape};
use forge_audio::{
    AudioEngine, BLOCK, BusId, BusLayout, Control, Foley, Footwear, Impact, Listener, MixerConfig,
    MixerStats, OfflineRenderer, PlayParams, SampleBuffer, Step, Surface, WavFormat, create,
    decode, wav_bytes, write_wav,
};
use forge_core::SplitMix64;
use glam::{DVec3, Vec3};

const RATE: u32 = 48_000;

#[derive(Parser, Debug)]
#[command(
    about = "forge-audio rendered headless: scenes to WAV files, the mixer's cost, test tones"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Renders the demo scenes to WAV files.
    Scenes {
        /// Where the WAV files go.
        #[arg(long, default_value = "captures/audio")]
        out: PathBuf,
        /// Only the scene of this name.
        #[arg(long)]
        only: Option<String>,
    },
    /// Measures the mixer: one second of sound for 32, 64 and 128 voices.
    Bench {
        /// Seconds of sound a run renders.
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        /// Runs per count (the best is kept).
        #[arg(long, default_value_t = 3)]
        runs: usize,
    },
    /// Writes a stereo tone: a sine on each side.
    Tone {
        /// The WAV file.
        out: PathBuf,
        /// Hz on the left.
        #[arg(long, default_value_t = 440.0)]
        left: f64,
        /// Hz on the right.
        #[arg(long, default_value_t = 1000.0)]
        right: f64,
        /// Seconds.
        #[arg(long, default_value_t = 1.0)]
        seconds: f64,
    },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Command::Scenes { out, only } => scenes(&out, only.as_deref()),
        Command::Bench { seconds, runs } => {
            bench(seconds, runs);
            Ok(())
        }
        Command::Tone {
            out,
            left,
            right,
            seconds,
        } => {
            let sound = stereo(
                &sine(left, 0.5, seconds, RATE),
                &sine(right, 0.25, seconds, RATE),
            );
            write_wav(&out, sound.samples(), 2, RATE, WavFormat::Pcm16)?;
            println!(
                "{}: {left} Hz at 0.5 left, {right} Hz at 0.25 right, {seconds} s",
                out.display()
            );
            Ok(())
        }
    }
}

/// A scene: a name, a line on what to listen for, and how to render it.
struct Scene {
    name: &'static str,
    what: &'static str,
    render: fn() -> (Vec<f32>, MixerStats),
}

fn scenes(out: &Path, only: Option<&str>) -> Result<()> {
    #[allow(unused_mut)]
    let mut all = vec![
        Scene {
            name: "orbit",
            what: "a hiss of rain on a tin roof circling at 4 m every 4 s, a tone straight ahead",
            render: orbit,
        },
        Scene {
            name: "steal",
            what: "16 tones into a budget of 6, then one stopping every half second: low priorities give way, then come back",
            render: steal,
        },
        Scene {
            name: "doppler",
            what: "a buzz passing 4 m ahead at 30 m/s, left to right",
            render: doppler,
        },
        Scene {
            name: "limiter",
            what: "clicks and bursts 18 dB over full scale, held under -1 dBFS",
            render: limiter,
        },
        Scene {
            name: "stream",
            what: "a WAV made in memory, streamed and looped while a tone plays from memory",
            render: stream,
        },
        Scene {
            name: "wall",
            what: "rain and a buzz 6 m ahead; a wood door closes at 1.5 s and opens at 4.5 s, a concrete wall slides across at 7 s",
            render: wall,
        },
        Scene {
            name: "zones",
            what: "walking at 5 m/s from the open through a stone hall, out, and into a cave, clapping",
            render: zones,
        },
        Scene {
            name: "footsteps",
            what: "a walk in leather shoes over stone, gravel, grass, wood, metal and water, a run back; a crate on wood, a stone on metal, a sack into water; stone in trainers, boots, bare feet, sandals",
            render: footsteps,
        },
        Scene {
            name: "village",
            what: "a day in a minute: night, the dawn chorus, cicadas from mid-morning, wind and a shower in the afternoon, a walk to the quay, a dive at 45 s",
            render: village,
        },
    ];
    #[cfg(all(windows, feature = "steam-audio"))]
    all.push(Scene {
        name: "orbit-hrtf",
        what: "the orbit through Steam Audio's HRTF (headphones): the rain behind now sounds behind",
        render: orbit_hrtf,
    });
    for scene in all.iter().filter(|s| only.is_none_or(|o| o == s.name)) {
        let started = Instant::now();
        let (samples, stats) = (scene.render)();
        let seconds = samples.len() as f64 / 2.0 / RATE as f64;
        let path = out.join(format!("{}.wav", scene.name));
        write_wav(&path, &samples, 2, RATE, WavFormat::Pcm16)?;
        let peak = samples.iter().fold(0f32, |m, &x| m.max(x.abs()));
        println!(
            "{:8} {:5.1} s in {:5.0} ms, peak {:6.1} dBFS, started {:3}, stolen {:2}, virtualized {:2}, promoted {:2}, limiter {:4.1} dB deep, {} clamped: {}",
            scene.name,
            seconds,
            started.elapsed().as_secs_f64() * 1000.0,
            forge_audio::gain_to_db(peak),
            stats.started,
            stats.stolen,
            stats.virtualized,
            stats.promoted,
            stats.limiter_reduction_max_db,
            stats.limiter_overs,
            scene.what,
        );
        println!("         {}", path.display());
    }
    Ok(())
}

/// Renders `seconds` in steps of `step` frames, calling `each` with the frame before each.
fn run(
    engine: &mut AudioEngine,
    renderer: &mut OfflineRenderer,
    seconds: f64,
    step: usize,
    mut each: impl FnMut(&mut AudioEngine, u64),
) -> Vec<f32> {
    let frames = (seconds * RATE as f64) as usize;
    let mut out = Vec::with_capacity(frames * 2);
    let mut done = 0;
    while done < frames {
        each(engine, done as u64);
        let n = step.min(frames - done);
        out.extend(renderer.render(n));
        engine.update();
        done += n;
    }
    out
}

/// Noise in bursts: drops on a roof, in a loop.
fn rain(seed: u64, seconds: f64) -> Arc<SampleBuffer> {
    let base = noise(seed, 0.6, seconds, RATE);
    let mut rng = SplitMix64::new(seed ^ 0x5eed);
    let mut env = vec![0.0f32; base.frames()];
    let mut at = 0usize;
    while at < env.len() {
        let length = 600 + rng.below(1800) as usize;
        let level = rng.range_f32(0.2, 1.0);
        for i in 0..length.min(env.len() - at) {
            env[at + i] = env[at + i].max(level * (-(i as f32) / (length as f32 / 4.0)).exp());
        }
        at += 300 + rng.below(2400) as usize;
    }
    let data = base
        .samples()
        .iter()
        .zip(&env)
        .map(|(&x, &e)| x * (0.15 + 0.85 * e))
        .collect();
    SampleBuffer::mono(RATE, data).shared()
}

/// A buzz: odd and even harmonics of `frequency` falling as 1/n, looping.
fn buzz(frequency: f64, seconds: f64) -> Arc<SampleBuffer> {
    let base = sine_loop(frequency, 1.0, seconds, RATE);
    let period = base.frames() as f64 / (frequency * seconds).round();
    let data = (0..base.frames())
        .map(|i| {
            let phase = std::f64::consts::TAU * i as f64 / period;
            (1..=12)
                .map(|n| (phase * n as f64).sin() / n as f64)
                .sum::<f64>() as f32
                * 0.25
        })
        .collect();
    SampleBuffer::mono(RATE, data).shared()
}

fn orbit() -> (Vec<f32>, MixerStats) {
    orbit_with(false)
}

/// The orbit through Steam Audio's HRTF: listen on headphones; behind now sounds behind.
#[cfg(all(windows, feature = "steam-audio"))]
fn orbit_hrtf() -> (Vec<f32>, MixerStats) {
    orbit_with(true)
}

/// Gives `mixer` Steam Audio's HRTF when `hrtf` (and the feature) says so.
fn spatialize(mixer: &mut forge_audio::Mixer, config: &MixerConfig, hrtf: bool) {
    #[cfg(all(windows, feature = "steam-audio"))]
    if hrtf {
        let steam = forge_audio::steam::SteamSpatializer::for_mixer(config)
            .expect("Steam Audio starts (steam-audio-sdk/, docs/demos/audio.md)");
        mixer.set_spatializer(Box::new(steam));
    }
    #[cfg(not(all(windows, feature = "steam-audio")))]
    let _ = (mixer, config, hrtf);
}

fn orbit_with(hrtf: bool) -> (Vec<f32>, MixerStats) {
    let config = MixerConfig::default();
    let (mut engine, mut mixer) = create(config, &BusLayout::new());
    spatialize(&mut mixer, &config, hrtf);
    let mut renderer = OfflineRenderer::new(mixer);
    let hiss = rain(1, 2.0);
    let tone = sine_loop(220.0, 0.15, 1.0, RATE).shared();
    let mut ahead = PlayParams::at(DVec3::new(0.0, 0.0, -3.0));
    ahead.looping = true;
    ahead.gain = 2.0;
    engine.play(&tone, ahead);
    let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -4.0));
    params.looping = true;
    params.gain = 3.0;
    let voice = engine.play(&hiss, params);
    let out = run(&mut engine, &mut renderer, 8.0, 480, |engine, frame| {
        // Once round every 4 s, counter-clockwise from ahead: left, behind, right.
        let angle = frame as f64 / RATE as f64 * std::f64::consts::TAU / 4.0;
        let position = DVec3::new(-4.0 * angle.sin(), 0.0, -4.0 * angle.cos());
        engine.control(Control::Position {
            voice,
            position,
            velocity: Vec3::ZERO,
        });
    });
    (out, engine.stats())
}

fn steal() -> (Vec<f32>, MixerStats) {
    let config = MixerConfig {
        max_voices: 6,
        ..MixerConfig::default()
    };
    let (mut engine, mixer) = create(config, &BusLayout::new());
    let mut renderer = OfflineRenderer::new(mixer);
    let tones: Vec<_> = (0..16)
        .map(|i| sine_loop(200.0 * 1.12f64.powi(i), 0.08, 1.0, RATE).shared())
        .collect();
    let mut ids = Vec::new();
    let mut stopped = 0;
    let out = run(&mut engine, &mut renderer, 10.0, 480, |engine, frame| {
        // A tone every quarter second; from 4.5 s, the oldest still playing stops every half
        // second, and a virtual one takes its place.
        let started = ids.len();
        if started < 16 && frame >= started as u64 * RATE as u64 / 4 {
            ids.push(engine.play(
                &tones[started],
                PlayParams {
                    looping: true,
                    virtualize: true,
                    priority: [3, 1, 4, 1, 5, 9, 2, 6][started % 8],
                    pan: (started as f32 / 7.5) - 1.0,
                    ..PlayParams::default()
                },
            ));
        }
        if frame >= RATE as u64 * 9 / 2 + stopped as u64 * RATE as u64 / 2 && stopped < ids.len() {
            engine.stop(ids[stopped], 2400);
            stopped += 1;
        }
    });
    (out, engine.stats())
}

fn doppler() -> (Vec<f32>, MixerStats) {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let mut renderer = OfflineRenderer::new(mixer);
    let sound = buzz(160.0, 1.0);
    let speed = 30.0;
    let start = DVec3::new(-120.0, 0.0, -4.0);
    let mut params = PlayParams::at(start);
    params.looping = true;
    params.gain = 2.0;
    if let Some(s) = &mut params.spatial {
        s.velocity = Vec3::new(speed as f32, 0.0, 0.0);
        s.attenuation = Attenuation::Inverse {
            reference: 2.0,
            max: 500.0,
            rolloff: 1.0,
        };
    }
    let voice = engine.play(&sound, params);
    let out = run(&mut engine, &mut renderer, 8.0, BLOCK, |engine, frame| {
        let t = frame as f64 / RATE as f64;
        engine.control(Control::Position {
            voice,
            position: start + DVec3::new(speed * t, 0.0, 0.0),
            velocity: Vec3::new(speed as f32, 0.0, 0.0),
        });
    });
    (out, engine.stats())
}

fn limiter() -> (Vec<f32>, MixerStats) {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let mut renderer = OfflineRenderer::new(mixer);
    let burst = rain(7, 0.25);
    let click = forge_audio::sound::clicks(RATE as usize / 4, 0.9, RATE as usize, RATE).shared();
    engine.control(Control::BusGain {
        bus: BusId::MASTER,
        gain: forge_audio::db_to_gain(18.0),
    });
    engine.play(
        &click,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    let out = run(&mut engine, &mut renderer, 4.0, 480, |engine, frame| {
        if frame % (RATE as u64 / 2) == 0 {
            engine.play(
                &burst,
                PlayParams {
                    pan: -0.5,
                    ..PlayParams::default()
                },
            );
        }
    });
    (out, engine.stats())
}

fn stream() -> (Vec<f32>, MixerStats) {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let (opener, pump) = streams(0.25);
    // A 2 s stereo WAV at 44.1 kHz, made in memory: rain left, a low tone right.
    let left = noise(4, 0.2, 2.0, 44_100);
    let right = sine(110.0, 0.3, 2.0, 44_100);
    let bytes = wav_bytes(stereo(&left, &right).samples(), 2, 44_100, WavFormat::Pcm16);
    let decoder = decode::open(std::io::Cursor::new(bytes)).expect("a WAV decoder");
    let stream = opener.open(decoder, true).expect("a stream");
    engine.play_stream(
        stream,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    let tone = sine_loop(660.0, 0.1, 1.0, RATE).shared();
    engine.play(
        &tone,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    let mut renderer = OfflineRenderer::with_streams(mixer, pump);
    let out = run(&mut engine, &mut renderer, 6.0, 480, |_, _| {});
    (out, engine.stats())
}

fn wall() -> (Vec<f32>, MixerStats) {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let mut renderer = OfflineRenderer::new(mixer);
    // Rain and a tone 6 m ahead; a wood door closes between at 1.5 s and opens at 4.5 s, then a
    // concrete wall slides across at 7 s.
    let hiss = rain(3, 2.0);
    let tone = buzz(220.0, 1.0);
    let mut voices = Vec::new();
    for (sound, gain) in [(&hiss, 3.0), (&tone, 1.0)] {
        let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -6.0));
        params.looping = true;
        params.gain = gain;
        voices.push(engine.play(sound, params));
    }
    let out = run(&mut engine, &mut renderer, 10.0, 480, |engine, frame| {
        let t = frame as f64 / RATE as f64;
        let ramp = |from: f64| ((t - from) / 1.0).clamp(0.0, 1.0) as f32;
        let (amount, material) = if t < 7.0 {
            (ramp(1.5) - ramp(4.5), AcousticMaterial::WOOD)
        } else {
            (ramp(7.0), AcousticMaterial::CONCRETE)
        };
        for &voice in &voices {
            engine.control(Control::Occlusion {
                voice,
                occlusion: Occlusion::through(amount, &[material]),
            });
        }
    });
    (out, engine.stats())
}

fn zones() -> (Vec<f32>, MixerStats) {
    let mut layout = BusLayout::new();
    let reverb = layout.add_reverb("reverb", BusId::MASTER, ReverbParams::OPEN);
    let (mut engine, mixer) = create(MixerConfig::default(), &layout);
    let mut renderer = OfflineRenderer::new(mixer);
    // Out in the open, through a stone hall (20 m across, 6 m of fade), out again and into a
    // cave (a sphere of 15 m): the listener walks at 5 m/s, clapping every 0.6 s.
    let mut zones = ReverbZones::new(ReverbParams::OPEN);
    zones.add(ReverbZone {
        shape: ZoneShape::Box {
            min: DVec3::new(-10.0, -5.0, -10.0),
            max: DVec3::new(10.0, 15.0, 10.0),
        },
        fade: 6.0,
        params: ReverbParams::HALL,
    });
    zones.add(ReverbZone {
        shape: ZoneShape::Sphere {
            centre: DVec3::new(45.0, 0.0, 0.0),
            radius: 15.0,
        },
        fade: 4.0,
        params: ReverbParams::CAVE,
    });
    let clap = rain(9, 0.06);
    let mut next_clap = 0u64;
    let out = run(&mut engine, &mut renderer, 14.0, 480, |engine, frame| {
        let t = frame as f64 / RATE as f64;
        let position = DVec3::new(-25.0 + 5.0 * t, 0.0, 0.0);
        engine.set_listener(Listener::at(position));
        zones.follow(engine, reverb, position);
        if frame >= next_clap {
            let mut params = PlayParams::at(position + DVec3::new(0.0, 0.0, -0.5));
            params.sends[0] = Some((reverb, 0.6));
            engine.play(&clap, params);
            next_clap += RATE as u64 * 6 / 10;
        }
    });
    (out, engine.stats())
}

fn footsteps() -> (Vec<f32>, MixerStats) {
    let mut layout = BusLayout::new();
    let reverb = layout.add_reverb("reverb", BusId::MASTER, ReverbParams::OPEN);
    let (mut engine, mixer) = create(MixerConfig::default(), &layout);
    let mut renderer = OfflineRenderer::new(mixer);
    let mut params = PlayParams::default();
    params.sends[0] = Some((reverb, 0.25));
    let mut foley = Foley::new(RATE, 267, params);
    // The walker hears its own feet (played without a place, a little to the side of each
    // foot): walking at 1.4 m/s (a step every 0.55 s) in leather shoes over each surface for
    // 3.3 s, then running back at 4 m/s (a step every 0.33 s); three things fall 3 m ahead;
    // then on stone again, 3.3 s in each of the other shoes: trainers, boots, bare feet,
    // sandals.
    let mut next_step = 0u64;
    let mut foot = 0.1;
    let mut x = 0.0f64;
    let mut dropped = 0;
    let shoes = [
        Footwear::Sneakers,
        Footwear::Boots,
        Footwear::Barefoot,
        Footwear::Sandals,
    ];
    let out = run(&mut engine, &mut renderer, 50.0, 480, |engine, frame| {
        let t = frame as f64 / RATE as f64;
        let (speed, surface, footwear) = if t < 19.8 {
            (
                1.4,
                Surface::ALL[((t / 3.3) as usize).min(5)],
                Footwear::LeatherShoes,
            )
        } else if t < 36.0 {
            (
                4.0,
                Surface::ALL[5 - ((t - 19.8) / 2.0).min(5.0) as usize],
                Footwear::LeatherShoes,
            )
        } else {
            (
                1.4,
                Surface::Stone,
                shoes[(((t - 36.0) / 3.3) as usize).min(3)],
            )
        };
        engine.set_listener(Listener::at(DVec3::new(x, 1.6, 0.0)));
        if (t < 31.8 || (36.5..49.5).contains(&t)) && frame >= next_step {
            foot = -foot;
            foley.own_step(
                engine,
                &Step {
                    speed,
                    footwear,
                    start: Some(frame),
                    ..Step::walk(surface, DVec3::new(x + foot, 0.0, -0.2))
                },
                foot.signum() as f32,
            );
            let period = if speed > 2.0 { 0.33 } else { 0.55 };
            next_step = frame + (period * RATE as f64) as u64;
        }
        x += speed as f64 * 480.0 / RATE as f64 * if t < 19.8 { 1.0 } else { -1.0 };
        let falls = [
            (32.5, Surface::Wood, 20.0, 3.0),
            (33.7, Surface::Metal, 1.0, 5.0),
            (34.9, Surface::Water, 30.0, 4.0),
        ];
        if dropped < falls.len() && t >= falls[dropped].0 {
            let (_, surface, mass, speed) = falls[dropped];
            foley.impact(
                engine,
                &Impact {
                    surface,
                    position: DVec3::new(x, 0.0, -3.0),
                    speed,
                    mass,
                    start: Some(frame),
                },
            );
            dropped += 1;
        }
    });
    (out, engine.stats())
}

/// The village's day in a minute (3 s an hour from 3:00), the listener walking down to
/// the quay from 30 s and diving from 45 to 50 s.
fn village_state(t: f64) -> AmbienceState {
    let t = t as f32;
    let bump = |from: f32, to: f32| {
        let u = ((t - from) / (to - from)).clamp(0.0, 1.0);
        (std::f32::consts::PI * u).sin()
    };
    AmbienceState {
        hour: 3.0 + t / 3.0,
        wind: 1.0 + 9.0 * bump(26.0, 44.0),
        rain: 6.0 * bump(31.0, 40.0),
        shore: Some(Shore {
            distance: (300.0 - 10.0 * (t - 30.0).max(0.0)).max(3.0),
            direction: Vec3::X,
        }),
        medium: if (45.0..50.0).contains(&t) {
            Medium::Water
        } else {
            Medium::Air
        },
        ..AmbienceState::default()
    }
}

fn village() -> (Vec<f32>, MixerStats) {
    let mut layout = BusLayout::new();
    let buses = AmbienceBuses::add(&mut layout);
    let (mut engine, mixer) = create(MixerConfig::default(), &layout);
    let (opener, pump) = streams(0.25);
    // The recorded birds when tools/fetch-sounds.sh has fetched them.
    let assets = AmbienceAssets::load("assets/sounds", RATE).unwrap_or_default();
    let mut ambience = Ambience::start_with(
        &mut engine,
        &opener,
        buses,
        266,
        &village_state(0.0),
        &assets,
    )
    .expect("the beds start");
    let mut renderer = OfflineRenderer::with_streams(mixer, pump);
    let out = run(&mut engine, &mut renderer, 60.0, 800, |engine, frame| {
        let t = frame as f64 / RATE as f64;
        ambience.update(engine, &village_state(t), 800.0 / RATE as f32);
    });
    (out, engine.stats())
}

/// One run: `voices` spatial voices for `seconds`, moving every 10 ms. Returns the
/// mixer's seconds of work and its counters.
fn bench_run(voices: usize, seconds: f64, hrtf: bool) -> (f64, MixerStats) {
    let mut layout = BusLayout::new();
    let reverb = layout.add_reverb("reverb", BusId::MASTER, ReverbParams::HALL);
    let sfx = layout.add("sfx", BusId::MASTER);
    let config = MixerConfig {
        max_voices: voices.max(1),
        ..MixerConfig::default()
    };
    let (mut engine, mut mixer) = create(config, &layout);
    spatialize(&mut mixer, &config, hrtf);
    let mut renderer = OfflineRenderer::new(mixer);
    let sounds: Vec<_> = (0..8)
        .map(|i| {
            if i % 2 == 0 {
                rain(i as u64, 1.0)
            } else {
                sine_loop(150.0 * (i + 1) as f64, 0.2, 1.0, RATE).shared()
            }
        })
        .collect();
    let mut rng = SplitMix64::new(42);
    let mut ids = Vec::new();
    let mut orbits = Vec::new();
    for v in 0..voices {
        let radius = rng.range_f64(2.0, 40.0);
        let phase = rng.range_f64(0.0, std::f64::consts::TAU);
        let speed = rng.range_f64(-1.0, 1.0);
        orbits.push((radius, phase, speed));
        let mut params =
            PlayParams::at(DVec3::new(radius * phase.cos(), 1.0, radius * phase.sin()));
        params.bus = sfx;
        params.looping = true;
        params.pitch = rng.range_f32(0.8, 1.25);
        params.gain = 0.05;
        params.sends[0] = Some((reverb, 0.3));
        ids.push(engine.play(&sounds[v % sounds.len()], params));
    }
    engine.set_listener(Listener::default());
    let frames = (seconds * RATE as f64) as usize;
    let mut work = 0.0;
    let mut done = 0;
    let step = 480;
    while done < frames {
        let t = done as f64 / RATE as f64;
        for (id, &(radius, phase, speed)) in ids.iter().zip(&orbits) {
            let angle = phase + speed * t;
            engine.control(Control::Position {
                voice: *id,
                position: DVec3::new(radius * angle.cos(), 1.0, radius * angle.sin()),
                velocity: Vec3::new(
                    (-radius * speed * angle.sin()) as f32,
                    0.0,
                    (radius * speed * angle.cos()) as f32,
                ),
            });
        }
        let started = Instant::now();
        renderer.render(step);
        work += started.elapsed().as_secs_f64();
        engine.update();
        done += step;
    }
    (work / seconds, engine.stats())
}

fn bench(seconds: f64, runs: usize) {
    println!(
        "forge-audio mixer, {RATE} Hz, blocks of {BLOCK} frames ({:.2} ms), release build, one thread",
        BLOCK as f64 * 1000.0 / RATE as f64
    );
    println!(
        "each voice: a looping mono sound at a pitch of 0.8 to 1.25, placed (distance, air, Doppler, pan), moving, 0.3 sent to a hall reverb"
    );
    println!(
        "{:>7} {:>16} {:>12} {:>14} {:>14}",
        "voices", "ms a second", "% of a core", "µs a block", "max µs a block"
    );
    for voices in [0usize, 32, 64, 128] {
        let mut best = f64::MAX;
        let mut stats = MixerStats::default();
        for _ in 0..runs.max(1) {
            let (cost, s) = bench_run(voices, seconds, false);
            if cost < best {
                best = cost;
                stats = s;
            }
        }
        println!(
            "{voices:>7} {:>16.2} {:>12.2} {:>14.1} {:>14.1}",
            best * 1000.0,
            best * 100.0,
            stats.block_ns_mean as f64 / 1000.0,
            stats.block_ns_max as f64 / 1000.0
        );
    }
    #[cfg(all(windows, feature = "steam-audio"))]
    {
        println!(
            "the same voices through Steam Audio's HRTF (a binaural effect a voice, bilinear):"
        );
        for voices in [32usize, 64, 128] {
            let mut best = f64::MAX;
            let mut stats = MixerStats::default();
            for _ in 0..runs.max(1) {
                let (cost, s) = bench_run(voices, seconds, true);
                if cost < best {
                    best = cost;
                    stats = s;
                }
            }
            println!(
                "{voices:>7} {:>16.2} {:>12.2} {:>14.1} {:>14.1}",
                best * 1000.0,
                best * 100.0,
                stats.block_ns_mean as f64 / 1000.0,
                stats.block_ns_max as f64 / 1000.0
            );
        }
    }
    // The game thread's side: a step synthesised on each surface.
    println!("synthesising a walking step of 80 kg on each surface (the mean of 200):");
    for surface in Surface::ALL {
        let started = Instant::now();
        let mut seconds = 0.0;
        for seed in 0..200 {
            let sound =
                forge_audio::foley::step_sound(&Step::walk(surface, DVec3::ZERO), seed, RATE);
            seconds += sound.seconds();
        }
        let us = started.elapsed().as_secs_f64() / 200.0 * 1e6;
        println!(
            "  {:8} {us:6.0} µs a step ({:.2} s of sound)",
            surface.name(),
            seconds / 200.0
        );
    }
    // The decode thread's side: ten seconds of each bed.
    println!(
        "the beds on the decode thread, ten seconds each (the best of {} runs):",
        runs.max(1)
    );
    for (kind, intensity) in [
        (BedKind::Wind, 10.0),
        (BedKind::Rain, 2.0),
        (BedKind::Rain, 20.0),
        (BedKind::Cicadas, 1.0),
        (BedKind::Birds, 0.15),
        (BedKind::Birds, 1.0),
        (BedKind::Sea, 0.5),
        (BedKind::Underwater, 1.0),
    ] {
        let mut best = f64::MAX;
        for _ in 0..runs.max(1) {
            let (mut bed, _control) = Bed::new(kind, 5, RATE, intensity);
            let mut out = vec![0.0f32; RATE as usize * 20];
            let started = Instant::now();
            bed.read(&mut out).expect("a bed never fails");
            best = best.min(started.elapsed().as_secs_f64() / 10.0);
        }
        println!(
            "  {:10} at {intensity:5.2}: {:5.2} ms a second ({:.2} % of a core)",
            kind.name(),
            best * 1000.0,
            best * 100.0
        );
    }
    // The decode thread's side: a second of stereo 48 kHz from each codec.
    let source = stereo(&sine(440.0, 0.5, 1.0, RATE), &sine(1000.0, 0.25, 1.0, RATE));
    let wav = wav_bytes(source.samples(), 2, RATE, WavFormat::Pcm16);
    let files: [(&str, &[u8]); 3] = [
        ("WAV, 16-bit", &wav),
        (
            "Ogg Vorbis, 128 kbit/s",
            include_bytes!("../tests/data/tone-vorbis.ogg"),
        ),
        (
            "Ogg Opus, 64 kbit/s",
            include_bytes!("../tests/data/tone-opus.ogg"),
        ),
    ];
    println!(
        "decoding a second of stereo 48 kHz (the best of {} runs):",
        runs.max(1) * 10
    );
    for (name, bytes) in files {
        let mut best = f64::MAX;
        for _ in 0..runs.max(1) * 10 {
            let started = Instant::now();
            let mut decoder =
                decode::open(std::io::Cursor::new(bytes.to_vec())).expect("a decoder");
            let sound = decode::decode_all(decoder.as_mut()).expect("decoded");
            let cost = started.elapsed().as_secs_f64() / sound.seconds();
            best = best.min(cost);
        }
        println!(
            "  {name:24} {:6.2} ms ({:.2} % of a core while streaming)",
            best * 1000.0,
            best * 100.0
        );
    }
}
