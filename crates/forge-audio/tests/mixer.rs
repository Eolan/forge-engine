//! The mixer, rendered headless and checked on its samples (issue #267, scope 1 and 3).

mod common;

use std::sync::Arc;

use common::*;
use forge_audio::sound::{impulse, noise, sine, sine_loop};
use forge_audio::spatial::{Attenuation, pan_gains};
use forge_audio::{
    BLOCK, BusId, BusLayout, Control, EndReason, Event, Listener, MixThread, MixerConfig,
    OfflineRenderer, PlayParams, SampleBuffer, TickClock, VoiceId, create, db_to_gain,
};
use glam::{DVec3, Vec3};

const RATE: u32 = 48_000;
/// A centred mono voice's gain in each channel.
const C: f32 = std::f32::consts::FRAC_1_SQRT_2;

fn offline(config: MixerConfig, layout: &BusLayout) -> (forge_audio::AudioEngine, OfflineRenderer) {
    let (engine, mixer) = create(config, layout);
    (engine, OfflineRenderer::new(mixer))
}

fn plain() -> (forge_audio::AudioEngine, OfflineRenderer) {
    offline(MixerConfig::default(), &BusLayout::new())
}

/// A constant level, for checks that need no frequency.
fn dc(level: f32, seconds: f64) -> Arc<SampleBuffer> {
    SampleBuffer::mono(RATE, vec![level; (seconds * RATE as f64) as usize]).shared()
}

fn ended(events: &[Event]) -> Vec<(VoiceId, EndReason)> {
    events
        .iter()
        .map(|e| match *e {
            Event::Ended { voice, reason } => (voice, reason),
        })
        .collect()
}

#[test]
fn a_source_on_the_left_is_louder_in_the_left_channel() {
    for (x, louder) in [(-3.0, 0), (3.0, 1)] {
        let (mut engine, mut renderer) = plain();
        let tone = sine_loop(500.0, 0.5, 0.1, RATE).shared();
        let mut params = PlayParams::at(DVec3::new(x, 0.0, -3.0));
        params.looping = true;
        engine.play(&tone, params);
        let out = renderer.render_seconds(0.5);
        let skip = 2 * BLOCK;
        let left = rms(&channel(&out, 0)[skip..]);
        let right = rms(&channel(&out, 1)[skip..]);
        let (near, far) = if louder == 0 {
            (left, right)
        } else {
            (right, left)
        };
        // 45° to the side: the pan is ∓0.707, so the channels' gains are those of the law.
        let gains = pan_gains(if louder == 0 {
            -std::f32::consts::FRAC_1_SQRT_2
        } else {
            std::f32::consts::FRAC_1_SQRT_2
        });
        let expected = db((gains[louder] / gains[1 - louder]) as f64);
        assert!(expected > 12.0);
        assert!(
            (db(near / far) - expected).abs() < 0.1,
            "{} dB, expected {expected}",
            db(near / far)
        );
    }
}

#[test]
fn two_sources_mix_to_the_expected_rms() {
    let (mut engine, mut renderer) = plain();
    let a = sine_loop(440.0, 0.3, 0.5, RATE).shared();
    let b = sine_loop(1000.0, 0.2, 0.5, RATE).shared();
    let looping = PlayParams {
        looping: true,
        ..PlayParams::default()
    };
    engine.play(&a, looping);
    engine.play(&b, looping);
    let out = renderer.render_seconds(1.0);
    // Uncorrelated tones add in power; a centred mono voice gets −3 dB a channel.
    let expected =
        std::f64::consts::FRAC_1_SQRT_2 * ((0.3f64.powi(2) + 0.2f64.powi(2)) / 2.0).sqrt();
    for c in 0..2 {
        let level = rms(&channel(&out, c)[RATE as usize / 2..]);
        assert!(
            (level / expected - 1.0).abs() < 0.005,
            "channel {c}: {level} against {expected}"
        );
    }
    // Two copies of one sound in phase add in amplitude: +6 dB.
    let (mut engine, mut renderer) = plain();
    engine.play(&a, looping);
    let single = rms(&channel(&renderer.render_seconds(0.5), 0)[4096..]);
    let (mut engine, mut renderer) = plain();
    engine.play(&a, looping);
    engine.play(&a, looping);
    let double = rms(&channel(&renderer.render_seconds(0.5), 0)[4096..]);
    assert!((db(double / single) - 6.02).abs() < 0.01);
}

