//! Ambience: beds by the hour, the weather and the medium, measured on rendered samples.

mod common;

use common::*;
use forge_audio::ambience::{
    Ambience, AmbienceBuses, AmbienceState, Bed, BedKind, Medium, Shore, targets,
};
use forge_audio::decode::Decoder;
use forge_audio::dsp::{BusFilter, ReverbParams};
use forge_audio::sound::{impulse, sine_loop};
use forge_audio::stream::streams;
use forge_audio::{
    AudioEngine, BusLayout, MixerConfig, OfflineRenderer, PlayParams, create, db_to_gain,
};
use glam::Vec3;

const RATE: u32 = 48_000;
/// A tick of the simulation: 60 a second.
const TICK: usize = 800;

/// A bed alone, straight from its decoder.
fn bed(kind: BedKind, intensity: f32, seconds: f64, seed: u64) -> Vec<f32> {
    let (mut bed, _control) = Bed::new(kind, seed, RATE, intensity);
    let mut out = vec![0.0f32; (seconds * RATE as f64) as usize * 2];
    bed.read(&mut out).unwrap();
    out
}

/// The ambience's buses in a mixer, its beds streamed and pumped before each block.
struct Scene {
    engine: AudioEngine,
    renderer: OfflineRenderer,
    ambience: Ambience,
    buses: AmbienceBuses,
}

impl Scene {
    fn new(state: &AmbienceState, seed: u64) -> Self {
        let mut layout = BusLayout::new();
        let buses = AmbienceBuses::add(&mut layout);
        let (mut engine, mixer) = create(MixerConfig::default(), &layout);
        let (opener, pump) = streams(0.25);
        let ambience = Ambience::start(&mut engine, &opener, buses, seed, state).unwrap();
        Self {
            engine,
            renderer: OfflineRenderer::with_streams(mixer, pump),
            ambience,
            buses,
        }
    }

    /// Renders `seconds`, a tick at a time, the state at each tick from `state`.
    fn run(&mut self, seconds: f64, mut state: impl FnMut(f64) -> AmbienceState) -> Vec<f32> {
        let ticks = (seconds * RATE as f64 / TICK as f64).round() as usize;
        let mut out = Vec::with_capacity(ticks * TICK * 2);
        for k in 0..ticks {
            let t = (k * TICK) as f64 / RATE as f64;
            self.ambience
                .update(&mut self.engine, &state(t), TICK as f32 / RATE as f32);
            out.extend(self.renderer.render(TICK));
            self.engine.update();
        }
        out
    }
}

/// `seconds` of a still state after `settle` seconds, left channel.
fn steady(state: AmbienceState, settle: f64, seconds: f64, seed: u64) -> Vec<f32> {
    let mut scene = Scene::new(&state, seed);
    scene.run(settle, |_| state);
    channel(&scene.run(seconds, |_| state), 0)
}

