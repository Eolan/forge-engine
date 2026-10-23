//! Footsteps and impacts, measured on their samples: each surface its own spectrum, each shoe
//! its own sound on hard ground, steps that vary audibly, levels by weight and speed, no
//! clicks, and steps landing where and when the game says.

mod common;

use common::*;
use forge_audio::foley::{Footwear, Impact, ImpactBody, Step, Surface, impact_sound, step_sound};
use forge_audio::{
    BusLayout, Control, Foley, Listener, MixerConfig, OfflineRenderer, PlayParams, create,
};
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
            (0..6)
                .map(|seed| octaves(step_sound(&walk(s), seed, RATE).samples(), RATE))
                .collect()
        })
        .collect();
    let mut own = 0.0f64;
    let mut apart = f64::MAX;
    for (i, a) in spectra.iter().enumerate() {
        for p in 0..6 {
            for q in p + 1..6 {
                own = own.max(distance(&a[p], &a[q]));
            }
        }
        for b in spectra.iter().skip(i + 1) {
            apart = apart.min(distance(&a[0], &b[0]));
        }
    }
    // Steps on one surface vary (up to 4.5 dB between two of them) but every surface stays
    // further from every other than that.
    assert!(own < 4.5, "{own:.2} dB between steps on one surface");
    assert!(
        apart > 5.0 && apart > 1.5 * own,
        "surfaces {apart:.1} dB apart, steps {own:.2}"
    );
    let a = step_sound(&walk(Surface::Stone), 1, RATE);
    assert_ne!(a, step_sound(&walk(Surface::Stone), 2, RATE));
    assert_eq!(a, step_sound(&walk(Surface::Stone), 1, RATE));
}