#[test]
fn the_limiter_holds_the_ceiling() {
    let (mut engine, mut renderer) = plain();
    let loud = sine(220.0, 0.9, 1.0, RATE).shared();
    let hiss = noise(3, 0.8, 1.0, RATE).shared();
    engine.play(&loud, PlayParams::default());
    engine.play(
        &hiss,
        PlayParams {
            pan: 0.4,
            ..PlayParams::default()
        },
    );
    // +18 dB on the master.
    engine.control(Control::BusGain {
        bus: BusId::MASTER,
        gain: 8.0,
    });
    let out = renderer.render_seconds(1.0);
    let ceiling = db_to_gain(-1.0);
    assert!(peak(&out) <= ceiling, "peak {}", peak(&out));
    assert!(peak(&out) > 0.99 * ceiling, "the limiter took too much");
    let stats = engine.stats();
    assert_eq!(
        stats.limiter_overs, 0,
        "the clamp caught samples the limiter let through"
    );
}

#[test]
fn voice_stealing_drops_the_lowest_priority() {
    let config = MixerConfig {
        max_voices: 4,
        ..MixerConfig::default()
    };
    let (mut engine, mut renderer) = offline(config, &BusLayout::new());
    let frequencies = [300.0, 500.0, 700.0, 900.0, 1100.0];
    let priorities = [5u8, 1, 7, 3, 6];
    let tones: Vec<_> = frequencies
        .iter()
        .map(|&f| sine_loop(f, 0.1, 0.1, RATE).shared())
        .collect();
    let mut ids = Vec::new();
    for i in 0..4 {
        ids.push(engine.play(
            &tones[i],
            PlayParams {
                looping: true,
                priority: priorities[i],
                ..PlayParams::default()
            },
        ));
    }
    renderer.render_seconds(0.1);
    ids.push(engine.play(
        &tones[4],
        PlayParams {
            looping: true,
            priority: priorities[4],
            ..PlayParams::default()
        },
    ));
    let out = renderer.render_seconds(0.3);
    let left = channel(&out, 0);
    let late = &left[RATE as usize / 10..];
    for (i, &f) in frequencies.iter().enumerate() {
        let level = tone(late, f, RATE);
        if i == 1 {
            assert!(level < 1e-4, "{f} Hz, priority 1, still plays at {level}");
        } else {
            assert!((level - 0.1 * C as f64).abs() < 0.002, "{f} Hz at {level}");
        }
    }
    assert_eq!(ended(engine.update()), vec![(ids[1], EndReason::Stolen)]);
    let stats = engine.stats();
    assert_eq!((stats.stolen, stats.real_voices), (1, 4));
}

#[test]
fn at_equal_priority_the_farthest_voice_is_stolen() {
    let config = MixerConfig {
        max_voices: 4,
        ..MixerConfig::default()
    };
    let (mut engine, mut renderer) = offline(config, &BusLayout::new());
    let tone = sine_loop(400.0, 0.2, 0.1, RATE).shared();
    let mut ids = Vec::new();
    for distance in [2.0, 16.0, 4.0, 8.0, 3.0] {
        let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -distance));
        params.looping = true;
        ids.push(engine.play(&tone, params));
    }
    renderer.render_seconds(0.1);
    assert_eq!(ended(engine.update()), vec![(ids[1], EndReason::Stolen)]);
    // A sixth, farther than all: refused.
    let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -50.0));
    params.looping = true;
    let far = engine.play(&tone, params);
    renderer.render_seconds(0.05);
    assert_eq!(ended(engine.update()), vec![(far, EndReason::Rejected)]);
}