#[test]
fn cicadas_sing_in_the_heat_of_the_day() {
    let at = |hour, temperature| AmbienceState {
        hour,
        temperature,
        ..AmbienceState::default()
    };
    // Noon at 30 °C against noon at 18 °C: the same birds, cicadas only in the heat.
    let hot = steady(at(13.0, 30.0), 2.0, 6.0, 1);
    let cool = steady(at(13.0, 18.0), 2.0, 6.0, 1);
    let band = |x: &[f32]| band_db(x, 3000.0, 8000.0, RATE);
    // The full chorus near −33 dBFS since the owner found −24 too loud for a game (#357).
    assert!(band(&hot) > -38.0, "{:.1} dBFS", band(&hot));
    assert!(
        band(&hot) - band(&cool) > 10.0,
        "{:.1} against {:.1}",
        band(&hot),
        band(&cool)
    );
    // At 3:00 and 21:30 on a calm dry night nothing plays at all.
    for hour in [3.0, 21.5] {
        let night = steady(at(hour, 30.0), 1.0, 2.0, 1);
        assert!(peak(&night) < 1e-5, "{hour}: {}", peak(&night));
    }

    // One cicada alone sings its echemes: the envelope repeats every 130 to 450 ms
    // (Cicada orni's echeme period, 230 ± 70 ms), its carrier near 4.8 kHz.
    let lone = channel(&bed(BedKind::Cicadas, 0.03, 12.0, 4), 0);
    let lone = &lone[2 * RATE as usize..];
    let bands = octaves(lone, RATE);
    let loudest = (0..9)
        .max_by(|&i, &j| bands[i].total_cmp(&bands[j]))
        .unwrap();
    assert_eq!(OCTAVES[loudest], 4000.0);
    let env = envelope(lone, RATE as usize / 200);
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    let centred: Vec<f64> = env.iter().map(|e| e - mean).collect();
    let auto = |lag: usize| -> f64 {
        centred
            .iter()
            .zip(&centred[lag..])
            .map(|(a, b)| a * b)
            .sum()
    };
    // The first peak of the autocorrelation after it has crossed zero.
    let lags: Vec<f64> = (0..100).map(auto).collect();
    let cross = lags.iter().position(|&a| a < 0.0).unwrap();
    let period = (cross..100)
        .max_by(|&i, &j| lags[i].total_cmp(&lags[j]))
        .unwrap();
    let period = period as f64 * 0.005;
    assert!((0.13..=0.45).contains(&period), "{period} s");
}

#[test]
fn birds_sing_the_dawn_chorus() {
    let at = |hour| AmbienceState {
        hour,
        temperature: 18.0,
        ..AmbienceState::default()
    };
    let band = |hour| band_db(&steady(at(hour), 3.0, 20.0, 2), 1000.0, 8000.0, RATE);
    let dawn = band(6.5);
    let dusk = band(20.3);
    let day = band(13.0);
    // The dawn chorus (5 calls a second) above the dusk (2) above the day (0.75).
    assert!(dawn > dusk + 2.0, "dawn {dawn:.1}, dusk {dusk:.1}");
    assert!(dusk > day + 2.0, "dusk {dusk:.1}, day {day:.1}");
    assert!((-36.0..-24.0).contains(&dawn), "dawn {dawn:.1} dBFS");
    // Rain and wind silence them (the rules: 5 mm/h, or 12 m/s).
    let birds = |state: AmbienceState| targets(&state)[3].intensity;
    assert_eq!(birds(at(6.5)), 1.0);
    assert_eq!(
        birds(AmbienceState {
            rain: 5.0,
            ..at(6.5)
        }),
        0.0
    );
    assert_eq!(
        birds(AmbienceState {
            wind: 12.0,
            ..at(6.5)
        }),
        0.0
    );
    assert_eq!(birds(at(2.0)), 0.0);
}

#[test]
fn wind_and_rain_follow_the_weather() {
    let wind: Vec<(f64, f64, Vec<f32>)> = [3.0, 8.0, 15.0]
        .map(|v| {
            let x = bed(BedKind::Wind, v, 10.0, 5);
            let l = channel(&x, 0);
            (db(rms(&l)), centroid(&l, RATE), x)
        })
        .into_iter()
        .collect();
    // Louder and brighter with speed.
    assert!(
        wind[1].0 - wind[0].0 > 8.0,
        "{:.1} {:.1}",
        wind[0].0,
        wind[1].0
    );
    assert!(
        wind[2].0 - wind[1].0 > 5.0,
        "{:.1} {:.1}",
        wind[1].0,
        wind[2].0
    );
    assert!(wind[0].1 < wind[1].1 && wind[1].1 < wind[2].1);
    for (_, _, x) in &wind {
        let (l, r) = (channel(x, 0), channel(x, 1));
        // Gusts: the level in 100 ms windows wanders by more than 2 dB.
        let levels: Vec<f64> = envelope(&l, 4800).iter().map(|&e| db(e)).collect();
        let mean = levels.iter().sum::<f64>() / levels.len() as f64;
        let spread =
            (levels.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / levels.len() as f64).sqrt();
        assert!(spread > 2.0, "{spread:.2} dB");
        // Each ear its own air.
        assert!(correlation(&l, &r).abs() < 0.2);
    }
    // Rain: 40 drops a second each mm/h, so ten times the rate is +10 dB, steady.
    let light = channel(&bed(BedKind::Rain, 2.0, 10.0, 6), 0);
    let heavy = channel(&bed(BedKind::Rain, 20.0, 10.0, 6), 0);
    let gain = db(rms(&heavy) / rms(&light));
    assert!((gain - 10.0).abs() < 1.0, "{gain:.2} dB");
}

