//! The fauna's tests: the data, the example plan, determinism by seed, the flight distances on
//! direct approaches, flocks that hold together and clear the walls, and a day that keeps every
//! animal in its habitat and its routine.

use std::sync::OnceLock;

use glam::Vec2;

use crate::passers::{Disturber, Passers, Threat};
use crate::plan::{self, Parts, PatchKind, Plan, SiteKind};
use crate::sim::{Activity, Fauna, HZ, Mode, Roster, SAFE_BIRD, SAFE_CAT, STEP, ticks};
use crate::species::{Behaviour, Species, index_of, village};

fn the_plan() -> &'static Plan {
    static P: OnceLock<Plan> = OnceLock::new();
    P.get_or_init(|| Plan::new(Parts::example(1)))
}

fn the_species() -> &'static [Species] {
    static S: OnceLock<Vec<Species>> = OnceLock::new();
    S.get_or_init(village)
}

fn index(b: Behaviour) -> u32 {
    index_of(the_species(), b).expect("a village species") as u32
}

#[test]
fn the_species_file_reads() {
    let all = the_species();
    assert_eq!(all.len(), 8);
    for s in all {
        assert!(
            s.body > 0.0 && s.group[0] >= 1 && s.group[0] <= s.group[1],
            "{}",
            s.id
        );
        assert!(s.cruise > 0.0 && s.turn > 0.0 && s.range > 0.0, "{}", s.id);
        for d in [s.flee_person, s.flee_vehicle, s.flee_dog, s.flee_cat] {
            assert!((0.0..=30.0).contains(&d), "{}", s.id);
        }
    }
    let sparrow = &all[index(Behaviour::Sparrow) as usize];
    assert_eq!(
        sparrow.flee_person, 10.0,
        "García-Arroyo et al. 2023's mean"
    );
    assert_eq!(all[index(Behaviour::Dog) as usize].leash, 1.8);
}

#[test]
fn the_example_plan_has_its_places() {
    let plan = the_plan();
    let p = &plan.parts;
    assert!(p.buildings.len() >= 50, "{} houses", p.buildings.len());
    assert!(p.trees.len() >= 100 && p.shrubs.len() >= 500);
    for kind in [
        SiteKind::Roof,
        SiteKind::Tree,
        SiteKind::Wall,
        SiteKind::Quay,
        SiteKind::Nest,
        SiteKind::Water,
    ] {
        assert!(plan.sites.iter().any(|s| s.kind == kind), "{kind:?}");
    }
    for kind in [
        PatchKind::Garden,
        PatchKind::Square,
        PatchKind::Street,
        PatchKind::Quay,
    ] {
        assert!(plan.patches.iter().any(|q| q.kind == kind), "{kind:?}");
    }
    // Nests on the street and garden fronts, outside the walls, never on a party wall.
    let with = plan.nests.iter().filter(|n| n[0] != u32::MAX).count();
    assert!(
        with * 10 >= p.buildings.len() * 9,
        "{with} houses with nests"
    );
    for s in plan.sites.iter().filter(|s| s.kind == SiteKind::Nest) {
        assert!(
            !plan.solid(s.pos, 0.0),
            "a nest inside a wall at {:?}",
            s.pos
        );
    }
    // The shrubs a herd browses are in its pasture; the fold opens onto it.
    assert!(p.shrubs.iter().all(|&s| plan.in_pasture(s)));
    let f = plan::centroid(&p.fold);
    assert!(plan.in_pasture(f) && plan.in_pasture(f - Vec2::new(0.0, 10.0)));
    // A point inside a house is moved out of it.
    let b = &p.buildings[0];
    let inside = plan::centroid(&b.footprint);
    assert!(plan.building_at(inside).is_some());
    assert!(plan.building_at(plan.push_out(inside)).is_none());
}

/// A run of `minutes` from `hour` with the example's passers.
fn run(seed: u64, roster: &Roster, hour: f32, minutes: f32) -> u64 {
    let plan = the_plan();
    let passers = Passers::example(plan, 60, 20, seed);
    let mut f = Fauna::new(plan, the_species(), roster, seed, hour * 3600.0);
    f.run(ticks(minutes * 60.0), Some(&passers));
    f.digest()
}

#[test]
fn the_same_seed_makes_the_same_animals() {
    let roster = Roster::village(the_species());
    let a = run(3, &roster, 7.0, 20.0);
    let b = run(3, &roster, 7.0, 20.0);
    let c = run(4, &roster, 7.0, 20.0);
    assert_eq!(a, b, "the same seed, the same digest");
    assert_ne!(a, c, "another seed, other animals");
}