#[test]
fn scheduling_is_sample_accurate() {
    let (mut engine, mut renderer) = plain();
    let latency = renderer.mixer().latency();
    let click = impulse(1.0, RATE).shared();
    // Frames on block edges and inside blocks, sent before and while rendering.
    let at = [12_345u64, 12_800, 30_001];
    for &frame in &at[..2] {
        engine.play(
            &click,
            PlayParams {
                start: Some(frame),
                ..PlayParams::default()
            },
        );
    }
    let mut out = renderer.render(20_000);
    assert!(engine.now() <= at[2]);
    engine.play(
        &click,
        PlayParams {
            start: Some(at[2]),
            ..PlayParams::default()
        },
    );
    out.extend(renderer.render(20_000));
    let left = channel(&out, 0);
    let hits: Vec<usize> = (0..left.len()).filter(|&i| left[i].abs() > 0.5).collect();
    let expected: Vec<usize> = at.iter().map(|&f| f as usize + latency).collect();
    assert_eq!(hits, expected);
    assert!(
        left.iter()
            .enumerate()
            .all(|(i, &v)| expected.contains(&i) || v == 0.0)
    );

    // A simulation tick and a fraction of it, through a tick clock tied at frame 4800.
    let (mut engine, mut renderer) = plain();
    let ticks = TickClock::new(10, 4800, 60, RATE);
    let frame = ticks.frame_at(40, 0.25);
    assert_eq!(frame, 4800 + 30 * 800 + 200);
    engine.play(
        &click,
        PlayParams {
            start: Some(frame),
            ..PlayParams::default()
        },
    );
    let left = channel(&renderer.render_seconds(1.0), 0);
    assert_eq!(onset(&left, 0.5), Some(frame as usize + latency));
    assert_eq!(engine.stats().late_commands, 0);
}

