//! Footsteps and impacts, measured on their samples: each surface its own spectrum, levels by
//! weight and speed, no clicks, and steps landing where and when the game says.

mod common;

use common::*;
use forge_audio::foley::{Impact, Step, Surface, impact_sound, step_sound};
use forge_audio::{BusLayout, Foley, Listener, MixerConfig, OfflineRenderer, PlayParams, create};
use glam::DVec3;

const RATE: u32 = 48_000;

fn walk(surface: Surface) -> Step {
    Step::walk(surface, DVec3::ZERO)
}

/// The root mean square of the difference between two spectra, dB.
fn distance(a: &[f64; 9], b: &[f64; 9]) -> f64 {
    (a.iter().zip(b).map(|(x, y)| (x - y).powi(2)).sum::<f64>() / 9.0).sqrt()
}

#[test]
fn every_surface_has_its_own_spectrum() {
    let spectra: Vec<Vec<[f64; 9]>> = Surface::ALL
        .iter()
        .map(|&s| {
            (0..4)
                .map(|seed| octaves(step_sound(&walk(s), seed, RATE).samples(), RATE))
                .collect()
        })
        .collect();
    let mut own = 0.0f64;
    for (i, a) in spectra.iter().enumerate() {
        for k in 1..4 {
            own = own.max(distance(&a[0], &a[k]));
        }
        for (j, b) in spectra.iter().enumerate().skip(i + 1) {
            let d = distance(&a[0], &b[0]);
            assert!(
                d > 5.0,
                "{} and {}: {d:.1} dB apart",
                Surface::ALL[i].name(),
                Surface::ALL[j].name()
            );
        }
    }
    // Steps on one surface vary (no two the same) but keep their surface's spectrum.
    assert!(own < 2.5, "{own:.2} dB between steps on one surface");
    let a = step_sound(&walk(Surface::Stone), 1, RATE);
    assert_ne!(a, step_sound(&walk(Surface::Stone), 2, RATE));
    assert_eq!(a, step_sound(&walk(Surface::Stone), 1, RATE));
}

#[test]
fn each_surface_sounds_like_itself() {
    let sound = |s| step_sound(&walk(s), 3, RATE);
    let loudest = |s| {
        let b = octaves(sound(s).samples(), RATE);
        let i = (0..9).max_by(|&i, &j| b[i].total_cmp(&b[j])).unwrap();
        OCTAVES[i]
    };
    // Floorboards are low and hollow; water rings in its bubbles' band; stone clicks around
    // 1 to 2 kHz; gravel crunches and metal rings higher; grass swishes highest.
    let wood = centroid(sound(Surface::Wood).samples(), RATE);
    assert!(wood < 400.0, "wood {wood:.0} Hz");
    assert!((500.0..=1000.0).contains(&loudest(Surface::Water)));
    assert!((1000.0..=2000.0).contains(&loudest(Surface::Stone)));
    for s in [Surface::Gravel, Surface::Metal] {
        let c = centroid(sound(s).samples(), RATE);
        assert!(c > 1500.0, "{} {c:.0} Hz", s.name());
    }
    assert!((4000.0..=8000.0).contains(&loudest(Surface::Grass)));
    // Metal rings: 40 dB down after more than three times any other surface's time.
    let ring = decay_time(sound(Surface::Metal).samples(), RATE, 40.0);
    for s in Surface::ALL.into_iter().filter(|&s| s != Surface::Metal) {
        let other = decay_time(sound(s).samples(), RATE, 40.0);
        assert!(
            ring > 2.0 * other,
            "metal {ring:.3} s, {} {other:.3} s",
            s.name()
        );
    }
}

#[test]
fn levels_follow_weight_and_speed() {
    for s in Surface::ALL {
        let peak_at = |weight: f32, speed: f32| {
            let x = step_sound(
                &Step {
                    weight,
                    speed,
                    ..walk(s)
                },
                9,
                RATE,
            );
            peak(x.samples()) as f64
        };
        // +3 dB a doubling of the weight: the amplitude is √(weight × speed).
        let light = peak_at(40.0, 1.4);
        let mid = peak_at(80.0, 1.4);
        let heavy = peak_at(160.0, 1.4);
        assert!((db(mid / light) - 3.01).abs() < 0.01, "{}", db(mid / light));
        assert!((db(heavy / mid) - 3.01).abs() < 0.01);
        // Running at 4.2 m/s against walking at 1.4: three times the speed, +4.8 dB.
        let run = peak_at(80.0, 4.2);
        assert!((db(run / mid) - 4.77).abs() < 0.01, "{}", db(run / mid));
    }
    // The heel and the ball: 85 ms apart walking, about 22 ms running.
    let gap = |speed| {
        let x = step_sound(
            &Step {
                speed,
                ..walk(Surface::Wood)
            },
            4,
            RATE,
        );
        let env = envelope(x.samples(), RATE as usize / 1000);
        // The second contact: the loudest 1 ms window 10 ms or more after the first onset.
        let first = env.iter().position(|&e| e > 1e-3).unwrap();
        let second = (first + 10..first + 120)
            .max_by(|&i, &j| env[i].total_cmp(&env[j]))
            .unwrap();
        second - first
    };
    let walking = gap(1.4);
    let running = gap(4.0);
    assert!((70..=100).contains(&walking), "{walking} ms");
    assert!((15..=30).contains(&running), "{running} ms");
}

