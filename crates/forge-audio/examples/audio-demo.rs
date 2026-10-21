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
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use anyhow::Result;
    use clap::Parser;
    use cpal::traits::{DeviceTrait, HostTrait};
    use forge_audio::ambience::{
        Ambience, AmbienceAssets, AmbienceBuses, AmbienceState, BedKind, Medium, Shore,
    };
    use forge_audio::device::Device;
    use forge_audio::dsp::ReverbParams;
    use forge_audio::stream::streams;
    use forge_audio::{
        AudioEngine, BusId, BusLayout, Control, Foley, Footwear, Impact, Listener, MixerConfig,
        PlayParams, Step, Surface, create, db_to_gain,
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
        /// Places the sounds through Steam Audio's HRTF (headphones; needs the `steam-audio`
        /// feature and the SDK in steam-audio-sdk/).
        #[arg(long)]
        hrtf: bool,
    }

    const HELP: &str = "\
commands (one a line); a line `now:` names each sound as it comes or goes:
  time <h>               the hour, 0 to 24: birds sing at dawn (about 5 to 8), call a little
                         through the day, again at dusk; cicadas sing from mid-morning to sunset
  lapse <h>              hours that pass a minute (0: the clock stands)
  temp <C>               the temperature: cicadas sing only when it is warm (above 22 C)
  wind <m/s>             the wind (cicadas give way to it from 5 m/s)
  rain <mm/h>            the rain (cicadas stop at once, and wait until the place dries)
  sea <m> | sea off      the distance to the quay, on the right
  walk <surface>         walk (stone, gravel, grass, wood, metal, water)
  run <surface>          run
  shoes <kind>           barefoot, sneakers, leather, boots, sandals
  stop                   stand still
  drop <surface> <kg> <m/s>   something falls 3 m ahead
  dive | up              the head under water, or out
  volume <dB>            the master's level
  tour                   the tour (about 160 s)
  status                 the state and the mixer's counters
  help | quit";

    /// The tour: commands at their second.
    const TOUR: &[(f64, &str)] = &[
        (0.0, "time 3"),
        (0.0, "temp 26"),
        (0.0, "lapse 0"),
        (4.0, "time 6.3"),
        (16.0, "shoes leather"),
        (16.0, "walk stone"),
        (20.0, "walk gravel"),
        (24.0, "walk grass"),
        (28.0, "walk wood"),
        (32.0, "walk metal"),
        (36.0, "walk stone"),
        (36.0, "shoes sneakers"),
        (40.0, "shoes boots"),
        (44.0, "shoes barefoot"),
        (48.0, "shoes sandals"),
        (52.0, "shoes leather"),
        (52.0, "run stone"),
        (56.0, "stop"),
        (57.0, "time 13"),
        (57.0, "temp 31"),
        (69.0, "wind 9"),
        (77.0, "rain 6"),
        (89.0, "rain 0"),
        (89.0, "wind 3"),
        (93.0, "sea 60"),
        (99.0, "sea 20"),
        (105.0, "sea 4"),
        (111.0, "walk water"),
        (116.0, "stop"),
        (118.0, "dive"),
        (128.0, "up"),
        (132.0, "drop wood 20 3"),
        (134.0, "drop metal 1 5"),
        (136.0, "drop water 30 4"),
        (139.0, "time 20.3"),
        (139.0, "temp 24"),
        (153.0, "time 22.5"),
        (161.0, "sea off"),
    ];

    /// The world the sounds come from, as the terminal sets it.
    struct World {
        state: AmbienceState,
        lapse: f32,
        gait: Option<(Surface, f32)>,
        footwear: Footwear,
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
                (Some("shoes"), _) => match words.get(1).and_then(|w| Footwear::from_name(w)) {
                    Some(f) => self.footwear = f,
                    None => println!("which shoes? barefoot, sneakers, leather, boots, sandals"),
                },
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
                        "{} on {} in {}",
                        if speed > 2.0 { "running" } else { "walking" },
                        surface.name(),
                        self.footwear.name()
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
                    // The walker's own feet: without a place (straight below the head an HRTF
                    // has nothing measured), a little to the side of the foot.
                    foley.own_step(
                        engine,
                        &Step {
                            surface,
                            position: self.position + DVec3::new(self.foot, -1.6, -0.2),
                            speed,
                            weight: 80.0,
                            footwear: self.footwear,
                            start: None,
                        },
                        self.foot.signum() as f32,
                    );
                    self.next_step = now + if speed > 2.0 { 0.33 } else { 0.55 };
                }
            }
            self.state.listener = Listener::at(self.position);
            engine.set_listener(self.state.listener);
        }
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
        let device = Device::default_output()?;
        let rate = device.sample_rate();
        println!(
            "{}: {rate} Hz, {} channels; the ring adds {:.1} ms",
            device.name(),
            device.channels(),
            args.ring as f64 * 1000.0 / rate as f64
        );

        // The mix: the ambience's buses, and the walker's steps and the falls under the world.
        let mut layout = BusLayout::new();
        let buses = AmbienceBuses::add(&mut layout);
        let reverb = layout.add_reverb("reverb", buses.world, ReverbParams::OPEN);
        let foley_bus = layout.add("foley", buses.world);
        let mix_config = MixerConfig {
            sample_rate: rate,
            ..MixerConfig::default()
        };
        #[allow(unused_mut)]
        let (mut engine, mut mixer) = create(mix_config, &layout);
        if args.hrtf {
            #[cfg(feature = "steam-audio")]
            {
                use anyhow::Context as _;
                let steam = forge_audio::steam::SteamSpatializer::for_mixer(&mix_config)
                    .context("Steam Audio (steam-audio-sdk/, docs/demos/audio.md)")?;
                mixer.set_spatializer(Box::new(steam));
                println!("placing the sounds through Steam Audio's HRTF: listen on headphones");
            }
            #[cfg(not(feature = "steam-audio"))]
            println!("--hrtf needs the steam-audio feature; panning instead");
        }
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
            footwear: Footwear::Sneakers,
            position: DVec3::new(0.0, 1.6, 0.0),
            tour: None,
            master: 1.0,
            quit: false,
        };
        // The recorded birds when tools/fetch-sounds.sh has fetched them; birds made in code
        // otherwise.
        let assets = AmbienceAssets::load("assets/sounds", rate).unwrap_or_default();
        println!(
            "birds: {}; cicadas: {}",
            if assets.birds.is_some() {
                "recorded"
            } else {
                "made in code"
            },
            if assets.cicadas.is_some() {
                "recorded"
            } else {
                "made in code (tools/fetch-sounds.sh fetches the recordings)"
            }
        );
        let mut ambience =
            Ambience::start_with(&mut engine, &opener, buses, 266, &world.state, &assets)?;
        let mut params = PlayParams {
            bus: foley_bus,
            ..PlayParams::default()
        };
        params.sends[0] = Some((reverb, 0.25));
        let mut foley = Foley::new(rate, 267, params);

        let output = device.play(mixer, args.ring)?;

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
        let mut heard = [false; 6];
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
            // Name each bed as it comes or goes.
            for (i, kind) in BedKind::ALL.into_iter().enumerate() {
                let on = ambience.gain(kind) > 0.05;
                if on != heard[i] {
                    heard[i] = on;
                    println!(
                        "now: {} {}",
                        kind.name(),
                        if on { "starts" } else { "stops" }
                    );
                }
            }
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
        let short = output.short_pulls();
        drop(output);
        world.status(&engine);
        println!("the device found the ring short {short} times");
        Ok(())
    }
}