#[test]
fn the_sea_is_heard_from_the_quays() {
    let at = |distance| AmbienceState {
        hour: 2.0,
        shore: Some(Shore {
            distance,
            direction: Vec3::X,
        }),
        ..AmbienceState::default()
    };
    let level = |distance| {
        let mut scene = Scene::new(&at(distance), 3);
        let out = scene.run(6.0, |_| at(distance));
        (rms(&channel(&out, 0)), rms(&channel(&out, 1)))
    };
    let (l4, r4) = level(4.0);
    let (_, r16) = level(16.0);
    let (_, r64) = level(64.0);
    // A line source: −3 dB a doubling of the distance, −6.02 dB every four times.
    assert!((db(r4 / r16) - 6.02).abs() < 0.05, "{}", db(r4 / r16));
    assert!((db(r16 / r64) - 6.02).abs() < 0.05);
    assert!(
        (-28.0..-20.0).contains(&db(r4)),
        "{:.1} dBFS at 4 m",
        db(r4)
    );
    // The water to the right: the right channel louder.
    assert!(db(r4 / l4) > 6.0, "{:.1} dB", db(r4 / l4));
    // Out of earshot past 400 m.
    let (_, far) = level(450.0);
    assert!(far < 1e-6);
}

#[test]
fn underwater_muffles_the_world() {
    // A calm night with a light wind; a 400 Hz tone and a 3 kHz tone hard left in the
    // world's bus, then the head goes under at 3 s.
    let air = AmbienceState {
        hour: 2.0,
        wind: 2.0,
        ..AmbienceState::default()
    };
    let water = AmbienceState {
        medium: Medium::Water,
        ..air
    };
    let mut scene = Scene::new(&air, 7);
    let low = sine_loop(400.0, 0.3, 1.0, RATE).shared();
    let high = sine_loop(3000.0, 0.3, 1.0, RATE).shared();
    for tone in [&low, &high] {
        scene.engine.play(
            tone,
            PlayParams {
                bus: scene.buses.world,
                looping: true,
                pan: -1.0,
                ..PlayParams::default()
            },
        );
    }
    let out = scene.run(6.0, |t| if t < 3.0 { air } else { water });
    let (left, right) = (channel(&out, 0), channel(&out, 1));
    let second = |x: &[f32], from: f64| {
        let a = (from * RATE as f64) as usize;
        x[a..a + RATE as usize].to_vec()
    };
    let (before, after) = (second(&left, 1.5), second(&left, 4.5));
    // 400 Hz passes the 600 Hz low-pass (0.17 dB down): 29 dB for the surface, and 4.4 dB
    // as the width falls to a fifth (a tone hard left keeps (1 + 0.2) / 2 of itself there).
    let drop_low = db(tone(&before, 400.0, RATE) / tone(&after, 400.0, RATE));
    assert!((drop_low - 33.6).abs() < 0.3, "{drop_low:.2} dB");
    // 3 kHz is gone: the 29 dB and 2.3 octaves of a fourth-order low-pass.
    let drop_high = db(tone(&before, 3000.0, RATE) / tone(&after, 3000.0, RATE));
    assert!(drop_high > 70.0, "{drop_high:.1} dB");
    // A fifth of the width: the tone hard left reaches the right ear (1 − 0.2) / (1 + 0.2).
    let width = tone(&second(&right, 4.5), 400.0, RATE) / tone(&after, 400.0, RATE);
    assert!((width - 0.8 / 1.2).abs() < 0.015, "{width:.3}");
    assert!(tone(&second(&right, 1.5), 400.0, RATE) < 0.01 * tone(&before, 400.0, RATE));
    // The underwater rumble comes up below 160 Hz.
    let rumble = band_db(&after, 30.0, 150.0, RATE) - band_db(&before, 30.0, 150.0, RATE);
    assert!(rumble > 15.0, "{rumble:.1} dB");
    // Within 150 ms of going under, the 3 kHz tone is 40 dB down.
    let switch = 3 * RATE as usize;
    let at_150 = &left[switch + 7200..switch + 7200 + 480];
    let before_window = &left[switch - 960..switch - 480];
    let fall = db(tone(before_window, 3000.0, RATE) / tone(at_150, 3000.0, RATE));
    assert!(fall > 40.0, "{fall:.1} dB after 150 ms");
    // No click going under or coming back up.
    let worst = worst_window_above_db(&left, 16_000.0, RATE);
    assert!(worst < -25.0, "{worst:.1} dB");
}