#[test]
fn each_surface_sounds_like_itself() {
    let sound = |s| step_sound(&walk(s), 3, RATE);
    let bands = |s| octaves(sound(s).samples(), RATE);
    let loudest = |s| {
        let b = bands(s);
        let i = (0..9).max_by(|&i, &j| b[i].total_cmp(&b[j])).unwrap();
        OCTAVES[i]
    };
    // Floorboards are low and hollow; water rings in its bubbles' band; a hard heel on stone
    // clicks across the spectrum, its 4 and 8 kHz octaves within 4 dB of its loudest; gravel
    // crunches and metal rings higher; grass swishes highest.
    let wood = centroid(sound(Surface::Wood).samples(), RATE);
    assert!(wood < 400.0, "wood {wood:.0} Hz");
    assert!((500.0..=1000.0).contains(&loudest(Surface::Water)));
    let stone = bands(Surface::Stone);
    let top = stone.iter().cloned().fold(f64::MIN, f64::max);
    assert!(stone[6].max(stone[7]) > top - 4.0, "stone {stone:?}");
    for s in [Surface::Gravel, Surface::Metal] {
        let c = centroid(sound(s).samples(), RATE);
        assert!(c > 1500.0, "{} {c:.0} Hz", s.name());
    }
    assert!((4000.0..=8000.0).contains(&loudest(Surface::Grass)));
    // Metal rings: 40 dB down after more than twice any other surface's time.
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
fn shoes_change_a_step_on_hard_ground() {
    let on = |surface, footwear| {
        step_sound(
            &Step {
                footwear,
                ..walk(surface)
            },
            3,
            RATE,
        )
    };
    let c = |surface, footwear| centroid(on(surface, footwear).samples(), RATE);
    // On stone: a leather heel clicks, trainers and bare feet pat, boots thud.
    let leather = c(Surface::Stone, Footwear::LeatherShoes);
    assert!(
        leather > 2.0 * c(Surface::Stone, Footwear::Sneakers),
        "{leather:.0} Hz"
    );
    assert!(leather > 3.0 * c(Surface::Stone, Footwear::Barefoot));
    let low = |f| band_db(on(Surface::Stone, f).samples(), 40.0, 250.0, RATE);
    assert!(low(Footwear::Boots) > low(Footwear::LeatherShoes) + 6.0);
    // Bare feet are the quietest.
    let barefoot = peak(on(Surface::Stone, Footwear::Barefoot).samples());
    for f in Footwear::ALL
        .into_iter()
        .filter(|&f| f != Footwear::Barefoot)
    {
        assert!(
            barefoot < peak(on(Surface::Stone, f).samples()),
            "{}",
            f.name()
        );
    }
    // A sandal slaps the heel 120 to 200 ms after the ball lands: more there than a trainer.
    let late = |f| {
        let x = on(Surface::Stone, f);
        let from = (0.17 * 48_000.0) as usize;
        let to = (0.31 * 48_000.0) as usize;
        // The loudest 5 ms between 500 Hz and 3.5 kHz.
        x.samples()[from..to]
            .chunks(240)
            .map(|w| band_db(w, 500.0, 3500.0, RATE))
            .fold(f64::MIN, f64::max)
    };
    assert!(late(Footwear::Sandals) > late(Footwear::Sneakers) + 6.0);
    // On grass the ground decides: the shoes (not the sandal's slap) move the centroid by
    // under 15 %, where on stone they move it threefold.
    let grass: Vec<f64> = [
        Footwear::Barefoot,
        Footwear::Sneakers,
        Footwear::LeatherShoes,
        Footwear::Boots,
    ]
    .map(|f| c(Surface::Grass, f))
    .to_vec();
    let (min, max) = grass
        .iter()
        .fold((f64::MAX, 0.0f64), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(max / min < 1.15, "grass {grass:?}");
}

#[test]
fn repeated_steps_differ_audibly() {
    // Twelve steps in a row on stone in leather: no two alike, their brightness, their level
    // and the space between heel and ball each spread wide enough to hear.
    let steps: Vec<_> = (0..12)
        .map(|seed| step_sound(&walk(Surface::Stone), seed, RATE))
        .collect();
    for (i, a) in steps.iter().enumerate() {
        for b in &steps[i + 1..] {
            assert_ne!(a, b);
        }
    }
    let centroids: Vec<f64> = steps.iter().map(|s| centroid(s.samples(), RATE)).collect();
    let (lo, hi) = centroids
        .iter()
        .fold((f64::MAX, 0.0f64), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi / lo > 1.2, "centroids {lo:.0} to {hi:.0} Hz");
    let levels: Vec<f64> = steps.iter().map(|s| db(rms(s.samples()))).collect();
    let (lo, hi) = levels
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi - lo > 2.0, "levels {lo:.1} to {hi:.1} dBFS");
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
    // The heel and the ball: 85 ms apart walking (±15 %), about 22 ms running. A hard heel on
    // wood rings briefly; the ball is where the level jumps most.
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
        let first = env.iter().position(|&e| e > 1e-3).unwrap();
        let second = (first + 15..first + 120)
            .max_by(|&i, &j| (env[i] - env[i - 1]).total_cmp(&(env[j] - env[j - 1])))
            .unwrap();
        second - first
    };
    let walking = gap(1.4);
    let running = gap(4.0);
    assert!((68..=102).contains(&walking), "{walking} ms");
    assert!((15..=30).contains(&running), "{running} ms");
}