/// One scene: a group of `b` held at `held`, a threat of kind `threat` going from `from` at `speed`
/// through `at` and on for a minute. Returns the first flight's distance and threshold, and the
/// closest any member then came to it without fleeing, over its threshold at that moment.
fn approach(
    b: Behaviour,
    held: Vec2,
    at: Vec2,
    from: Vec2,
    speed: f32,
    threat: Threat,
) -> (f32, f32, f32) {
    let plan = the_plan();
    let s = index(b);
    let mut f = Fauna::new(plan, the_species(), &Roster::only(s, 1), 9, 12.0 * 3600.0);
    f.hold(0, held, 600.0);
    let mut pos = from;
    let mut first = None;
    let mut worst = f32::MAX;
    let z = if matches!(threat, Threat::Vehicle { .. }) {
        0.8
    } else {
        1.0
    };
    let vel = (at - from).normalize() * speed;
    for _ in 0..ticks(60.0) {
        let me = Disturber {
            pos: pos.extend(z),
            vel: vel.extend(0.0),
            threat,
        };
        f.step(&[me]);
        pos += vel * STEP;
        if first.is_none()
            && let Some(x) = f.flushes.first()
        {
            first = Some((x.distance, x.threshold));
        }
        let g = &f.groups[0];
        if g.activity == Activity::Flee {
            continue;
        }
        for i in g.first..g.first + g.len {
            let i = i as usize;
            let safe = match b {
                Behaviour::Cat => SAFE_CAT,
                Behaviour::Fish => f32::MAX,
                _ => SAFE_BIRD,
            };
            if !matches!(
                f.mode[i],
                Mode::Ground | Mode::Rest | Mode::Walk | Mode::Water
            ) || f.pos[i].z >= safe
            {
                continue;
            }
            let off = (f.pos[i] - me.pos).truncate();
            let coming = me.vel.truncate().dot(off.normalize_or_zero()) > 0.2;
            let thr = f.threshold(i, threat, coming);
            worst = worst.min(off.length() / thr);
        }
    }
    let (d, t) = first.unwrap_or_else(|| panic!("{b:?} never fled {threat:?}"));
    (d, t, worst)
}

#[test]
fn animals_flee_at_their_distances() {
    let plan = the_plan();
    let square = plan::centroid(&plan.parts.squares[0]);
    let quay = &plan.parts.quay;
    let on_quay = (quay[0] + quay[1]) * 0.5 - Vec2::new(0.0, 1.5);
    let in_water = (quay[0] + quay[1]) * 0.5 + Vec2::new(0.0, 1.5);
    let walkway = (quay[0] + quay[1]) * 0.5 - Vec2::new(0.0, 1.5);
    let pasture = plan::centroid(&plan.parts.pasture);
    let walk = 1.3;
    let cases = [
        (
            Behaviour::Sparrow,
            square,
            Vec2::new(-30.0, 0.0),
            walk,
            Threat::Person,
            10.0,
        ),
        (
            Behaviour::Pigeon,
            square,
            Vec2::new(0.0, -20.0),
            walk,
            Threat::Person,
            3.0,
        ),
        (
            Behaviour::Gull,
            on_quay,
            Vec2::new(-30.0, 0.0),
            walk,
            Threat::Person,
            9.0,
        ),
        (
            Behaviour::Goat,
            pasture,
            Vec2::new(30.0, 0.0),
            walk,
            Threat::Person,
            4.0,
        ),
        (
            Behaviour::Goat,
            pasture,
            Vec2::new(0.0, 40.0),
            2.0,
            Threat::Dog,
            20.0,
        ),
        (
            Behaviour::Fish,
            walkway,
            Vec2::new(-30.0, 0.0),
            walk,
            Threat::Person,
            3.0,
        ),
        // A car at 30 km/h along the street past a sparrows' patch: 6 m at 50 km/h, scaled.
        (
            Behaviour::Sparrow,
            Vec2::new(150.0, 250.0),
            Vec2::new(-60.0, 0.0),
            30.0 / 3.6,
            Threat::Vehicle { limit: 30.0 },
            3.6,
        ),
    ];
    for (b, at, off, speed, threat, expect) in cases {
        let held = if b == Behaviour::Fish { in_water } else { at };
        let (d, t, worst) = approach(b, held, at, at + off, speed, threat);
        assert!(
            (t - expect).abs() < 1e-4,
            "{b:?}: threshold {t} against {expect}"
        );
        assert!(
            d <= t && d >= t - speed * STEP - 0.05,
            "{b:?} fled {threat:?} at {d:.2} m, its distance {t:.2} m"
        );
        assert!(
            worst >= 0.95,
            "{b:?}: a member let {threat:?} within {worst:.2} of its distance"
        );
    }
}