#[test]
fn the_water_has_its_own_reverb() {
    let state = AmbienceState {
        hour: 2.0,
        ..AmbienceState::default()
    };
    let mut scene = Scene::new(&state, 8);
    let click = impulse(1.0, RATE).shared();
    scene.engine.play(
        &click,
        PlayParams {
            bus: scene.buses.water,
            start: Some(4800),
            ..PlayParams::default()
        },
    );
    let out = channel(&scene.run(3.0, |_| state), 0);
    // The tail from 2 ms after the click: Schroeder's backward integral, fitted between −5
    // and −35 dB.
    let tail = &out[4800 + 63 + 96..];
    let mut energy: Vec<f64> = tail.iter().map(|&x| x as f64 * x as f64).collect();
    for i in (0..energy.len() - 1).rev() {
        energy[i] += energy[i + 1];
    }
    let level = |i: usize| 10.0 * (energy[i] / energy[0]).log10();
    let t5 = (0..energy.len()).find(|&i| level(i) <= -5.0).unwrap();
    let t35 = (0..energy.len()).find(|&i| level(i) <= -35.0).unwrap();
    let rt60 = 2.0 * (t35 - t5) as f64 / RATE as f64;
    let expected = ReverbParams::UNDERWATER.rt60 as f64;
    assert!((rt60 / expected - 1.0).abs() < 0.15, "{rt60:.2} s");
}

#[test]
fn a_day_in_time_lapse_crossfades_without_clicks() {
    // Thirty seconds for a morning: the hour from 4:00 to 14:00, a wind rising to 12 m/s and
    // falling, a dive from 19 to 23 s, the walk down to the quay, and a shower from 24 s that
    // silences the cicadas (it falls once their hours have begun: the wet place keeps them
    // quiet after).
    let state = |t: f64| {
        let t = t as f32;
        let bump = |from: f32, to: f32| {
            let u = ((t - from) / (to - from)).clamp(0.0, 1.0);
            (std::f32::consts::PI * u).sin()
        };
        AmbienceState {
            hour: 4.0 + t / 3.0,
            wind: 2.0 + 10.0 * bump(5.0, 15.0),
            rain: 8.0 * bump(24.0, 30.0),
            shore: Some(Shore {
                distance: 100.0 - 95.0 * (t / 18.0).min(1.0),
                direction: Vec3::NEG_Z,
            }),
            medium: if (19.0..23.0).contains(&t) {
                Medium::Water
            } else {
                Medium::Air
            },
            ..AmbienceState::default()
        }
    };
    let mut scene = Scene::new(&state(0.0), 9);
    let mut gains = Vec::new();
    let mut out = Vec::new();
    for k in 0..30 {
        out.extend(scene.run(1.0, |t| state(k as f64 + t)));
        gains.push(BedKind::ALL.map(|b| scene.ambience.gain(b)));
    }
    let left = channel(&out, 0);
    // Every bed had its turn.
    for (i, kind) in BedKind::ALL.iter().enumerate() {
        let most = gains.iter().map(|g| g[i]).fold(0.0f32, f32::max);
        assert!(most > 0.5, "{} never rose: {most}", kind.name());
    }
    let worst = worst_window_above_db(&left, 16_000.0, RATE);
    assert!(worst < -25.0, "{worst:.1} dB");
    assert!(peak(&left) < db_to_gain(-1.0) + 1e-3);

    // The same day again: the same samples.
    let mut again = Scene::new(&state(0.0), 9);
    let mut repeat = Vec::new();
    for k in 0..30 {
        repeat.extend(again.run(1.0, |t| state(k as f64 + t)));
    }
    assert_eq!(out, repeat);
}