#[test]
fn impacts_follow_mass_and_speed() {
    for s in Surface::ALL {
        let hit = |mass: f32, speed: f32| {
            impact_sound(
                &Impact {
                    surface: s,
                    against: None,
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

/// Two bodies meeting: each with a surface rings, both driven by the pair's reduced mass, and
/// the softer surface sets how long the contact lasts (Phase 7's contacts to the material
/// layer).
#[test]
fn two_bodies_meeting_ring_each_other() {
    let body = |surface, mass| ImpactBody { surface, mass };
    let at = DVec3::new(1.0, 0.0, 2.0);
    // A 20 kg crate dropped on fixed stone: the stone and the crate, each at 20 kg.
    let crate_on_stone: Vec<Impact> = Impact::between(
        body(Some(Surface::Wood), Some(20.0)),
        body(Some(Surface::Stone), None),
        at,
        3.0,
        Some(48),
    )
    .collect();
    assert_eq!(crate_on_stone.len(), 2);
    assert_eq!(
        (crate_on_stone[0].surface, crate_on_stone[0].against),
        (Surface::Wood, Some(Surface::Stone))
    );
    assert_eq!(
        (crate_on_stone[1].surface, crate_on_stone[1].against),
        (Surface::Stone, Some(Surface::Wood))
    );
    for hit in &crate_on_stone {
        assert_eq!((hit.mass, hit.speed, hit.start), (20.0, 3.0, Some(48)));
        assert_eq!(hit.position, at);
    }
    // Two 10 kg bodies: 5 kg each way.
    let pair: Vec<Impact> = Impact::between(
        body(Some(Surface::Metal), Some(10.0)),
        body(Some(Surface::Metal), Some(10.0)),
        at,
        2.0,
        None,
    )
    .collect();
    assert_eq!(pair.len(), 2);
    assert!(pair.iter().all(|h| (h.mass - 5.0).abs() < 1e-6));
    // A body of no surface on a plate: the plate alone; two fixed bodies: nothing.
    let plate: Vec<Impact> = Impact::between(
        body(None, Some(2.0)),
        body(Some(Surface::Metal), None),
        at,
        2.0,
        None,
    )
    .collect();
    assert_eq!(plate.len(), 1);
    assert_eq!((plate[0].surface, plate[0].against), (Surface::Metal, None));
    let none = Impact::between(
        body(Some(Surface::Stone), None),
        body(Some(Surface::Stone), None),
        at,
        2.0,
        None,
    );
    assert_eq!(none.count(), 0);

    // The softer decides: a steel plate's upper modes ring less the softer what strikes it; a
    // harder body changes nothing (stone struck by metal is stone's own contact).
    let hit = |surface, against| {
        impact_sound(
            &Impact {
                surface,
                against,
                position: DVec3::ZERO,
                speed: 2.0,
                mass: 2.0,
                start: None,
            },
            7,
            RATE,
        )
    };
    // The plate's upper modes (3 to 9 kHz): a long, soft contact hardly excites them.
    let ring = |against| band_db(hit(Surface::Metal, against).samples(), 3000.0, 9000.0, RATE);
    // Struck by steel −29.7 dB, by a wooden crate −53.3, by a sack (grass) nothing (−126).
    let by_steel = ring(Some(Surface::Metal));
    let by_wood = ring(Some(Surface::Wood));
    let by_soft = ring(Some(Surface::Grass));
    assert!(
        by_wood < by_steel - 15.0 && by_soft < by_wood - 30.0,
        "steel {by_steel:.1}, wood {by_wood:.1}, grass {by_soft:.1} dB"
    );
    assert_eq!(
        hit(Surface::Stone, Some(Surface::Metal)).samples(),
        hit(Surface::Stone, None).samples()
    );
}

/// A material row's sound layer names each of the foley's surfaces.
#[test]
fn a_material_rows_sound_layer_names_each_surface() {
    use forge_core::material::SoundSurface;
    for s in Surface::ALL {
        assert_eq!(Surface::from(SoundSurface::from(s)), s);
    }
    assert_eq!(Surface::from(SoundSurface::default()), Surface::Stone);
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
        // The first contact is 1 ms into the sound (a soft one, water's 30 ms, rises over its
        // first millisecond and a half).
        assert!(onset < 0.0025, "step {i}: {onset} s late");
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

/// Made on the engine's thread (`AudioEngine::make_off_thread`): the same steps, named the same
/// voices, give the same samples as made in the call once `update` has started them; a control
/// sent to a voice while it is being made waits for it (a step silenced before it was made stays
/// silent).
#[test]
fn sounds_made_off_the_game_thread_are_the_same() {
    let render = |off: bool| {
        let layout = BusLayout::new();
        let (mut engine, mixer) = create(MixerConfig::default(), &layout);
        if off {
            engine.make_off_thread().expect("the maker thread");
        }
        let mut foley = Foley::new(RATE, 21, PlayParams::default());
        engine.set_listener(Listener::at(DVec3::new(0.0, 1.6, 0.0)));
        let voices: Vec<_> = Surface::ALL
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let step = Step {
                    start: Some(2400 + i as u64 * 72_000),
                    ..Step::walk(s, DVec3::new(0.0, 0.0, -0.3))
                };
                foley.step(&mut engine, &step)
            })
            .collect();
        engine.control(Control::Gain {
            voice: voices[2],
            gain: 0.0,
        });
        engine.update();
        let mut offline = OfflineRenderer::new(mixer);
        let out = offline.render(RATE as usize * 9);
        (out, voices, offline.mixer().latency())
    };
    let (inline, voices, latency) = render(false);
    let (made, made_voices, _) = render(true);
    assert_eq!(voices, made_voices);
    assert_eq!(inline, made, "the same samples");
    let left = channel(&made, 0);
    let silenced = 2400 + 2 * 72_000 + latency;
    assert!(peak(&left[silenced..silenced + 4800]) < 1e-6);
    let heard = 2400 + 3 * 72_000 + latency;
    assert!(peak(&left[heard..heard + 4800]) > 1e-3);
}
