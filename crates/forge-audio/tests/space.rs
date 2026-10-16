//! Space on rendered samples: walls, air, Doppler, reverb zones (issue #267, scope 2).

mod common;

use std::sync::Arc;

use common::*;
use forge_audio::dsp::{OnePole, ReverbParams};
use forge_audio::sound::{impulse, sine_loop};
use forge_audio::spatial::{AcousticMaterial, Attenuation, Occlusion};
use forge_audio::zones::{ReverbZone, ReverbZones, ZoneShape};
use forge_audio::{
    BusId, BusLayout, Listener, MixerConfig, OfflineRenderer, PlayParams, SampleBuffer, create,
};
use glam::{DVec3, Vec3};

const RATE: u32 = 48_000;
/// One tone in each of Steam Audio's bands.
const BANDS: [f64; 3] = [150.0, 2500.0, 16_000.0];

/// The three tones in one looping sound, 0.2 each.
fn three_tones() -> Arc<SampleBuffer> {
    let tones: Vec<_> = BANDS
        .iter()
        .map(|&f| sine_loop(f, 0.2, 0.1, RATE))
        .collect();
    let data = (0..tones[0].frames())
        .map(|i| tones.iter().map(|t| t.samples()[i]).sum())
        .collect();
    SampleBuffer::mono(RATE, data).shared()
}

/// The level of each band's tone in the left channel, for a source placed by `params`.
fn band_levels(params: PlayParams, listener: Listener) -> [f64; 3] {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    engine.set_listener(listener);
    engine.play(
        &three_tones(),
        PlayParams {
            looping: true,
            ..params
        },
    );
    let out = OfflineRenderer::new(mixer).render_seconds(0.6);
    let left = channel(&out, 0);
    let steady = &left[9600..9600 + 19_200];
    BANDS.map(|f| tone(steady, f, RATE))
}

/// A source 5 m ahead with no distance curve and no air, so only what is tested changes.
fn ahead(occlusion: Occlusion) -> PlayParams {
    let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -5.0));
    if let Some(s) = &mut params.spatial {
        s.attenuation = Attenuation::None;
        s.air = [0.0; 3];
        s.occlusion = occlusion;
    }
    params
}

#[test]
fn a_wall_between_source_and_listener_drops_the_level() {
    let clear = band_levels(ahead(Occlusion::NONE), Listener::default());
    // Behind a wood wall: each band falls by the wood's transmission (−23, −37, −46 dB); the
    // shelves reach each band's gain within 1 dB.
    let wood = AcousticMaterial::WOOD;
    let walled = band_levels(ahead(Occlusion::through(1.0, &[wood])), Listener::default());
    let drops: Vec<f64> = (0..3).map(|b| db(walled[b] / clear[b])).collect();
    for (b, (&drop, &t)) in drops.iter().zip(&wood.transmission).enumerate() {
        let expected = db(t as f64);
        assert!(
            (drop - expected).abs() < 1.0,
            "band {b}: {drop:.1} dB, expected {expected:.1}"
        );
    }
    assert!(
        drops[0] > drops[1] && drops[1] > drops[2],
        "the highs fall most: {drops:?}"
    );
    // Half hidden behind brick (flat −36 dB): (1 − 0.5) + 0.5 × 0.015 of the level, −5.9 dB.
    let half = band_levels(
        ahead(Occlusion::through(0.5, &[AcousticMaterial::BRICK])),
        Listener::default(),
    );
    for b in 0..3 {
        assert!(
            (db(half[b] / clear[b]) - db(0.5075)).abs() < 0.2,
            "band {b}"
        );
    }
    // Two walls: their transmissions multiply.
    let two = Occlusion::through(1.0, &[AcousticMaterial::BRICK, AcousticMaterial::GLASS]);
    assert!((two.transmission[0] - 0.015 * 0.06).abs() < 1e-7);
}

#[test]
fn air_takes_the_highs_from_a_far_source() {
    let at = |distance: f64| {
        let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -distance));
        if let Some(s) = &mut params.spatial {
            s.attenuation = Attenuation::None;
        }
        band_levels(params, Listener::default())
    };
    let (near, far) = (at(1.0), at(100.0));
    // Steam Audio's coefficients over 99 m: −0.17, −1.46 and −15.65 dB.
    for (b, tolerance) in [(0, 0.1), (1, 0.3), (2, 0.5)] {
        let expected = -(forge_audio::spatial::AIR_ABSORPTION[b] as f64) * 99.0 * 20.0
            / std::f64::consts::LN_10;
        let measured = db(far[b] / near[b]);
        assert!(
            (measured - expected).abs() < tolerance,
            "band {b}: {measured:.2} dB, expected {expected:.2}"
        );
    }
}