#[test]
fn a_quiet_bed_sleeps_and_wakes() {
    let wet = AmbienceState {
        hour: 2.0,
        rain: 5.0,
        ..AmbienceState::default()
    };
    let dry = AmbienceState { rain: 0.0, ..wet };
    let mut scene = Scene::new(&wet, 10);
    scene.run(2.0, |_| wet);
    assert!(scene.ambience.awake(BedKind::Rain));
    // The rain stops: its gain falls (2 s time constant) and the bed sleeps.
    let out = scene.run(30.0, |_| dry);
    assert!(!scene.ambience.awake(BedKind::Rain));
    assert!(peak(&channel(&out, 0)[RATE as usize * 28..]) < 1e-6);
    // It starts again: the bed wakes and fades in without a click.
    let out = scene.run(4.0, |_| wet);
    assert!(scene.ambience.awake(BedKind::Rain));
    let left = channel(&out, 0);
    assert!(rms(&left[RATE as usize * 3..]) > 1e-3);
    let worst = worst_window_above_db(&left, 16_000.0, RATE);
    assert!(worst < -25.0, "{worst:.1} dB");
    // The filter the medium sets is the world's.
    assert_eq!(BusFilter::UNDERWATER.width, 0.2);
}

/// The recordings `tools/fetch-sounds.sh` fetches, if they are there.
fn recordings() -> Option<forge_audio::ambience::AmbienceAssets> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds");
    let assets = forge_audio::ambience::AmbienceAssets::load(dir, RATE).expect("they decode");
    if assets.birds.is_none() {
        eprintln!("no recordings in assets/sounds/ (tools/fetch-sounds.sh): skipped");
        return None;
    }
    Some(assets)
}

#[test]
fn recorded_birds_are_cut_into_calls() {
    use forge_audio::ambience::Species;
    let Some(assets) = recordings() else {
        return;
    };
    let birds = assets.birds.expect("birds");
    for (species, least, band) in [
        (Species::Blackbird, 15, 1500.0..4500.0),
        (Species::Sparrow, 5, 3000.0..4500.0),
        (Species::Serin, 10, 3500.0..8000.0),
    ] {
        let calls: Vec<_> = birds.of(species).collect();
        assert!(calls.len() >= least, "{species:?}: {} calls", calls.len());
        for call in calls {
            let x = &call.samples;
            let seconds = x.len() as f64 / RATE as f64;
            assert!(
                (0.06..=3.05).contains(&seconds),
                "{species:?}: {seconds:.2} s"
            );
            assert_eq!((x[0], x[x.len() - 1]), (0.0, 0.0));
            assert!((peak(x) - 0.5).abs() < 1e-6);
            let c = centroid(x, RATE);
            assert!(band.contains(&c), "{species:?}: centroid {c:.0} Hz");
        }
    }
    let chorus = birds.chorus.as_ref().expect("the morning chorus");
    assert!(chorus.len() / 2 > 170 * RATE as usize);
    // Without the files, no recordings and no error: the birds are made in code.
    let empty = std::env::temp_dir().join("forge-audio-no-recordings");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(
        forge_audio::ambience::AmbienceAssets::load(&empty, RATE)
            .unwrap()
            .birds
            .is_none()
    );
}