#[test]
fn a_cat_leaves_for_a_wall_or_home_when_a_dog_comes() {
    let plan = the_plan();
    let s = index(Behaviour::Cat);
    let mut f = Fauna::new(plan, the_species(), &Roster::only(s, 1), 5, 12.0 * 3600.0);
    let door = f.groups[0].yard.as_ref().expect("a yard").door;
    let at = door + Vec2::new(0.0, if door.y < 250.0 { -1.5 } else { 1.5 });
    f.hold(0, at, 600.0);
    let from = at + Vec2::new(-25.0, 0.0);
    let mut pos = from;
    let mut fled = false;
    for _ in 0..ticks(40.0) {
        let to = at - pos;
        let d = to.length();
        let vel = if d > 0.15 { to / d * 1.5 } else { Vec2::ZERO };
        let dog = Disturber {
            pos: pos.extend(0.0),
            vel: vel.extend(0.0),
            threat: Threat::Dog,
        };
        f.step(&[dog]);
        pos += vel * STEP;
        fled |= !f.flushes.is_empty();
    }
    assert!(fled, "the cat saw the dog");
    let x = f.flushes[0];
    assert!(
        x.distance <= 12.0 && x.distance >= 12.0 - 0.2,
        "fled at {}",
        x.distance
    );
    // On the street the cat goes in at its door.
    assert_eq!(f.mode[0], Mode::Hidden);
}

#[test]
fn flocks_hold_together_and_clear_the_walls() {
    // The evening: sparrows to roost, pigeons circling and to their ledges, swifts' screaming
    // parties down the streets, with the passers.
    let plan = the_plan();
    let sp = the_species();
    let roster = Roster {
        groups: vec![
            (index(Behaviour::Sparrow), 6),
            (index(Behaviour::Pigeon), 3),
            (index(Behaviour::Swift), 4),
            (index(Behaviour::Gull), 2),
        ],
    };
    let passers = Passers::example(plan, 60, 20, 2);
    let mut f = Fauna::new(plan, sp, &roster, 2, 19.5 * 3600.0);
    let mut out = Vec::new();
    let mut flying_samples = 0;
    let mut parties = 0;
    for k in 0..ticks(2.25 * 3600.0) {
        out.clear();
        passers.at(f.seconds(), plan, &mut out);
        f.step(&out);
        if k % u64::from(HZ) != 0 {
            continue;
        }
        for (g, gr) in f.groups.iter().enumerate() {
            let up: Vec<usize> = (gr.first..gr.first + gr.len)
                .map(|i| i as usize)
                .filter(|&i| f.mode[i] == Mode::Air)
                .collect();
            if gr.activity == Activity::Party {
                parties += 1;
            }
            // A flock aloft for a while (not taking off or landing one by one).
            if up.len() < gr.len as usize
                || !matches!(gr.activity, Activity::Circle | Activity::Party)
            {
                continue;
            }
            flying_samples += 1;
            let c = f.centroid(g, Some(Mode::Air));
            let spread = up
                .iter()
                .map(|&i| (f.pos[i] - c).length())
                .fold(0.0, f32::max);
            let limit = match sp[gr.species as usize].behaviour {
                Behaviour::Swift => 45.0,
                Behaviour::Gull => 60.0,
                _ => 30.0,
            };
            assert!(spread < limit, "group {g} spread over {spread:.1} m");
        }
        for i in 0..f.len() {
            if f.mode[i] == Mode::Air {
                assert!(
                    !plan.solid(f.pos[i], 0.0),
                    "a bird in a wall at {:?}",
                    f.pos[i]
                );
            }
        }
    }
    assert!(
        flying_samples > 500 && parties > 500,
        "{flying_samples} {parties}"
    );
    assert_eq!(
        f.contacts, 0,
        "flyers that had to slide along a wall: {:?}",
        f.contacts_by
    );
}

