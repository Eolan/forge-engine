//! Steam Audio's HRTF as the spatializer, measured on rendered samples. These need the Steam
//! Audio library (`steam-audio-sdk/`), so they run only with the `steam-audio` feature:
//! `cargo test --release -p forge-audio --features steam-audio --test steam`.

#![cfg(all(windows, feature = "steam-audio"))]

mod common;

use common::*;
use forge_audio::dsp::Biquad;
use forge_audio::dsp::synth::LowPass4;
use forge_audio::sound::{impulse, noise};
use forge_audio::spatial::Attenuation;
use forge_audio::steam::SteamSpatializer;
use forge_audio::{
    AudioEngine, BusLayout, Control, MixerConfig, OfflineRenderer, PlayParams, SampleBuffer, create,
};
use glam::{DVec3, Vec3};

const RATE: u32 = 48_000;

/// White noise under 9 kHz (an eighth-order low-pass, 40 dB down at 16 kHz), looping: what
/// the HRTF shapes. A linear filter cannot add to the top octave, so energy there is a click.
fn hiss(seed: u64) -> std::sync::Arc<SampleBuffer> {
    let mut x = noise(seed, 0.5, 2.0, RATE).samples().to_vec();
    LowPass4::new(9000.0, RATE).run(&mut x);
    LowPass4::new(9000.0, RATE).run(&mut x);
    SampleBuffer::mono(RATE, x).shared()
}

/// A mixer whose spatializer is Steam Audio's, or the panner.
fn mixer(hrtf: bool) -> (AudioEngine, OfflineRenderer) {
    let config = MixerConfig::default();
    let (engine, mut mixer) = create(config, &BusLayout::new());
    if hrtf {
        let steam = SteamSpatializer::for_mixer(&config).expect("Steam Audio starts");
        mixer.set_spatializer(Box::new(steam));
    }
    (engine, OfflineRenderer::new(mixer))
}

/// `seconds` of a looping sound at `position` (no air; no distance curve unless `curve`), the
/// listener at the origin facing −Z.
fn place_with(position: DVec3, seconds: f64, hrtf: bool, curve: bool) -> Vec<f32> {
    let (mut engine, mut renderer) = mixer(hrtf);
    let mut params = PlayParams::at(position);
    let spatial = params.spatial.as_mut().unwrap();
    if !curve {
        spatial.attenuation = Attenuation::None;
    }
    spatial.air = [0.0; 3];
    params.looping = true;
    engine.play(&hiss(1), params);
    renderer.render_seconds(seconds)
}

fn place(position: DVec3, seconds: f64, hrtf: bool) -> Vec<f32> {
    place_with(position, seconds, hrtf, false)
}

/// Levels in the third octaves from 1 to 20 kHz, dB.
fn thirds(x: &[f32]) -> Vec<f64> {
    (0..14)
        .map(|k| {
            let f = 1000.0 * 2f32.powf(k as f32 / 3.0);
            let mut a = Biquad::band_pass(f, 4.3, RATE);
            let mut b = a;
            let e: f64 = x
                .iter()
                .map(|&v| {
                    let y = b.tick(a.tick(v)) as f64;
                    y * y
                })
                .sum();
            10.0 * e.max(1e-30).log10()
        })
        .collect()
}

/// A circle at 4 m, once every two seconds, positions every 10 ms: the left channel.
fn circle() -> Vec<f32> {
    let (mut engine, mut renderer) = mixer(true);
    let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -4.0));
    params.looping = true;
    let voice = engine.play(&hiss(3), params);
    let mut out = Vec::new();
    for k in 0..400 {
        let a = k as f64 * std::f64::consts::TAU / 200.0;
        engine.control(Control::Position {
            voice,
            position: DVec3::new(4.0 * a.sin(), 0.0, -4.0 * a.cos()),
            velocity: Vec3::ZERO,
        });
        out.extend(renderer.render(480));
        engine.update();
    }
    channel(&out, 0)
}

