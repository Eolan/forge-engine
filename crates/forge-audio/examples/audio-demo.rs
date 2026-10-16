//! `cargo run --release -p forge-audio --features device --example audio-demo`: forge-audio
//! through the audio device (issue #267). It opens the default output device through `cpal`
//! (WASAPI), so the owner starts it; no test, script or session runs it.
//!
//! The village of #266 in sound: the ambience beds driven by the hour, the weather, the
//! distance to the quay and the medium, and the walker's own steps. Commands are typed on the
//! terminal (`help` lists them); `--tour` plays a two-and-a-half-minute tour of them. The mixer
//! runs on its mix thread ahead of a ring of `--ring` frames, the beds on the decode thread,
//! the device's callback only copies out of the ring (`docs/demos/audio.md`).

#![forbid(unsafe_code)]

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "audio-demo opens the device through cpal, which forge-audio builds on Windows only for \
         now (WASAPI); `audio-render scenes` renders the same sounds to WAV files"
    );
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    device::run()
}

#[cfg(windows)]
mod device {
    use std::io::BufRead;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow, bail};
    use clap::Parser;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, SizedSample};
    use forge_audio::ambience::{Ambience, AmbienceBuses, AmbienceState, Medium, Shore};
    use forge_audio::dsp::ReverbParams;
    use forge_audio::stream::streams;
    use forge_audio::{
        AudioEngine, BusId, BusLayout, Control, Foley, Impact, Listener, MixThread, MixerConfig,
        OutputRing, PlayParams, Step, Surface, create, db_to_gain,
    };
    use glam::{DVec3, Vec3};

    #[derive(Parser, Debug)]
    #[command(
        about = "forge-audio through the audio device: the village in sound, driven from the terminal"
    )]
    struct Args {
        /// Lists the output devices and stops.
        #[arg(long)]
        list: bool,
        /// Plays the tour at once.
        #[arg(long)]
        tour: bool,
        /// Stops after this many seconds (0: at `quit`).
        #[arg(long, default_value_t = 0.0)]
        seconds: f64,
        /// Frames in the ring between the mix thread and the device (the latency it adds).
        #[arg(long, default_value_t = 1024)]
        ring: usize,
    }

    const HELP: &str = "\
commands (one a line):
  time <h>               the hour, 0 to 24 (cicadas in the heat of the day, birds at dawn)
  lapse <h>              hours that pass a minute (0: the clock stands)
  temp <C>               the temperature (cicadas sing above 22 C)
  wind <m/s>             the wind
  rain <mm/h>            the rain
  sea <m> | sea off      the distance to the quay, on the right
  walk <surface>         walk (stone, gravel, grass, wood, metal, water)
  run <surface>          run
  stop                   stand still
  drop <surface> <kg> <m/s>   something falls 3 m ahead
  dive | up              the head under water, or out
  volume <dB>            the master's level
  tour                   the tour (about 150 s)
  status                 the state and the mixer's counters
  help | quit";

    /// The tour: commands at their second.
    const TOUR: &[(f64, &str)] = &[
        (0.0, "time 3"),
        (0.0, "temp 26"),
        (0.0, "lapse 0"),
        (4.0, "time 6.3"),
        (16.0, "walk stone"),
        (21.0, "walk gravel"),
        (26.0, "walk grass"),
        (31.0, "walk wood"),
        (36.0, "walk metal"),
        (41.0, "run stone"),
        (45.0, "stop"),
        (46.0, "time 13"),
        (46.0, "temp 31"),
        (58.0, "wind 9"),
        (66.0, "rain 6"),
        (78.0, "rain 0"),
        (78.0, "wind 3"),
        (82.0, "sea 60"),
        (88.0, "sea 20"),
        (94.0, "sea 4"),
        (100.0, "walk water"),
        (105.0, "stop"),
        (107.0, "dive"),
        (117.0, "up"),
        (121.0, "drop wood 20 3"),
        (123.0, "drop metal 1 5"),
        (125.0, "drop water 30 4"),
        (128.0, "time 20.3"),
        (128.0, "temp 24"),
        (142.0, "time 22.5"),
        (150.0, "sea off"),
    ];

    /// The world the sounds come from, as the terminal sets it.
    struct World {
        state: AmbienceState,
        lapse: f32,
        gait: Option<(Surface, f32)>,
        next_step: f64,
        foot: f64,
        position: DVec3,
        tour: Option<(f64, usize)>,
        master: f32,
        quit: bool,
    }

    impl World {
        fn command(&mut self, line: &str, engine: &mut AudioEngine, foley: &mut Foley, now: f64) {
            let words: Vec<&str> = line.split_whitespace().collect();
            let number = |i: usize| words.get(i).and_then(|w| w.parse::<f32>().ok());
            let surface = |i: usize| words.get(i).and_then(|w| Surface::from_name(w));
            match (words.first().copied(), number(1)) {
                (Some("time"), Some(h)) => self.state.hour = h.rem_euclid(24.0),
                (Some("lapse"), Some(h)) => self.lapse = h,
                (Some("temp"), Some(t)) => self.state.temperature = t,
                (Some("wind"), Some(v)) => self.state.wind = v.max(0.0),
                (Some("rain"), Some(r)) => self.state.rain = r.max(0.0),
                (Some("sea"), Some(d)) => {
                    self.state.shore = Some(Shore {
                        distance: d.max(0.0),
                        direction: Vec3::X,
                    })
                }
                (Some("sea"), None) => self.state.shore = None,
                (Some("walk" | "run"), _) => match surface(1) {
                    Some(s) => {
                        let speed = if words[0] == "run" { 4.0 } else { 1.4 };
                        self.gait = Some((s, speed));
                        self.next_step = now;
                    }
                    None => println!("which surface? stone, gravel, grass, wood, metal, water"),
                },
                (Some("stop"), _) => self.gait = None,
                (Some("drop"), _) => match (surface(1), number(2), number(3)) {
                    (Some(s), Some(mass), Some(speed)) => {
                        foley.impact(
                            engine,
                            &Impact {
                                surface: s,
                                position: self.position + DVec3::new(0.0, 0.0, -3.0),
                                speed,
                                mass,
                                start: None,
                            },
                        );
                    }
                    _ => println!("drop <surface> <kg> <m/s>"),
                },
                (Some("dive"), _) => self.state.medium = Medium::Water,
                (Some("up"), _) => self.state.medium = Medium::Air,
                (Some("volume"), Some(db)) => {
                    self.master = db_to_gain(db.min(6.0));
                    engine.control(Control::BusGain {
                        bus: BusId::MASTER,
                        gain: self.master,
                    });
                }
                (Some("tour"), _) => self.tour = Some((now, 0)),
                (Some("status"), _) => self.status(engine),
                (Some("help"), _) => println!("{HELP}"),
                (Some("quit" | "exit"), _) => self.quit = true,
                (None, _) => {}
                _ => println!("? {line} (help lists the commands)"),
            }
        }

        fn status(&self, engine: &AudioEngine) {
            let s = &self.state;
            let stats = engine.stats();
            println!(
                "{:02}:{:02}, {:.0} C, wind {:.1} m/s, rain {:.1} mm/h, sea {}, {}, {}",
                s.hour as u32,
                (s.hour.fract() * 60.0) as u32,
                s.temperature,
                s.wind,
                s.rain,
                s.shore
                    .map_or("off".to_owned(), |shore| format!("{:.0} m", shore.distance)),
                if s.medium == Medium::Water {
                    "under water"
                } else {
                    "in air"
                },
                self.gait.map_or("standing".to_owned(), |(surface, speed)| {
                    format!(
                        "{} on {}",
                        if speed > 2.0 { "running" } else { "walking" },
                        surface.name()
                    )
                }),
            );
            println!(
                "mixer: {} voices, blocks {:.0} us mean, {:.0} us longest, {} stream underruns, limiter {:.1} dB deep",
                stats.real_voices,
                stats.block_ns_mean as f64 / 1000.0,
                stats.block_ns_max as f64 / 1000.0,
                stats.underruns,
                stats.limiter_reduction_max_db,
            );
        }

        /// One tick of `dt` seconds at `now`.
        fn tick(&mut self, dt: f64, now: f64, engine: &mut AudioEngine, foley: &mut Foley) {
            self.state.hour = (self.state.hour + self.lapse * dt as f32 / 60.0).rem_euclid(24.0);
            if let Some((surface, speed)) = self.gait {
                self.position.z -= speed as f64 * dt;
                if now >= self.next_step {
                    self.foot = -self.foot;
                    foley.step(
                        engine,
                        &Step {
                            surface,
                            position: self.position + DVec3::new(self.foot, -1.6, -0.2),
                            speed,
                            weight: 80.0,
                            start: None,
                        },
                    );
                    self.next_step = now + if speed > 2.0 { 0.33 } else { 0.55 };
                }
            }
            self.state.listener = Listener::at(self.position);
            engine.set_listener(self.state.listener);
        }
    }

    /// Opens the device's stream of samples of type `T`: the callback copies out of the ring
    /// (stereo into the first two channels, the rest silent; a mono device gets the mean).
    fn open<T>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        mut ring: OutputRing,
        short: Arc<AtomicU64>,
    ) -> Result<cpal::Stream>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = config.channels as usize;
        // Sized here: the callback never allocates.
        let mut stereo = vec![0.0f32; 8192];
        let stream = device.build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for chunk in data.chunks_mut(channels * stereo.len() / 2) {
                    let frames = chunk.len() / channels;
                    let s = &mut stereo[..frames * 2];
                    if ring.pull(s) < frames {
                        short.fetch_add(1, Ordering::Relaxed);
                    }
                    for (frame, out) in s.as_chunks::<2>().0.iter().zip(chunk.chunks_mut(channels))
                    {
                        if channels == 1 {
                            out[0] = T::from_sample(0.5 * (frame[0] + frame[1]));
                            continue;
                        }
                        for (c, o) in out.iter_mut().enumerate() {
                            *o = if c < 2 {
                                T::from_sample(frame[c])
                            } else {
                                T::EQUILIBRIUM
                            };
                        }
                    }
                }
            },
            |e| eprintln!("the device: {e}"),
            None,
        )?;
        Ok(stream)
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();
        let host = cpal::default_host();
        if args.list {
            for device in host.output_devices()? {
                let config = device.default_output_config();
                println!("{device}: {config:?}");
            }
            return Ok(());
        }
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no output device"))?;
        let supported = device
            .default_output_config()
            .context("the device's default output format")?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate;
        println!(
            "{device}: {rate} Hz, {} channels, {format:?}; the ring adds {:.1} ms",
            config.channels,
            args.ring as f64 * 1000.0 / rate as f64
        );

        // The mix: the ambience's buses, and the walker's steps and the falls under the world.
        let mut layout = BusLayout::new();
        let buses = AmbienceBuses::add(&mut layout);
        let reverb = layout.add_reverb("reverb", buses.world, ReverbParams::OPEN);
        let foley_bus = layout.add("foley", buses.world);
        let (mut engine, mixer) = create(
            MixerConfig {
                sample_rate: rate,
                ..MixerConfig::default()
            },
            &layout,
        );
        let (opener, pump) = streams(0.25);
        let _decode = pump.spawn(Duration::from_millis(5));
        let mut world = World {
            state: AmbienceState {
                hour: 6.3,
                ..AmbienceState::default()
            },
            lapse: 0.0,
            gait: None,
            next_step: 0.0,
            foot: 0.1,
            position: DVec3::new(0.0, 1.6, 0.0),
            tour: None,
            master: 1.0,
            quit: false,
        };
        let mut ambience = Ambience::start(&mut engine, &opener, buses, 266, &world.state)?;
        let mut params = PlayParams {
            bus: foley_bus,
            ..PlayParams::default()
        };
        params.sends[0] = Some((reverb, 0.25));
        let mut foley = Foley::new(rate, 267, params);

        let (mix, ring) = MixThread::spawn(mixer, args.ring);
        let short = Arc::new(AtomicU64::new(0));
        let stream = match format {
            cpal::SampleFormat::F32 => open::<f32>(&device, config, ring, short.clone())?,
            cpal::SampleFormat::I16 => open::<i16>(&device, config, ring, short.clone())?,
            cpal::SampleFormat::I32 => open::<i32>(&device, config, ring, short.clone())?,
            other => bail!("the device wants {other:?} samples; the demo plays f32, i16 and i32"),
        };
        stream.play()?;

        let (lines, input) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        println!("{HELP}");
        let started = Instant::now();
        if args.tour {
            world.tour = Some((0.0, 0));
        }
        let tick = Duration::from_secs_f64(1.0 / 60.0);
        let mut next = Instant::now();
        let mut last = 0.0f64;
        while !world.quit {
            let now = started.elapsed().as_secs_f64();
            let dt = now - last;
            last = now;
            while let Ok(line) = input.try_recv() {
                world.command(&line, &mut engine, &mut foley, now);
            }
            if let Some((from, mut index)) = world.tour {
                while let Some(&(at, line)) = TOUR.get(index) {
                    if now - from < at {
                        break;
                    }
                    println!("[tour {at:5.1} s] {line}");
                    world.command(line, &mut engine, &mut foley, now);
                    index += 1;
                }
                world.tour = (index < TOUR.len()).then_some((from, index));
            }
            world.tick(dt, now, &mut engine, &mut foley);
            ambience.update(&mut engine, &world.state, dt as f32);
            engine.update();
            if args.seconds > 0.0 && now >= args.seconds {
                break;
            }
            next += tick;
            if let Some(wait) = next.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            } else {
                next = Instant::now();
            }
        }
        drop(stream);
        let _mixer = mix.stop();
        world.status(&engine);
        println!(
            "the device found the ring short {} times",
            short.load(Ordering::Relaxed)
        );
        Ok(())
    }
}