#[test]
fn a_day_keeps_each_animal_in_its_habitat() {
    let plan = the_plan();
    let sp = the_species();
    let roster = Roster {
        groups: vec![
            (index(Behaviour::Sparrow), 2),
            (index(Behaviour::Pigeon), 1),
            (index(Behaviour::Gull), 1),
            (index(Behaviour::Swift), 1),
            (index(Behaviour::Cat), 4),
            (index(Behaviour::Dog), 4),
            (index(Behaviour::Goat), 1),
            (index(Behaviour::Fish), 1),
        ],
    };
    let passers = Passers::example(plan, 60, 20, 6);
    let mut f = Fauna::new(plan, sp, &roster, 6, 0.0);
    let mut out = Vec::new();
    let n = f.len();
    let mut out_of_doors = vec![0u32; n];
    for k in 0..ticks(86_400.0) {
        out.clear();
        passers.at(f.seconds(), plan, &mut out);
        f.step(&out);
        if k % u64::from(HZ) != 0 {
            continue;
        }
        let hour = f.seconds() / 3600.0;
        for (i, out) in out_of_doors.iter_mut().enumerate() {
            let g = &f.groups[f.group[i] as usize];
            let s = &sp[g.species as usize];
            let p = f.pos[i];
            let m = f.mode[i];
            if m != Mode::Hidden {
                *out += 1;
            }
            match s.behaviour {
                Behaviour::Goat => assert!(plan.in_pasture(p.truncate()), "a goat out at {p:?}"),
                Behaviour::Fish => {
                    assert!(plan.in_water(p.truncate()), "a fish ashore at {p:?}");
                    assert!(plan.from_quay(p.truncate()) <= s.range + 0.01);
                    assert!((-4.0..=-0.15).contains(&p.z));
                }
                Behaviour::Cat if m != Mode::Hidden => {
                    let door = g.yard.as_ref().map_or(p.truncate(), |y| y.door);
                    assert!(
                        (p.truncate() - door).length() <= s.range,
                        "a cat far from home: {p:?}, its door {door:?}, {m:?}, {:?}, path {:?} spot {:?}",
                        g.activity,
                        f.path[i],
                        f.spot[i]
                    );
                    assert!(plan.building_at(p.truncate()).is_none(), "a cat in a wall");
                }
                Behaviour::Dog if m != Mode::Hidden => {
                    let owner = g.leash.as_ref().map(|l| l.owner).expect("an owner");
                    assert!(
                        (p - owner).truncate().length() <= s.leash + 0.01,
                        "off the leash"
                    );
                    assert!(
                        plan.building_at(p.truncate()).is_none() && !plan.in_water(p.truncate())
                    );
                }
                Behaviour::Sparrow => {
                    let home = plan.sites[g.home as usize].pos.truncate();
                    assert!(
                        (p.truncate() - home).length() <= s.range * 1.6,
                        "a sparrow strayed"
                    );
                }
                _ => {}
            }
            if matches!(m, Mode::Ground | Mode::Walk | Mode::Rest | Mode::Flee) && p.z < 0.5 {
                assert!(
                    plan.building_at(p.truncate()).is_none(),
                    "{:?} in a house",
                    s.behaviour
                );
            }
            if m == Mode::Air {
                assert!(!plan.solid(p, 0.0), "a bird in a wall");
            }
        }
        // The routine by the hour.
        let all = |b: Behaviour, ok: &dyn Fn(usize) -> bool| {
            (0..n)
                .filter(|&i| sp[f.groups[f.group[i] as usize].species as usize].behaviour == b)
                .all(ok)
        };
        if (2.0..2.01).contains(&hour) {
            assert!(
                all(Behaviour::Swift, &|i| f.mode[i] == Mode::Hidden),
                "swifts in their nests"
            );
            assert!(
                all(Behaviour::Sparrow, &|i| f.mode[i] == Mode::Rest),
                "sparrows at roost"
            );
            assert!(all(Behaviour::Goat, &|i| plan::inside(
                f.pos[i].truncate(),
                &plan.parts.fold
            )));
            assert!(
                all(Behaviour::Dog, &|i| f.mode[i] == Mode::Hidden),
                "dogs at home"
            );
        }
        if (11.0..11.01).contains(&hour) {
            assert!(
                all(Behaviour::Swift, &|i| f.mode[i] == Mode::Air),
                "swifts aloft"
            );
            for i in 0..n {
                if sp[f.groups[f.group[i] as usize].species as usize].behaviour == Behaviour::Goat
                    && plan::inside(f.pos[i].truncate(), &plan.parts.fold)
                {
                    let g = &f.groups[f.group[i] as usize];
                    panic!(
                        "a goat in the fold at 11: {:?} {:?} {:?} then {:?} path {:?} spot {:?} goal {:?}",
                        f.pos[i], f.mode[i], g.activity, g.then, f.path[i], f.spot[i], g.goal
                    );
                }
            }
        }
    }
    // A goat walks 3–8 km a day (4.3–7.4 km on Mediterranean ranges); a cat is out 2–12 hours
    // (5 h the Danish median) and walks under 5 km; a dog is walked half an hour to three hours.
    for i in 0..n {
        let s = &sp[f.groups[f.group[i] as usize].species as usize];
        let km = f.travelled[i] / 1000.0;
        let hours = out_of_doors[i] as f32 / 3600.0;
        match s.behaviour {
            Behaviour::Goat => assert!((3.0..8.0).contains(&km), "a goat walked {km:.1} km"),
            Behaviour::Cat => {
                assert!((2.0..12.0).contains(&hours), "a cat out {hours:.1} h");
                assert!(km < 5.0, "a cat walked {km:.1} km");
            }
            Behaviour::Dog => assert!((0.5..3.0).contains(&hours), "a dog out {hours:.1} h"),
            _ => {}
        }
    }
}