#[test]
fn recorded_birds_sing_the_dawn() {
    let Some(assets) = recordings() else {
        return;
    };
    let render = |activity: f32, seed: u64| {
        let (mut bed, _control) = Bed::new_with(BedKind::Birds, seed, RATE, activity, &assets);
        let mut out = vec![0.0f32; RATE as usize * 2 * 20];
        bed.read(&mut out).unwrap();
        out
    };
    let dawn = channel(&render(1.0, 5), 0);
    let day = channel(&render(0.15, 5), 0);
    let (d, n) = (
        band_db(&dawn, 1000.0, 8000.0, RATE),
        band_db(&day, 1000.0, 8000.0, RATE),
    );
    // The dawn chorus, with the morning chorus far behind it, above the day's few calls.
    assert!(d > n + 6.0, "dawn {d:.1}, day {n:.1} dBFS");
    assert!((-28.0..-18.0).contains(&d), "dawn {d:.1} dBFS");
    for x in [&dawn, &day] {
        let worst = worst_window_above_db(x, 16_000.0, RATE);
        assert!(worst < -25.0, "{worst:.1} dB");
    }
    assert_eq!(render(1.0, 5), render(1.0, 5));
}

#[test]
fn cicadas_stop_in_rain_and_stay_quiet_while_it_is_wet() {
    let noon = AmbienceState {
        hour: 13.0,
        temperature: 30.0,
        ..AmbienceState::default()
    };
    let rain = AmbienceState { rain: 6.0, ..noon };
    let mut scene = Scene::new(&noon, 11);
    scene.run(8.0, |_| noon);
    assert!(scene.ambience.gain(BedKind::Cicadas) > 0.9);
    // Two seconds into the rain they have stopped (the gain falls over 0.8 s).
    scene.run(2.0, |_| rain);
    let stopped = scene.ambience.gain(BedKind::Cicadas);
    assert!(stopped < 0.1, "{stopped:.3}");
    scene.run(8.0, |_| rain);
    // A minute after the rain the place is still wet: still quiet.
    scene.run(60.0, |_| noon);
    assert!(scene.ambience.wet() > 0.5, "{:.2}", scene.ambience.wet());
    assert!(scene.ambience.gain(BedKind::Cicadas) < 0.01);
    // Five more minutes and it has dried: they sing again.
    scene.run(300.0, |_| noon);
    assert!(scene.ambience.gain(BedKind::Cicadas) > 0.9);
    // Wind: they give way from 5 m/s and are silent by 10.
    let cicadas = |wind: f32| targets(&AmbienceState { wind, ..noon })[2].intensity;
    assert_eq!(cicadas(4.0), 1.0);
    assert!(cicadas(9.0) < 0.15, "{}", cicadas(9.0));
    assert_eq!(cicadas(10.0), 0.0);
}

