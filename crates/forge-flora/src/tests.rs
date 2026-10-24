//! The growth's tests on the village's species: branch count by age, taper, no wood through
//! wood, determinism by seed, sizes within the published ranges, and the inputs (wind, pruning)
//! doing what they say.

use glam::DVec2;

use crate::graph::TreeGraph;
use crate::grow::{Conditions, FORK_HOPS, Pruning, grow};
use crate::species::{Species, mediterranean};

fn grown(sp: &Species, seed: u64) -> TreeGraph {
    grow(sp, &Conditions::new(sp.maturity, seed))
}

#[test]
fn branch_count_grows_with_age() {
    for sp in &mediterranean() {
        let counts: Vec<usize> = [0.2, 0.45, 1.0]
            .iter()
            .map(|k| grow(sp, &Conditions::new(sp.maturity * k, 7)).branch_count())
            .collect();
        assert!(
            counts.windows(2).all(|w| w[0] < w[1]),
            "{}: branches {counts:?} by age",
            sp.id
        );
    }
}

#[test]
fn a_child_is_never_thicker_than_its_parent() {
    for sp in &mediterranean() {
        for seed in [1, 2] {
            let g = grown(sp, seed);
            assert!(g.nodes.len() > 50, "{}: {} nodes", sp.id, g.nodes.len());
            let faults = g.taper_faults();
            assert!(
                faults.is_empty(),
                "{}: {} taper faults",
                sp.id,
                faults.len()
            );
            // And the trunk thickens to the base.
            assert!(g.base_radius() > 4.0 * sp.tip_radius as f32, "{}", sp.id);
        }
    }
}

#[test]
fn no_branch_passes_through_another() {
    for sp in &mediterranean() {
        for seed in [1, 2] {
            let g = grown(sp, seed);
            let crossings = g.crossings(FORK_HOPS);
            assert!(
                crossings.is_empty(),
                "{}: {} crossings, e.g. {:?}",
                sp.id,
                crossings.len(),
                &crossings[..crossings.len().min(4)]
            );
        }
    }
}

#[test]
fn the_same_seed_grows_the_same_plant() {
    for sp in &mediterranean() {
        let a = grown(sp, 11);
        let b = grown(sp, 11);
        assert_eq!(a, b, "{}", sp.id);
        let c = grown(sp, 12);
        assert_ne!(a.nodes, c.nodes, "{}: two seeds, one plant", sp.id);
    }
}

#[test]
fn grown_plants_have_their_published_size() {
    for sp in &mediterranean() {
        for seed in 1..=4 {
            let g = grown(sp, seed);
            let (h, w) = (f64::from(g.height()), f64::from(g.crown_width()));
            assert!(
                (sp.height[0]..=sp.height[1]).contains(&h),
                "{} seed {seed}: {h:.2} m tall, published {:?}",
                sp.id,
                sp.height
            );
            assert!(
                (sp.crown_width[0]..=sp.crown_width[1]).contains(&w),
                "{} seed {seed}: {w:.2} m across, published {:?}",
                sp.id,
                sp.crown_width
            );
        }
    }
}

#[test]
fn a_wind_leans_the_crown_downwind() {
    let all = mediterranean();
    for id in ["aleppo-pine", "holm-oak"] {
        let sp = crate::species::by_id(&all, id).unwrap();
        let still = grown(sp, 3);
        let mut c = Conditions::new(sp.maturity, 3);
        c.wind = DVec2::new(0.8, 0.0);
        let windy = grow(sp, &c);
        let shift = windy.crown_centre().x - still.crown_centre().x;
        assert!(
            f64::from(shift) > 0.08 * f64::from(still.height()),
            "{id}: the crown moved {shift:.2} m downwind"
        );
    }
}

#[test]
fn pruning_shapes_the_crown() {
    let all = mediterranean();
    let olive = crate::species::by_id(&all, "olive").unwrap();
    let mut c = Conditions::new(olive.maturity, 5);
    c.pruning = Pruning {
        lift: 1.0,
        top: 4.5,
        open_centre: 0.8,
    };
    let g = grow(olive, &c);
    assert!(
        g.height() < 4.5 + olive.leaf_cluster as f32 * 1.3,
        "{}",
        g.height()
    );
    for (i, n) in g.nodes.iter().enumerate() {
        if n.starts_branch && n.order > 0 {
            let p = g.nodes[n.parent as usize].pos;
            assert!(p.y >= 1.0 - 1e-4, "a branch from {} m at node {i}", p.y);
        }
    }
    // The vase: no wood within the open centre above the scaffold.
    let centre = g
        .nodes
        .iter()
        .filter(|n| n.pos.y > 2.5 && glam::Vec2::new(n.pos.x, n.pos.z).length() < 0.4)
        .count();
    assert_eq!(centre, 0, "wood in the olive's open centre");
}

/// How round a crown is from above: its outline's least reach over its greatest, the reach the
/// farthest leaf in each of 24 directions round the leaves' middle.
fn roundness(g: &TreeGraph) -> f32 {
    const SECTORS: usize = 24;
    let n = g.leaves.len().max(1) as f32;
    let (x, z) = g
        .leaves
        .iter()
        .fold((0.0, 0.0), |(x, z), l| (x + l.pos.x / n, z + l.pos.z / n));
    let mut reach = [0.0_f32; SECTORS];
    for l in &g.leaves {
        let (dx, dz) = (l.pos.x - x, l.pos.z - z);
        let turn = dz.atan2(dx) / std::f32::consts::TAU + 0.5;
        let s = ((turn * SECTORS as f32) as usize).min(SECTORS - 1);
        reach[s] = reach[s].max(dx.hypot(dz));
    }
    let most = reach.iter().copied().fold(0.0, f32::max);
    reach.iter().copied().fold(most, f32::min) / most.max(1e-6)
}

#[test]
fn broad_crowns_are_lobed_not_balls() {
    // The owner, 2026-10-24: every crown the same ball. The lobes make each outline less round
    // than the same plant's in the smooth envelope.
    for id in ["holm-oak", "london-plane", "common-lime"] {
        let sp = crate::species::by_id(&mediterranean(), id).unwrap().clone();
        let smooth = Species {
            lobe_depth: 0.0,
            ..sp.clone()
        };
        for seed in 1..=3 {
            let (lobed, ball) = (
                roundness(&grown(&sp, seed)),
                roundness(&grown(&smooth, seed)),
            );
            assert!(
                lobed < ball - 0.04,
                "{id} seed {seed}: roundness {lobed:.2} lobed, {ball:.2} smooth"
            );
        }
    }
}