#[test]
fn a_stop_lands_on_its_frame() {
    let (mut engine, mut renderer) = plain();
    let latency = renderer.mixer().latency();
    let level = dc(0.5, 1.0);
    let voice = engine.play(
        &level,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    engine.control_at(10_001, Control::Stop { voice, fade: 0 });
    let left = channel(&renderer.render(20_000), 0);
    let last = 10_001 + latency;
    assert!(
        left[latency..last]
            .iter()
            .all(|&v| (v - 0.5 * C).abs() < 1e-4)
    );
    assert!(left[last..].iter().all(|&v| v == 0.0));
    assert_eq!(ended(engine.update()), vec![(voice, EndReason::Stopped)]);

    // With a fade, it falls to silence over the fade's frames, starting on its frame.
    let (mut engine, mut renderer) = plain();
    let voice = engine.play(
        &level,
        PlayParams {
            looping: true,
            ..PlayParams::default()
        },
    );
    engine.control_at(5000, Control::Stop { voice, fade: 480 });
    let left = channel(&renderer.render(10_000), 0);
    let start = 5000 + latency;
    assert!((left[start - 1] - 0.3535).abs() < 1e-3);
    assert!(left[start + 240] < left[start] && left[start + 240] > 0.1);
    assert!(left[start + 480..].iter().all(|&v| v == 0.0));
}

#[test]
fn the_output_is_deterministic() {
    fn scene() -> Vec<f32> {
        let mut layout = BusLayout::new();
        let reverb = layout.add_reverb(
            "reverb",
            BusId::MASTER,
            forge_audio::dsp::ReverbParams::HALL,
        );
        let sfx = layout.add("sfx", BusId::MASTER);
        layout.send(sfx, reverb, 0.3);
        let config = MixerConfig {
            max_voices: 6,
            max_virtual: 8,
            ..MixerConfig::default()
        };
        let (mut engine, mut renderer) = offline(config, &layout);
        let hiss = noise(11, 0.3, 0.4, 44_100).shared();
        let tone = sine_loop(330.0, 0.2, 0.25, 32_000).shared();
        let mut out = Vec::new();
        for step in 0..40u64 {
            let angle = step as f64 * 0.3;
            let mut params =
                PlayParams::at(DVec3::new(10.0 * angle.cos(), 0.0, 10.0 * angle.sin()));
            params.bus = sfx;
            params.priority = (step % 5) as u8;
            params.pitch = 0.8 + (step % 7) as f32 * 0.1;
            params.virtualize = step % 2 == 0;
            params.looping = step % 3 == 0;
            params.start = Some(engine.now() + step * 37);
            let sound = if step % 2 == 0 { &hiss } else { &tone };
            let voice = engine.play(sound, params);
            engine.control_at(
                engine.now() + 3000,
                Control::Position {
                    voice,
                    position: DVec3::new(-5.0, 1.0, -2.0),
                    velocity: Vec3::new(20.0, 0.0, 0.0),
                },
            );
            engine.set_listener(Listener {
                forward: Vec3::new(angle.sin() as f32, 0.0, -angle.cos() as f32),
                ..Listener::default()
            });
            out.extend(renderer.render(1500));
            engine.update();
        }
        out
    }
    let first = scene();
    let second = scene();
    assert!(first.iter().any(|&v| v != 0.0));
    assert!(
        first
            .iter()
            .zip(&second)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn a_virtual_voice_keeps_its_time_and_comes_back() {
    let config = MixerConfig {
        max_voices: 1,
        ..MixerConfig::default()
    };
    let (mut engine, mut renderer) = offline(config, &BusLayout::new());
    let latency = renderer.mixer().latency();
    // 400 Hz in exactly 120 frames a period, looping.
    let tone = sine_loop(400.0, 0.5, 0.1, RATE).shared();
    let background = engine.play(
        &tone,
        PlayParams {
            looping: true,
            virtualize: true,
            priority: 10,
            ..PlayParams::default()
        },
    );
    renderer.render(4096);
    let urgent = engine.play(
        &dc(0.2, 0.1),
        PlayParams {
            priority: 200,
            ..PlayParams::default()
        },
    );
    let out = renderer.render(48_000);
    let events = ended(engine.update());
    assert_eq!(events, vec![(urgent, EndReason::Finished)]);
    assert!(!events.iter().any(|&(v, _)| v == background));
    let stats = engine.stats();
    assert_eq!((stats.virtualized, stats.promoted), (1, 1));
    // Back and in phase with where it would have been had it never left: the frame heard at
    // output frame f is the tone's frame f − latency.
    let left = channel(&out, 0);
    let offset = 4096;
    for (i, &heard) in left.iter().enumerate().take(21_000).skip(20_000) {
        let frame = (offset + i - latency) as f64;
        let expected =
            0.5 * std::f64::consts::FRAC_1_SQRT_2 * (std::f64::consts::TAU * frame / 120.0).sin();
        assert!(
            (heard as f64 - expected).abs() < 1e-3,
            "frame {i}: {heard} against {expected}"
        );
    }
}

#[test]
fn buses_scale_and_send() {
    let mut layout = BusLayout::new();
    let aux = layout.add("aux", BusId::MASTER);
    let sfx = layout.add("sfx", BusId::MASTER);
    let slot = layout.send(sfx, aux, 0.25);
    layout.set_gain(sfx, 0.5);
    let (mut engine, mut renderer) = offline(MixerConfig::default(), &layout);
    engine.play(
        &dc(0.4, 1.0),
        PlayParams {
            bus: sfx,
            looping: true,
            ..PlayParams::default()
        },
    );
    let left = channel(&renderer.render(4096), 0);
    // Direct 0.5, and the send's 0.25 of that: 0.625 of the voice's level.
    let voice = 0.4 * C;
    assert!((left[4000] - 0.625 * voice).abs() < 1e-4, "{}", left[4000]);
    engine.control(Control::BusSend {
        bus: sfx,
        slot,
        level: 0.0,
    });
    engine.control(Control::BusGain {
        bus: aux,
        gain: 0.0,
    });
    let left = channel(&renderer.render(4096), 0);
    assert!((left[4000] - 0.5 * voice).abs() < 1e-4);
}

#[test]
fn distance_follows_its_curve() {
    let mut levels = Vec::new();
    for distance in [1.0, 2.0, 4.0, 8.0] {
        let (mut engine, mut renderer) = plain();
        let tone = sine_loop(250.0, 0.5, 0.1, RATE).shared();
        let mut params = PlayParams::at(DVec3::new(0.0, 0.0, -distance));
        params.looping = true;
        if let Some(s) = &mut params.spatial {
            s.air = [0.0; 3];
            s.attenuation = Attenuation::Inverse {
                reference: 1.0,
                max: 100.0,
                rolloff: 1.0,
            };
        }
        engine.play(&tone, params);
        levels.push(rms(&channel(&renderer.render_seconds(0.3), 0)[4096..]));
    }
    for pair in levels.windows(2) {
        assert!((db(pair[1] / pair[0]) + 6.02).abs() < 0.05, "{levels:?}");
    }
}

#[test]
fn the_mix_thread_takes_commands_from_the_game_thread() {
    let (mut engine, mixer) = create(MixerConfig::default(), &BusLayout::new());
    let (thread, mut ring) = MixThread::spawn(mixer, 2048);
    let click = impulse(0.9, RATE).shared();
    // Fifty clicks 900 frames apart from half a second on, sent while the mixer runs (up to
    // the ring's 2048 frames ahead) and before the device pulls, so none can be late.
    let clicks: Vec<u64> = (0..50).map(|i| 24_000 + i * 900).collect();
    for &frame in &clicks {
        engine.play(
            &click,
            PlayParams {
                start: Some(frame),
                ..PlayParams::default()
            },
        );
    }
    // The device: a thread pulling 1.5 s in pieces of 480 frames, ten times faster than
    // real time.
    let device = std::thread::spawn(move || {
        let mut heard = Vec::new();
        let mut piece = vec![0.0f32; 960];
        while heard.len() < RATE as usize * 3 {
            // Keep what the ring had: a short pull pads with silence, which a device would
            // play, shifting what follows.
            let frames = ring.pull(&mut piece);
            heard.extend_from_slice(&piece[..frames * 2]);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        heard
    });
    let mut finished = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while finished < 50 && std::time::Instant::now() < deadline {
        finished += engine.update().len();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let heard = device.join().expect("the device thread");
    let mixer = thread.stop();
    assert!(mixer.now() >= RATE as u64 * 3 / 2);
    assert_eq!(finished, 50);
    let stats = engine.stats();
    assert_eq!((stats.started, stats.late_commands), (50, 0));
    // Every click on its frame, behind the limiter's look-ahead.
    let left = channel(&heard, 0);
    let hits: Vec<u64> = (0..left.len())
        .filter(|&i| left[i].abs() > 0.5)
        .map(|i| i as u64)
        .collect();
    let latency = mixer.latency() as u64;
    assert_eq!(hits, clicks.iter().map(|f| f + latency).collect::<Vec<_>>());
}

#[test]
fn a_full_command_ring_keeps_the_order() {
    let config = MixerConfig {
        command_capacity: 4,
        ..MixerConfig::default()
    };
    let (mut engine, mut renderer) = offline(config, &BusLayout::new());
    let click = impulse(0.5, RATE).shared();
    let mut ids = Vec::new();
    for _ in 0..20 {
        ids.push(engine.play(&click, PlayParams::default()));
    }
    assert_eq!(engine.backlog(), 16);
    let mut finished = Vec::new();
    for _ in 0..10 {
        renderer.render(BLOCK);
        finished.extend(ended(engine.update()).into_iter().map(|(v, _)| v));
    }
    assert_eq!(engine.backlog(), 0);
    assert_eq!(finished, ids);
}

#[test]
fn a_stopping_voice_gives_way_first_and_stays_stopped() {
    let config = MixerConfig {
        max_voices: 1,
        ..MixerConfig::default()
    };
    let (mut engine, mut renderer) = offline(config, &BusLayout::new());
    let tone = sine_loop(300.0, 0.3, 0.1, RATE).shared();
    let first = engine.play(
        &tone,
        PlayParams {
            looping: true,
            priority: 100,
            ..PlayParams::default()
        },
    );
    let second = engine.play(
        &tone,
        PlayParams {
            looping: true,
            priority: 50,
            virtualize: true,
            ..PlayParams::default()
        },
    );
    renderer.render(4096);
    assert_eq!(engine.stats().virtual_voices, 1);
    // A long fade: the waiting voice takes the place at once, and the first still fades out
    // as a tail and reports that it was stopped.
    engine.stop(first, 9600);
    renderer.render(4096);
    assert_eq!(ended(engine.update()), vec![(first, EndReason::Stopped)]);
    let stats = engine.stats();
    assert_eq!(
        (
            stats.stolen,
            stats.promoted,
            stats.real_voices,
            stats.virtual_voices
        ),
        (0, 1, 1, 0)
    );
    engine.stop(second, 0);
    renderer.render(1024);
    assert_eq!(ended(engine.update()), vec![(second, EndReason::Stopped)]);
}