#[test]
fn recorded_cicadas_sing_as_a_chorus() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sounds");
    let assets = forge_audio::ambience::AmbienceAssets::load(dir, RATE).expect("it decodes");
    let Some(song) = assets.cicadas.clone() else {
        eprintln!("no cicada recording in assets/sounds/ (tools/fetch-sounds.sh): skipped");
        return;
    };
    // Cicada orni's song: over a minute of it, centred near its measured peak (4.82 kHz).
    assert!(song.samples.len() > 60 * RATE as usize);
    let c = centroid(&song.samples, RATE);
    assert!((4000.0..6500.0).contains(&c), "{c:.0} Hz");
    let render = |activity: f32| {
        let (mut bed, _control) = Bed::new_with(BedKind::Cicadas, 5, RATE, activity, &assets);
        let mut out = vec![0.0f32; RATE as usize * 2 * 12];
        bed.read(&mut out).unwrap();
        // After the singers' first second of coming in.
        out[RATE as usize * 4..].to_vec()
    };
    let one = render(0.05);
    let all = render(1.0);
    let (l1, r1) = (channel(&one, 0), channel(&one, 1));
    let (la, ra) = (channel(&all, 0), channel(&all, 1));
    // One singer's echemes repeat within the species' range (130 to 450 ms).
    let env = envelope(&l1, RATE as usize / 200);
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    let centred: Vec<f64> = env.iter().map(|e| e - mean).collect();
    let lags: Vec<f64> = (0..100)
        .map(|k| centred.iter().zip(&centred[k..]).map(|(a, b)| a * b).sum())
        .collect();
    let cross = lags.iter().position(|&a| a < 0.0).unwrap();
    let period = (cross..100)
        .max_by(|&i, &j| lags[i].total_cmp(&lags[j]))
        .unwrap() as f64
        * 0.005;
    assert!((0.13..=0.45).contains(&period), "{period} s");
    // The full chorus: louder than one singer by over 4 dB, and spread between the ears.
    assert!(
        db(rms(&la) / rms(&l1)) > 4.0,
        "{:.1} dB",
        db(rms(&la) / rms(&l1))
    );
    assert!(correlation(&la, &ra) < correlation(&l1, &r1));
    for x in [&l1, &la] {
        let worst = worst_window_above_db(x, 16_000.0, RATE);
        assert!(worst < -25.0, "{worst:.1} dB");
    }
    assert_eq!(render(1.0), all);
}

#[test]
fn the_habitat_quiets_cicadas_among_houses_and_the_outside_indoors() {
    use forge_audio::ambience::Habitat;
    // Noon in the heat: the full cicada chorus and the day's birds, a breeze, the sea near.
    let noon = AmbienceState {
        hour: 13.0,
        temperature: 30.0,
        wind: 3.0,
        shore: Some(Shore {
            distance: 10.0,
            direction: Vec3::X,
        }),
        ..AmbienceState::default()
    };
    let open = targets(&noon);
    let at = |green: f32, indoors: f32| {
        targets(&AmbienceState {
            habitat: Habitat {
                green,
                indoors,
                ..Habitat::default()
            },
            ..noon
        })
    };
    // The default habitat is the open country the beds were made for: nothing changes.
    assert_eq!(at(1.0, 0.0), open);
    // Among houses (no green) the cicadas are silent and a third of the birds call.
    let streets = at(0.0, 0.0);
    assert_eq!(streets[2].intensity, 0.0);
    assert_eq!(streets[2].gain, 0.0);
    assert!((streets[3].intensity - 0.35 * open[3].intensity).abs() < 1e-6);
    // A street with a few trees: no cicadas (only in the open, #357); the birds in between.
    let trees = at(0.25, 0.0);
    assert_eq!(trees[2].intensity, 0.0);
    assert!(trees[3].intensity > streets[3].intensity && trees[3].intensity < open[3].intensity);
    // The open country's edge, a house or two within earshot: some cicadas, fewer than in the
    // open.
    let edge = at(0.75, 0.0);
    assert!(edge[2].intensity > 0.0 && edge[2].intensity < 0.7 * open[2].intensity);
    // The wind and the sea do not care how green the place is.
    assert_eq!(streets[0], open[0]);
    assert_eq!(streets[4], open[4]);
    // Well inside a building every bed of the outside is 16 dB down, the intensities kept.
    let inside = at(1.0, 1.0);
    for i in 0..5 {
        assert!(
            (inside[i].gain - 0.15 * open[i].gain).abs() < 1e-6,
            "bed {i}"
        );
        assert_eq!(inside[i].intensity, open[i].intensity);
    }
    assert_eq!(inside[5], open[5]);
}