#[test]
fn doppler_shifts_the_pitch() {
    let heard = |source_velocity: Vec3, listener_velocity: Vec3| {
        let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
        engine.set_listener(Listener {
            velocity: listener_velocity,
            ..Listener::default()
        });
        let tone = sine_loop(1000.0, 0.5, 0.1, RATE).shared();
        let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -200.0));
        params.looping = true;
        if let Some(s) = &mut params.spatial {
            s.velocity = source_velocity;
        }
        engine.play(&tone, params);
        let left = channel(&OfflineRenderer::new(mixer).render_seconds(1.0), 0);
        frequency(&left[9600..], RATE)
    };
    let c = forge_audio::spatial::SPEED_OF_SOUND as f64;
    // The source comes at a tenth of the speed of sound, then goes; then the listener comes.
    let cases = [
        (Vec3::new(0.0, 0.0, 34.3), Vec3::ZERO, c / (c - 34.3)),
        (Vec3::new(0.0, 0.0, -34.3), Vec3::ZERO, c / (c + 34.3)),
        (Vec3::ZERO, Vec3::new(0.0, 0.0, -34.3), (c + 34.3) / c),
    ];
    for (source, listener, factor) in cases {
        let f = heard(source, listener);
        assert!(
            (f / (1000.0 * factor) - 1.0).abs() < 1e-4,
            "{f} Hz, expected {}",
            1000.0 * factor
        );
    }
}

/// The decay time of the low band (below about 500 Hz) of a response, from Schroeder's
/// backward integral fitted between −5 and −35 dB.
fn low_band_rt60(response: &[f32]) -> f64 {
    let mut lp = OnePole::new(500.0, RATE);
    let filtered: Vec<f32> = response.iter().map(|&x| lp.tick(x)).collect();
    let mut energy: Vec<f64> = filtered.iter().map(|&x| x as f64 * x as f64).collect();
    for i in (0..energy.len() - 1).rev() {
        energy[i] += energy[i + 1];
    }
    let total = energy[0];
    let level = |e: f64| 10.0 * (e / total).log10();
    let t5 = energy
        .iter()
        .position(|&e| level(e) <= -5.0)
        .expect("−5 dB");
    let t35 = energy
        .iter()
        .position(|&e| level(e) <= -35.0)
        .expect("−35 dB");
    2.0 * (t35 - t5) as f64 / RATE as f64
}

#[test]
fn reverb_zones_follow_the_listener() {
    let mut zones = ReverbZones::new(ReverbParams::OPEN);
    zones.add(ReverbZone {
        shape: ZoneShape::Box {
            min: DVec3::new(-10.0, -5.0, -10.0),
            max: DVec3::new(10.0, 10.0, 10.0),
        },
        fade: 5.0,
        params: ReverbParams::HALL,
    });
    let inside = DVec3::ZERO;
    let fading = DVec3::new(12.5, 0.0, 0.0);
    let outside = DVec3::new(100.0, 0.0, 0.0);
    assert_eq!(zones.params_at(inside), ReverbParams::HALL);
    assert_eq!(zones.params_at(outside), ReverbParams::OPEN);
    let half = zones.params_at(fading);
    assert!(
        (half.rt60 as f64 - (2.4f64 * 0.35).sqrt()).abs() < 1e-3,
        "{}",
        half.rt60
    );

    for position in [inside, fading, outside] {
        let mut layout = BusLayout::new();
        let reverb = layout.add_reverb("reverb", BusId::MASTER, ReverbParams::OPEN);
        let dry = layout.add("dry", BusId::MASTER);
        layout.set_gain(dry, 0.0);
        let (mut engine, mixer) = create(MixerConfig::default(), &layout);
        let expected = zones.follow(&mut engine, reverb, position);
        // An impulse sent to the reverb alone (its own bus muted).
        let mut params = PlayParams {
            bus: dry,
            start: Some(4800),
            ..PlayParams::default()
        };
        params.sends[0] = Some((reverb, 1.0));
        engine.play(&impulse(1.0, RATE).shared(), params);
        let seconds = 0.2 + expected.rt60 as f64 * 1.2;
        let left = channel(&OfflineRenderer::new(mixer).render_seconds(seconds), 0);
        assert!(left[..4800].iter().all(|&v| v == 0.0));
        let measured = low_band_rt60(&left[4800..]);
        assert!(
            (measured / expected.rt60 as f64 - 1.0).abs() < 0.05,
            "at {position}: {measured:.2} s, expected {:.2}",
            expected.rt60
        );
    }
}
