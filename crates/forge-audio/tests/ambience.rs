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
    assert!(band(&hot) > -32.0, "{:.1} dBFS", band(&hot));
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
    // falling, a shower, the walk down to the quay, a dive at 20 s and up at 25 s.
    let state = |t: f64| {
        let t = t as f32;
        let bump = |from: f32, to: f32| {
            let u = ((t - from) / (to - from)).clamp(0.0, 1.0);
            (std::f32::consts::PI * u).sin()
        };
        AmbienceState {
            hour: 4.0 + t / 3.0,
            wind: 2.0 + 10.0 * bump(5.0, 15.0),
            rain: 8.0 * bump(8.0, 16.0),
            shore: Some(Shore {
                distance: 100.0 - 95.0 * (t / 18.0).min(1.0),
                direction: Vec3::NEG_Z,
            }),
            medium: if (20.0..25.0).contains(&t) {
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