#[test]
fn the_bird_mix_picks_each_voice_by_its_share() {
    use forge_audio::ambience::{BirdMix, BirdVoice};
    let count = |mix: BirdMix, voice: BirdVoice| {
        (0..10_000)
            .filter(|&i| mix.pick((i as f32 + 0.5) / 10_000.0) == voice)
            .count()
    };
    // The default: the dove one call in eighteen (0.05 of 0.9), no longer three in twenty.
    let default = BirdMix::default();
    let doves = count(default, BirdVoice::Dove);
    assert!((540..=572).contains(&doves), "{doves}");
    assert_eq!(count(default, BirdVoice::Blackbird), 3_333);
    // A species at 0 never calls; a mix of nothing sings the blackbird.
    let no_doves = BirdMix {
        dove: 0.0,
        ..default
    };
    assert_eq!(count(no_doves, BirdVoice::Dove), 0);
    let none = BirdMix {
        blackbird: 0.0,
        sparrow: 0.0,
        serin: 0.0,
        dove: 0.0,
    };
    assert_eq!(count(none, BirdVoice::Blackbird), 10_000);
    // The last share takes the top of the range: u just under 1 is the dove's.
    assert_eq!(default.pick(0.999_99), BirdVoice::Dove);
}

#[test]
fn the_dove_calls_only_near_woods() {
    use forge_audio::ambience::Habitat;
    let doves = |green: f32| {
        Habitat {
            green,
            ..Habitat::default()
        }
        .doves()
    };
    // None in a street with a few trees (the town measures 0.05 to 0.09), all of its share in a
    // wood and in the open country the beds were made for, some at a wood's edge.
    assert_eq!(doves(0.0), 0.0);
    assert_eq!(doves(0.1), 0.0);
    assert_eq!(doves(0.5), 1.0);
    assert_eq!(doves(1.0), 1.0);
    let edge = doves(0.35);
    assert!(edge > 0.3 && edge < 1.0, "{edge}");
}

#[test]
fn the_ground_s_singers_are_not_heard_from_the_air() {
    use forge_audio::ambience::Habitat;
    // Noon in the heat over open country: the cicadas and the birds by the listener's height.
    let noon = AmbienceState {
        hour: 13.0,
        temperature: 30.0,
        wind: 3.0,
        ..AmbienceState::default()
    };
    let at = |height: f32| {
        targets(&AmbienceState {
            habitat: Habitat {
                height,
                ..Habitat::default()
            },
            ..noon
        })
    };
    let ground = at(0.0);
    // Among the crowns, nothing changes; over the roofs, less; from the air (the owner's start,
    // 100 m and more up, #357), nothing.
    assert_eq!(at(12.0), ground);
    let roofs = at(40.0);
    assert!(roofs[2].intensity > 0.0 && roofs[2].intensity < 0.8 * ground[2].intensity);
    assert!(roofs[3].intensity > 0.0 && roofs[3].intensity < 0.8 * ground[3].intensity);
    let air = at(100.0);
    assert_eq!(air[2].gain, 0.0);
    assert_eq!(air[3].gain, 0.0);
    // The wind blows as hard up there.
    assert_eq!(air[0], ground[0]);
}

#[test]
fn only_the_world_s_things_sound_when_asked() {
    // Noon in the heat by the sea: the cicadas, the birds and the sea; with only the sounds of
    // things in the world (#357), the cicadas and the birds' bed fall silent and the sea stays.
    let noon = AmbienceState {
        hour: 13.0,
        temperature: 30.0,
        wind: 3.0,
        shore: Some(Shore {
            distance: 10.0,
            direction: Vec3::X,
        }),
        ..AmbienceState::default()
    };
    let all = targets(&noon);
    let placed = targets(&AmbienceState {
        placed_only: true,
        ..noon
    });
    assert!(all[2].gain > 0.0 && all[3].gain > 0.0);
    assert_eq!(placed[2].gain, 0.0);
    assert_eq!(placed[3].gain, 0.0);
    assert_eq!(placed[0], all[0]);
    assert_eq!(placed[4], all[4]);
}