#[test]
fn a_source_on_the_left_reaches_the_left_ear_first_and_louder() {
    let out = place(DVec3::new(-3.0, 0.0, 0.0), 1.0, true);
    let (l, r) = (channel(&out, 0), channel(&out, 1));
    let (l, r) = (&l[4800..], &r[4800..]);
    // The head's shadow: the far ear loses the highs.
    let ild = band_db(l, 2000.0, 8000.0, RATE) - band_db(r, 2000.0, 8000.0, RATE);
    assert!(ild > 10.0, "{ild:.1} dB");
    // The time difference, from the lows' cross-correlation: the right ear later.
    let low = |x: &[f32]| {
        let mut f = LowPass4::new(1500.0, RATE);
        x.iter().map(|&v| f.tick(v)).collect::<Vec<f32>>()
    };
    let (ll, rl) = (low(l), low(r));
    let c = |k: usize| -> f64 {
        ll.iter()
            .zip(&rl[k..])
            .map(|(&p, &q)| p as f64 * q as f64)
            .sum()
    };
    let lag = (0..60).max_by(|&a, &b| c(a).total_cmp(&c(b))).unwrap();
    let ms = lag as f64 * 1000.0 / RATE as f64;
    assert!((0.4..=0.9).contains(&ms), "{ms:.3} ms");
}

#[test]
fn ahead_behind_and_above_sound_different() {
    let spectrum = |position: DVec3| thirds(&channel(&place(position, 1.0, true), 0)[4800..]);
    let ahead = spectrum(DVec3::new(0.0, 0.0, -2.0));
    let behind = spectrum(DVec3::new(0.0, 0.0, 2.0));
    let above = spectrum(DVec3::new(0.0, 2.0, 0.0));
    // Over the third octaves of 1 to 20 kHz, where the head and the ear's folds shape the
    // sound: the root mean square of the differences.
    let apart = |a: &[f64], b: &[f64]| {
        (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f64>() / a.len() as f64).sqrt()
    };
    let back = apart(&ahead, &behind);
    let up = apart(&ahead, &above);
    assert!(back > 2.5, "ahead and behind {back:.2} dB");
    assert!(up > 2.5, "ahead and above {up:.2} dB");
    // The panner cannot tell ahead from behind: the same samples.
    let pan_ahead = place(DVec3::new(0.0, 0.0, -2.0), 0.5, false);
    let pan_behind = place(DVec3::new(0.0, 0.0, 2.0), 0.5, false);
    assert_eq!(pan_ahead, pan_behind);
}

#[test]
fn distance_and_level_hold() {
    // The same sound in two renders, at 2 m and at 4 m: the inverse law, applied before the
    // HRTF, −6.02 dB a doubling.
    let near = rms(&channel(
        &place_with(DVec3::new(0.0, 0.0, -2.0), 1.0, true, true),
        0,
    ));
    let far = rms(&channel(
        &place_with(DVec3::new(0.0, 0.0, -4.0), 1.0, true, true),
        0,
    ));
    assert!(
        (db(near / far) - 6.02).abs() < 0.01,
        "{:.3} dB",
        db(near / far)
    );
    // Straight ahead, the HRTF's level within 4 dB of the panner's.
    let hrtf = rms(&channel(&place(DVec3::new(0.0, 0.0, -2.0), 1.0, true), 0));
    let pan = rms(&channel(&place(DVec3::new(0.0, 0.0, -2.0), 1.0, false), 0));
    assert!(db(hrtf / pan).abs() < 4.0, "{:.2} dB", db(hrtf / pan));
}

#[test]
fn a_circling_source_does_not_click() {
    let left = circle();
    let worst = worst_window_above_db(&left[4800..], 16_000.0, RATE);
    // The same sound held still at four places: what the HRTF itself puts up there.
    let still = [0.0f64, 90.0, 180.0, 270.0]
        .map(|deg| {
            let a = deg.to_radians();
            let out = place(DVec3::new(4.0 * a.sin(), 0.0, -4.0 * a.cos()), 1.0, true);
            worst_window_above_db(&channel(&out, 0)[4800..], 16_000.0, RATE)
        })
        .into_iter()
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        worst < still + 3.0,
        "moving {worst:.1} dB, still {still:.1} dB"
    );
    // The same circle again: the same samples.
    assert_eq!(left, circle());
}

#[test]
fn a_click_lands_on_its_frame() {
    let (mut engine, mut renderer) = mixer(true);
    let click = impulse(1.0, RATE).shared();
    let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -1.0));
    params.start = Some(12_345);
    params.spatial.as_mut().unwrap().air = [0.0; 3];
    engine.play(&click, params);
    let out = renderer.render(24_000);
    let left = channel(&out, 0);
    let latency = renderer.mixer().latency();
    let at = onset(&left, 1e-4).expect("the click");
    // Silent before its frame; then the HRTF's own delay (its impulse responses start a
    // little late), 65 samples.
    assert!(at >= 12_345 + latency, "{at}");
    assert!(at < 12_345 + latency + 96, "{at}");
}