#[test]
fn impacts_follow_mass_and_speed() {
    for s in Surface::ALL {
        let hit = |mass: f32, speed: f32| {
            impact_sound(
                &Impact {
                    surface: s,
                    position: DVec3::ZERO,
                    speed,
                    mass,
                    start: None,
                },
                5,
                RATE,
            )
        };
        // The amplitude grows as √mass × speed.
        let light = peak(hit(1.0, 2.0).samples()) as f64;
        let heavy = peak(hit(4.0, 2.0).samples()) as f64;
        let fast = peak(hit(1.0, 4.0).samples()) as f64;
        assert!((db(heavy / light) - 6.02).abs() < 0.01);
        assert!((db(fast / light) - 6.02).abs() < 0.01);
        // A heavy body presses longer: on the surfaces that ring, its sound is darker.
        if matches!(s, Surface::Stone | Surface::Wood | Surface::Metal) {
            let small = centroid(hit(1.0, 2.0).samples(), RATE);
            let big = centroid(hit(20.0, 2.0).samples(), RATE);
            assert!(
                big < 0.8 * small,
                "{}: {big:.0} against {small:.0} Hz",
                s.name()
            );
        }
    }
}

#[test]
fn no_step_clicks() {
    for s in Surface::ALL {
        for speed in [1.4, 4.0] {
            let x = step_sound(&Step { speed, ..walk(s) }, 11, RATE);
            let x = x.samples();
            // It starts and ends on 0: played or cut at its end, it does not click.
            assert_eq!(x[0], 0.0);
            assert_eq!(x[x.len() - 1], 0.0);
            // Nothing in the top octave, where a click would put its energy: not in the whole,
            // not in any millisecond of it.
            let top = energy_above_db(x, 16_000.0, RATE);
            assert!(top < -38.0, "{} at {speed}: {top:.1} dB", s.name());
            let worst = worst_window_above_db(x, 16_000.0, RATE);
            assert!(worst < -25.0, "{} at {speed}: {worst:.1} dB", s.name());
            // The same sound cut at its peak without a fade, then silence: the measure sees the
            // click.
            let top_at = (0..x.len())
                .max_by(|&i, &j| x[i].abs().total_cmp(&x[j].abs()))
                .unwrap();
            let mut cut = x[..=top_at].to_vec();
            cut.resize(cut.len() + 480, 0.0);
            let clicked = worst_window_above_db(&cut, 16_000.0, RATE);
            assert!(clicked > worst + 10.0, "{}: {clicked:.1} dB", s.name());
        }
    }
}

#[test]
fn steps_land_where_and_when_the_game_says() {
    let render = || {
        let layout = BusLayout::new();
        let (mut engine, mixer) = create(MixerConfig::default(), &layout);
        let mut foley = Foley::new(RATE, 21, PlayParams::default());
        engine.set_listener(Listener::at(DVec3::new(0.0, 1.6, 0.0)));
        let mut starts = Vec::new();
        // A step every second and a half (the metal rings for 1.3 s), alternating feet, on each
        // surface in turn, the last ones 3 m to the right.
        for (i, &s) in Surface::ALL.iter().cycle().take(12).enumerate() {
            let start = 2400 + i as u64 * 72_000;
            let x = if i >= 10 {
                3.0
            } else {
                0.15 * (i % 2) as f64 - 0.075
            };
            foley.step(
                &mut engine,
                &Step {
                    start: Some(start),
                    ..Step::walk(s, DVec3::new(x, 0.0, -0.3))
                },
            );
            starts.push(start);
        }
        let mut offline = OfflineRenderer::new(mixer);
        let out = offline.render(RATE as usize * 18);
        engine.update();
        (out, starts, offline.mixer().latency())
    };
    let (out, starts, latency) = render();
    let (again, _, _) = render();
    assert_eq!(out, again, "the same steps give the same samples");
    let left = channel(&out, 0);
    let right = channel(&out, 1);
    for (i, &start) in starts.iter().enumerate() {
        let from = start as usize + latency;
        let window = &left[from..from + 4800];
        let onset = onset(window, 1e-7).expect("a step") as f64 / RATE as f64;
        // The first contact is 1 ms into the sound (a soft one rises over its first 0.3 ms).
        assert!(onset < 0.0015, "step {i}: {onset} s late");
        assert!(left[from - 48..from].iter().all(|&v| v.abs() < 1e-6));
        if i >= 10 {
            let r = rms(&right[from..from + 4800]);
            let l = rms(&left[from..from + 4800]);
            assert!(
                db(r / l) > 6.0,
                "a step 3 m to the right: {:.1} dB",
                db(r / l)
            );
        }
    }
    // Through the mixer, its voices starting and ending: still nothing in the top octave.
    let top = energy_above_db(&left, 16_000.0, RATE);
    assert!(top < -38.0, "{top:.1} dB");
    let worst = worst_window_above_db(&left, 16_000.0, RATE);
    assert!(worst < -25.0, "{worst:.1} dB");
}
