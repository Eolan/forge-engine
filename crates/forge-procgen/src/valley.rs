//! The rivers' valleys carved into the field itself (issue #116, D-041's floodplains and
//! benches): a floor for each river wide enough for its water and, on its gentler reaches, a
//! bench or a floodplain, the valley's walls lowered at their foot to meet it. Every level of
//! detail draws the field, so every level carries the valleys; the channels
//! ([`crate::channel`]) are cut into the floor afterwards, on finer cells.
//! - Each point of a ribbon is typed by the water's fall over [`ValleyParams::slope_run`] metres
//!   either way: over `reaches.1` the water's own room only (a V valley: D-041's steps and
//!   pools), between the two a bench `bench` widths past the water, under `reaches.0` a
//!   floodplain `floodplain.0` widths either side of the course; smoothly between them.
//! - Each side's floor stops where the ground stands [`ValleyParams::deepest`] metres over it,
//!   marching out across the course: a deep V keeps its walls, an open valley gets its floor.
//!   The floor's width changes along the course by `taper` metres a metre at most.
//! - The floor stands `rise` over the water and falls towards it by `fall` a metre. Past its
//!   edge a wall rises, steepening over `ease` metres to a little more than the ground's own
//!   slope there (`wall`), until it meets the ground, joined by a smooth minimum over `join`
//!   metres: the wall's foot moves out and down, and the wall is never steeper than that. The
//!   floor's edge wanders with noise.
//! - Nothing is ever raised. Within reach of a lake nothing goes under the lake's level plus
//!   `lake_guard` (the lakes are dams in narrow valleys), and under the lakes nothing is
//!   carved; from `sea.1` metres of water level down to `sea.0` the carve fades out, so the
//!   mouths keep their beaches.
//!
//! A pure function of the field and the ribbons, `f64` with `sqrt` only, in index order (D-016).

use forge_task::TaskPool;

use crate::field::Field2;
use crate::hydrology::Lakes;
use crate::noise::fbm;
use crate::river::{Ribbon, RibbonPoint, offset, segment_distance, smooth_height, smoothstep};

/// How the valleys are carved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValleyParams {
    /// The water's slopes, m/m, under which a reach has a floodplain and a bench.
    pub reaches: (f64, f64),
    /// Metres along the course, either way, over which a point's slope is measured.
    pub slope_run: f64,
    /// The floor's metres past the water's edge in a steep reach: room for its banks.
    pub room: f64,
    /// A bench's metres past the water's edge, in the river's widths.
    pub bench: f64,
    /// A floodplain's half width from the course in the river's widths, and its least and
    /// most metres.
    pub floodplain: (f64, f64, f64),
    /// The most the ground may stand over the floor where a side's floor ends, metres.
    pub deepest: f64,
    /// The floor's height over the water, `a + b × depth` metres.
    pub rise: (f64, f64),
    /// The floor's fall towards the course, m/m.
    pub fall: f64,
    /// The wall's steepest, m/m: `a + b ×` the ground's slope past the floor's edge.
    pub wall: (f64, f64),
    /// Metres past the floor's edge over which the wall steepens to it.
    pub ease: f64,
    /// Metres over which the wall joins the ground (a smooth minimum).
    pub join: f64,
    /// The most metres a wall may reach past the floor's edge.
    pub reach: f64,
    /// How far the floor's edge wanders, a share of its widening, and over how many metres.
    pub wander: (f64, f64),
    /// The wander's seed.
    pub seed: u64,
    /// The water's levels, metres, over which the carve fades in from the sea.
    pub sea: (f64, f64),
    /// Metres over a lake's level under which the ground within reach of it is never lowered.
    pub lake_guard: f64,
    /// How fast a floor's width may change along the course, metres a metre.
    pub taper: f64,
}

impl Default for ValleyParams {
    /// D-041's types (a floodplain under 2 %, a bench to 4 %) on the slope over 40 m either way;
    /// 2 m of room past the water, a bench of one and a half widths, a floodplain three widths
    /// either side (12 to 64 m); a floor's side ending where the ground stands 6 m over it, the
    /// floor 0.3 m + half the depth over the water, falling 2 % towards it; walls at most 0.15
    /// steeper than 1.3 times the ground's slope past the floor, steepening over 6 m and joining
    /// the ground over a metre, 96 m out at most; the edge wandering by a third over 96 m; the
    /// carve gone under 1.5 m of water level and whole from 4 m; 0.5 m over a lake's level kept
    /// within reach; the width changing by half a metre a metre at most.
    fn default() -> Self {
        Self {
            reaches: (0.02, 0.04),
            slope_run: 40.0,
            room: 2.0,
            bench: 1.5,
            floodplain: (3.0, 12.0, 64.0),
            deepest: 6.0,
            rise: (0.3, 0.5),
            fall: 0.02,
            wall: (0.15, 1.3),
            ease: 6.0,
            join: 1.0,
            reach: 96.0,
            wander: (0.33, 96.0),
            seed: 0x07a1_1e75,
            sea: (1.5, 4.0),
            lake_guard: 0.5,
            taper: 0.5,
        }
    }
}

/// What a carve did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ValleyStats {
    /// The ribbons' points by type (more than half their weight): a floodplain, a bench, the
    /// water's room only.
    pub floodplain: u32,
    /// See `floodplain`.
    pub bench: u32,
    /// See `floodplain`.
    pub room: u32,
    /// The points under a lake, where nothing is carved.
    pub in_lake: u32,
    /// The samples lowered by more than a centimetre.
    pub lowered: u32,
    /// The most a sample was lowered, metres.
    pub deepest_cut: f64,
    /// The samples the lakes' guard kept higher.
    pub guarded: u32,
    /// The floor's mean half width per side, metres: what the points got, and what their type
    /// asked for.
    pub mean_floor: (f64, f64),
}

/// A point's floor, per side (`0` to the left of the course seen downstream, `1` to its right).
#[derive(Clone, Copy, Debug, Default)]
struct Floor {
    at: [f64; 2],
    /// The floor's height on the course, metres.
    height: f64,
    /// The water's half width plus the room: where the floor's wander starts.
    room: f64,
    /// The floor's half width per side.
    half: [f64; 2],
    /// The wall's steepest per side, m/m.
    steepest: [f64; 2],
    /// How far from the course the carve may reach per side, metres (the edge's wander and the
    /// wall included).
    reach: [f64; 2],
    /// The carve's strength, 0..1 (gone towards the sea and under the lakes).
    strength: f64,
}

/// The wall's rise `x` metres past the floor's edge: steepening over `ease` metres to `steepest`.
fn wall(x: f64, steepest: f64, ease: f64) -> f64 {
    if x <= 0.0 {
        0.0
    } else if x < ease {
        steepest * x * x / (2.0 * ease)
    } else {
        steepest * (x - 0.5 * ease)
    }
}

/// Bins of the segments' lookup, metres.
const BIN: f64 = 64.0;

/// Carves the valleys of `ribbons` (made over `height`, with their levels) into `height`, whose
/// `lakes` keep their rims.
pub fn carve_valleys(
    height: &mut Field2<f32>,
    ribbons: &[Ribbon],
    lakes: &Lakes,
    params: &ValleyParams,
    pool: &TaskPool,
) -> ValleyStats {
    let mut stats = ValleyStats::default();
    let spacing = height.spacing;
    let last = f64::from(height.size - 1);
    let step = 0.25 * spacing;
    let in_lake = |p: [f64; 2]| {
        let (i, j) = (
            (p[0] / spacing).round().clamp(0.0, last) as u32,
            (p[1] / spacing).round().clamp(0.0, last) as u32,
        );
        lakes.lake_of[height.index(i, j)] != u32::MAX
    };
    let wander = params.wander.0;
    // Each ribbon's floors, then its segments between points outside the lakes.
    let mut floors: Vec<Floor> = Vec::new();
    let mut segments: Vec<[u32; 2]> = Vec::new();
    let (mut asked, mut got, mut sides) = (0.0, 0.0, 0.0);
    for r in ribbons {
        let p = &r.points;
        let n = p.len();
        if n < 2 {
            continue;
        }
        let f = |v: f32| f64::from(v);
        let at = |k: usize| [f(p[k].position[0]), f(p[k].position[1])];
        let ground = |k: usize, across: f64| {
            let q = offset(&p[k], 0.0, across);
            smooth_height(height, q[0], q[1])
        };
        let floor_at = |k: usize| f(p[k].unstepped) + params.rise.0 + params.rise.1 * f(p[k].depth);
        let mut arc = vec![0.0; n];
        for k in 1..n {
            let (a, b) = (at(k - 1), at(k));
            arc[k] = arc[k - 1] + (b[0] - a[0]).hypot(b[1] - a[1]);
        }
        let slopes = reach_slopes(p, params.slope_run);
        let mut half = [vec![0.0f64; n], vec![0.0f64; n]];
        let mut room = vec![0.0f64; n];
        for k in 0..n {
            let slope = slopes[k];
            let (r0, r1) = params.reaches;
            let plain = 1.0 - smoothstep(r0 - 0.005, r0 + 0.005, slope);
            let bench = 1.0 - smoothstep(r1 - 0.005, r1 + 0.005, slope);
            let water = f(p[k].half_width);
            let width = 2.0 * water;
            room[k] = water + params.room;
            let on_bench = water + params.bench * width;
            let (w, least, most) = params.floodplain;
            let on_plain = (w * width).clamp(least, most).max(on_bench);
            let asks = room[k] + (on_bench - room[k]) * bench + (on_plain - on_bench) * plain;
            if plain > 0.5 {
                stats.floodplain += 1;
            } else if bench > 0.5 {
                stats.bench += 1;
            } else {
                stats.room += 1;
            }
            // Each side's floor: out from the water's room until the ground stands `deepest`
            // over it.
            let floor = floor_at(k);
            for (s, sign) in [1.0, -1.0].into_iter().enumerate() {
                let mut reach = room[k];
                let mut d = room[k];
                while d <= asks {
                    if ground(k, sign * d) - (floor + params.fall * d) > params.deepest {
                        break;
                    }
                    reach = d;
                    d += step;
                }
                half[s][k] = reach;
                asked += asks;
                sides += 1.0;
            }
        }
        // The width changes gradually along the course.
        for side in &mut half {
            for k in 1..n {
                side[k] = side[k]
                    .min(side[k - 1] + params.taper * (arc[k] - arc[k - 1]))
                    .max(room[k]);
            }
            for k in (0..n - 1).rev() {
                side[k] = side[k]
                    .min(side[k + 1] + params.taper * (arc[k + 1] - arc[k]))
                    .max(room[k]);
            }
        }
        let first = floors.len() as u32;
        for k in 0..n {
            let floor = floor_at(k);
            let lake = in_lake(at(k));
            stats.in_lake += u32::from(lake);
            let (mut steepest, mut reach) = ([0.0; 2], [0.0; 2]);
            for (s, sign) in [1.0, -1.0].into_iter().enumerate() {
                // The wall: steeper than the ground past the floor's widest edge, out to where
                // it meets the ground.
                let widest = room[k] + (half[s][k] - room[k]) * (1.0 + wander);
                let beyond = (ground(k, sign * (widest + 16.0)) - ground(k, sign * widest)) / 16.0;
                steepest[s] = params.wall.0 + params.wall.1 * beyond.max(0.0);
                let edge = floor + params.fall * widest;
                let mut x = 0.0;
                while x < params.reach
                    && ground(k, sign * (widest + x)) > edge + wall(x, steepest[s], params.ease)
                {
                    x += step;
                }
                reach[s] = widest + x + params.join + step;
                got += half[s][k];
            }
            floors.push(Floor {
                at: at(k),
                height: floor,
                room: room[k],
                half: [half[0][k], half[1][k]],
                steepest,
                reach,
                strength: if lake {
                    0.0
                } else {
                    smoothstep(params.sea.0, params.sea.1, f(p[k].unstepped))
                },
            });
        }
        for k in 0..n as u32 - 1 {
            let (a, b) = (
                &floors[(first + k) as usize],
                &floors[(first + k + 1) as usize],
            );
            if a.strength > 0.0 || b.strength > 0.0 {
                segments.push([first + k, first + k + 1]);
            }
        }
    }
    if sides > 0.0 {
        stats.mean_floor = (got / sides, asked / sides);
    }
    // The segments each bin of `BIN` metres may reach.
    let size = height.size as usize;
    let bins = ((last * spacing) / BIN).floor() as usize + 1;
    let reach_of = |s: &[u32; 2]| {
        let (a, b) = (&floors[s[0] as usize], &floors[s[1] as usize]);
        a.reach[0].max(a.reach[1]).max(b.reach[0]).max(b.reach[1])
    };
    let bins_of = |s: &[u32; 2]| {
        let (a, b) = (floors[s[0] as usize].at, floors[s[1] as usize].at);
        let grow = reach_of(s);
        let lo = |v: f64| ((v - grow) / BIN).floor().clamp(0.0, (bins - 1) as f64) as usize;
        let hi = |v: f64| ((v + grow) / BIN).floor().clamp(0.0, (bins - 1) as f64) as usize;
        (
            lo(a[0].min(b[0]))..=hi(a[0].max(b[0])),
            lo(a[1].min(b[1]))..=hi(a[1].max(b[1])),
        )
    };
    let mut start = vec![0u32; bins * bins + 1];
    for s in &segments {
        let (xs, ys) = bins_of(s);
        for y in ys {
            for x in xs.clone() {
                start[y * bins + x + 1] += 1;
            }
        }
    }
    for c in 0..bins * bins {
        start[c + 1] += start[c];
    }
    let mut fill = start.clone();
    let mut list = vec![0u32; start[bins * bins] as usize];
    for (index, s) in segments.iter().enumerate() {
        let (xs, ys) = bins_of(s);
        for y in ys {
            for x in xs.clone() {
                let c = y * bins + x;
                list[fill[c] as usize] = index as u32;
                fill[c] += 1;
            }
        }
    }
    // The lakes' guard: the highest lake level plus `lake_guard` within the farthest reach.
    let farthest = segments.iter().map(reach_of).fold(0.0, f64::max);
    let radius = (farthest / spacing).ceil() as usize + 1;
    let mut guard: Vec<f32> = lakes
        .lake_of
        .iter()
        .map(|&l| {
            if l == u32::MAX {
                f32::NEG_INFINITY
            } else {
                lakes.lakes[l as usize].level + params.lake_guard as f32
            }
        })
        .collect();
    dilate(&mut guard, size, radius);
    // Each sample's lowering: the most any segment asks for.
    let source = &height.data;
    let join = params.join;
    let lowering = |index: usize| -> (f32, bool) {
        let (i, j) = (index % size, index / size);
        let q = [i as f64 * spacing, j as f64 * spacing];
        let (bx, by) = (
            ((q[0] / BIN) as usize).min(bins - 1),
            ((q[1] / BIN) as usize).min(bins - 1),
        );
        let c = by * bins + bx;
        let here = f64::from(source[index]);
        let mut most = 0.0f64;
        let mut noise = None;
        for &s in &list[start[c] as usize..start[c + 1] as usize] {
            let [ia, ib] = segments[s as usize];
            let (a, b) = (&floors[ia as usize], &floors[ib as usize]);
            let (r, t) = segment_distance(q, a.at, b.at);
            let left =
                (b.at[0] - a.at[0]) * (q[1] - a.at[1]) - (b.at[1] - a.at[1]) * (q[0] - a.at[0]);
            let side = usize::from(left < 0.0);
            let mix = |u: f64, v: f64| u + (v - u) * t;
            if r >= mix(a.reach[side], b.reach[side]) {
                continue;
            }
            let wobble = *noise.get_or_insert_with(|| {
                let w = params.wander.1;
                fbm(params.seed, q[0] / w, q[1] / w, 2, 2.0, 0.5)
            });
            let room = mix(a.room, b.room);
            let half = room + (mix(a.half[side], b.half[side]) - room) * (1.0 + wander * wobble);
            let floor = mix(a.height, b.height) + params.fall * r.min(half);
            let carved = if r <= half {
                here.min(floor)
            } else {
                // A smooth minimum where the wall meets the ground.
                let rise = floor
                    + wall(
                        r - half,
                        mix(a.steepest[side], b.steepest[side]),
                        params.ease,
                    );
                let gap = (here - rise).abs();
                here.min(rise)
                    - if gap < join {
                        (join - gap) * (join - gap) / (4.0 * join)
                    } else {
                        0.0
                    }
            };
            most = most.max((here - carved) * mix(a.strength, b.strength));
        }
        let carved = here - most;
        let g = f64::from(guard[index]);
        if carved < g && here > carved {
            (here.min(g) as f32, true)
        } else {
            (carved as f32, false)
        }
    };
    let mut out = vec![(0.0f32, false); size * size];
    pool.par_map_into(&mut out, 4096, lowering);
    for (index, &(h, guarded)) in out.iter().enumerate() {
        let before = height.data[index];
        stats.guarded += u32::from(guarded);
        if before - h > 0.01 {
            stats.lowered += 1;
            stats.deepest_cut = stats.deepest_cut.max(f64::from(before - h));
        }
        height.data[index] = h;
    }
    stats
}

/// The water's fall at each point of `points`, m/m: over `run` metres of the course either way
/// (fewer at its ends).
fn reach_slopes(points: &[RibbonPoint], run: f64) -> Vec<f64> {
    let n = points.len();
    let f = |v: f32| f64::from(v);
    let mut arc = vec![0.0; n];
    for k in 1..n {
        let (a, b) = (points[k - 1].position, points[k].position);
        arc[k] = arc[k - 1] + f(b[0] - a[0]).hypot(f(b[1] - a[1]));
    }
    let (mut lo, mut hi) = (0, 0);
    (0..n)
        .map(|k| {
            while arc[k] - arc[lo] > run {
                lo += 1;
            }
            while hi + 1 < n && arc[hi + 1] - arc[k] <= run {
                hi += 1;
            }
            let length = arc[hi] - arc[lo];
            if length > 0.0 {
                ((f(points[lo].unstepped) - f(points[hi].unstepped)) / length).max(0.0)
            } else {
                0.0
            }
        })
        .collect()
}

/// How the steeper rivers' valleys are painted (#118): their beds of gravel, scree at the foot of
/// their walls, and scrub on the walls above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValleyGround {
    /// The layer of the beds.
    pub gravel: u8,
    /// The layer at the walls' foot.
    pub scree: u8,
    /// The layer of the plants on the walls.
    pub scrub: u8,
    /// The layer the scrub takes: the bare rock.
    pub rock: u8,
    /// The water's slopes, m/m, over which a reach is painted, from none of it to all of it (the
    /// threshold wanders with noise in between).
    pub steep: (f64, f64),
    /// Over how many metres of the course, either way, a point's slope is measured.
    pub slope_run: f64,
    /// The gravel's metres past the water's edge, `a + b ×` noise.
    pub bed: (f64, f64),
    /// The scree's metres over the water, `a + b ×` noise.
    pub scree_rise: (f64, f64),
    /// The ground's slope, m/m, over which it takes scree (gentler ground keeps its own).
    pub scree_slope: f32,
    /// The scrub's metres over the water, `a + b ×` noise.
    pub scrub_rise: (f64, f64),
    /// The ground's slope, m/m, under which the rock takes scrub (cliffs stay bare).
    pub scrub_slope: f32,
    /// The noise (about −1 to 1) under which the walls stay bare between the scrub's patches.
    pub scrub_gaps: f64,
    /// The most metres past the water's edge the valley's ground reaches.
    pub reach: f64,
    /// Metres over which the noise wanders.
    pub patch: f64,
    /// The noise's seed.
    pub seed: u64,
}

impl Default for ValleyGround {
    /// Painted from a 2.5 % fall, all of it from 4 % (D-041's steps and pools), the slope over
    /// 40 m either way; gravel 2.5 m ± 1.5 m past the water; scree to 5 m ± 3 m over it where the
    /// ground is steeper than 0.55 (29°); scrub on the rock under 1.4 (54°) to 30 m ± 12 m over
    /// it, bare where the noise is under −0.25; 80 m out at most; the noise over 24 m.
    fn default() -> Self {
        Self {
            gravel: 0,
            scree: 0,
            scrub: 0,
            rock: 0,
            steep: (0.025, 0.04),
            slope_run: 40.0,
            bed: (2.5, 1.5),
            scree_rise: (5.0, 3.0),
            scree_slope: 0.55,
            scrub_rise: (30.0, 12.0),
            scrub_slope: 1.4,
            scrub_gaps: -0.25,
            reach: 80.0,
            patch: 24.0,
            seed: 0x5c2e_e118,
        }
    }
}

/// What [`paint_valley_ground`] painted: texels of gravel, scree and scrub.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValleyPainted {
    /// Gravel.
    pub gravel: usize,
    /// Scree.
    pub scree: usize,
    /// Scrub.
    pub scrub: usize,
}

/// Paints the steeper reaches' valleys into `layers` (over `height`'s square). Each texel within
/// `reach` of a steep river's water is painted from its nearest one: the texels of the layers
/// of `from` within the gravel's reach of the water become its bed, those on the walls' steep
/// foot up to the scree's rise over the water scree, and the rock above them, up to the scrub's
/// rise and under its slope, scrub in patches.
pub fn paint_valley_ground(
    layers: &mut Field2<u8>,
    height: &Field2<f32>,
    ribbons: &[Ribbon],
    from: &[u8],
    ground: &ValleyGround,
) -> ValleyPainted {
    let cell = layers.spacing;
    let last = i64::from(layers.size) - 1;
    // Per texel within reach: how far out of the nearest steep river's water it is, that
    // water's level there, and how steep its reach is.
    let mut near: std::collections::HashMap<u32, [f64; 3]> = std::collections::HashMap::new();
    for ribbon in ribbons {
        let p = &ribbon.points;
        let slopes = reach_slopes(p, ground.slope_run);
        for k in 0..p.len().saturating_sub(1) {
            let steep = smoothstep(ground.steep.0, ground.steep.1, slopes[k].max(slopes[k + 1]));
            if steep == 0.0 {
                continue;
            }
            let f = |v: f32| f64::from(v);
            let (a, b) = (
                [f(p[k].position[0]), f(p[k].position[1])],
                [f(p[k + 1].position[0]), f(p[k + 1].position[1])],
            );
            let grow = f(p[k].half_width.max(p[k + 1].half_width)) + ground.reach;
            let lo = |v: f64| (((v - grow) / cell).floor() as i64).clamp(0, last);
            let hi = |v: f64| (((v + grow) / cell).ceil() as i64).clamp(0, last);
            for ty in lo(a[1].min(b[1]))..=hi(a[1].max(b[1])) {
                for tx in lo(a[0].min(b[0]))..=hi(a[0].max(b[0])) {
                    let q = [(tx as f64 + 0.5) * cell, (ty as f64 + 0.5) * cell];
                    let (r, t) = segment_distance(q, a, b);
                    let mix = |u: f32, v: f32| f(u) + f(v - u) * t;
                    let out = r - mix(p[k].half_width, p[k + 1].half_width);
                    if out > ground.reach {
                        continue;
                    }
                    let index = ty as u32 * layers.size + tx as u32;
                    let entry = near.entry(index).or_insert([f64::MAX, 0.0, 0.0]);
                    if out < entry[0] {
                        *entry = [out, mix(p[k].unstepped, p[k + 1].unstepped), steep];
                    }
                }
            }
        }
    }
    let noise = |q: [f64; 2], salt: u64| {
        fbm(
            ground.seed ^ salt,
            q[0] / ground.patch,
            q[1] / ground.patch,
            2,
            2.0,
            0.5,
        )
    };
    // The ground's slope at a point, from the field's nearest sample.
    let top = f64::from(height.size - 1);
    let slope_at = |q: [f64; 2]| {
        let (x, y) = (
            (q[0] / height.spacing).round().clamp(0.0, top) as u32,
            (q[1] / height.spacing).round().clamp(0.0, top) as u32,
        );
        let (gx, gy) = height.gradient(x, y);
        (gx * gx + gy * gy).sqrt()
    };
    let mut painted = ValleyPainted::default();
    for (&index, &[out, level, steep]) in &near {
        let texel = layers.data[index as usize];
        let (tx, ty) = (index % layers.size, index / layers.size);
        let q = [(f64::from(tx) + 0.5) * cell, (f64::from(ty) + 0.5) * cell];
        // Where the reach is only partly steep, its share of the texels.
        let wobble = noise(q, 0);
        if steep < 0.5 + 0.5 * wobble {
            continue;
        }
        let above = f64::from(height.sample(q[0], q[1])) - level;
        let slope = slope_at(q);
        if from.contains(&texel) && out <= ground.bed.0 + ground.bed.1 * wobble {
            layers.data[index as usize] = ground.gravel;
            painted.gravel += 1;
        } else if from.contains(&texel)
            && above < ground.scree_rise.0 + ground.scree_rise.1 * wobble
            && slope > ground.scree_slope
        {
            layers.data[index as usize] = ground.scree;
            painted.scree += 1;
        } else if texel == ground.rock
            && slope < ground.scrub_slope
            && above < ground.scrub_rise.0 + ground.scrub_rise.1 * noise(q, 0x5c2b)
            && noise(q, 0xc0e2) > ground.scrub_gaps
        {
            layers.data[index as usize] = ground.scrub;
            painted.scrub += 1;
        }
    }
    painted
}

/// Each value of the `size × size` grid `values` replaced by the largest within `radius`
/// samples along the rows and the columns (a square), by a running maximum.
fn dilate(values: &mut [f32], size: usize, radius: usize) {
    let mut line = vec![0.0f32; size];
    let mut window = std::collections::VecDeque::new();
    let mut pass = |values: &mut [f32], at: &dyn Fn(usize, usize) -> usize| {
        for row in 0..size {
            for (k, v) in line.iter_mut().enumerate() {
                *v = values[at(row, k)];
            }
            window.clear();
            // The running maximum over [k − radius, k + radius]: the window's indices, their
            // values decreasing.
            let mut next = 0;
            for k in 0..size {
                while next < size && next <= k + radius {
                    while window
                        .back()
                        .is_some_and(|&b: &usize| line[b] <= line[next])
                    {
                        window.pop_back();
                    }
                    window.push_back(next);
                    next += 1;
                }
                while window.front().is_some_and(|&f| f + radius < k) {
                    window.pop_front();
                }
                values[at(row, k)] = line[*window.front().expect("the window holds k")];
            }
        }
    };
    pass(values, &|row, k| row * size + k);
    pass(values, &|row, k| k * size + row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::river::{ACROSS, RibbonPoint};
    use forge_task::{PoolConfig, TaskPool};

    /// A valley along x at y = 160 m whose walls rise `wall` m/m from its floor, falling 1 % to
    /// the east from 20 m, on a field of 4 m samples; and a river down it, 10 m wide, 0.2 m
    /// under the floor and 0.6 m deep.
    fn valley(wall: f64) -> (Field2<f32>, Ribbon) {
        falling(wall, 0.01)
    }

    /// [`valley`] falling `fall` m/m instead, to the same height at its east end.
    fn falling(wall: f64, fall: f64) -> (Field2<f32>, Ribbon) {
        let top = 20.0 + 320.0 * (fall - 0.01);
        let height = Field2::from_fn(81, 4.0, |x, y| {
            let (x, y) = (f64::from(x) * 4.0, f64::from(y) * 4.0);
            (top - fall * x + wall * (y - 160.0).abs()) as f32
        });
        let points = (0..=80)
            .map(|k| {
                let x = f64::from(k) * 4.0;
                RibbonPoint {
                    position: [x as f32, 160.0],
                    level: (top - fall * x - 0.2) as f32,
                    direction: [1.0, 0.0],
                    half_width: 5.0,
                    cover: 5.0,
                    reach: 6.0,
                    depth: 0.6,
                    bank: 0.0,
                    speed: 1.0,
                    slope: fall as f32,
                    grade: fall as f32,
                    unstepped: (top - fall * x - 0.2) as f32,
                    foam: 0.0,
                    key: k,
                    lip: [0.0; 2],
                    fade: 1.0,
                    ground: [0.0; ACROSS + 1],
                }
            })
            .collect();
        let ribbon = Ribbon {
            river: 0,
            mouth_area: 1e6,
            points,
            lake_runs: Vec::new(),
            corners: Vec::new(),
            steps: Vec::new(),
            deltas: Vec::new(),
            outlets: Vec::new(),
            bars: Vec::new(),
        };
        (height, ribbon)
    }

    fn no_lakes(height: &Field2<f32>) -> Lakes {
        Lakes {
            lakes: Vec::new(),
            lake_of: vec![u32::MAX; height.len()],
        }
    }

    fn steady() -> ValleyParams {
        ValleyParams {
            wander: (0.0, 96.0),
            ..ValleyParams::default()
        }
    }

    /// The ground across the valley at x = 160 m, from the course out `d` samples north.
    fn across(field: &Field2<f32>, d: u32) -> f64 {
        f64::from(field.get(40, 40 + d))
    }

    #[test]
    fn an_open_valley_gets_a_floodplain_and_a_deep_one_room_for_its_water() {
        let pool = TaskPool::new(PoolConfig::with_workers(2));
        // Walls rising 10 %: the gentle river (1 %) gets its floodplain, 30 m either side.
        let (mut open, ribbon) = valley(0.1);
        let before = open.clone();
        let lakes = no_lakes(&open);
        let stats = carve_valleys(&mut open, &[ribbon], &lakes, &steady(), &pool);
        assert!(stats.floodplain > 70, "{stats:?}");
        // The water at 18.2 m there, the floor 0.6 m over it and 2 % up towards 30 m out from the
        // course; downstream the course falls 1 %, so the lowest is `r √(0.02² − 0.01²)` up.
        for d in [3, 5, 7] {
            let expected = 18.2 + 0.6 + f64::from(d) * 4.0 * (0.02f64 * 0.02 - 0.0001).sqrt();
            assert!(
                (across(&open, d) - expected).abs() < 0.01,
                "{d}: {} for {expected}",
                across(&open, d)
            );
        }
        // Past the floor the wall comes back to the ground, and nothing is ever raised.
        assert!(across(&open, 10) < across(&before, 10) && across(&open, 10) > across(&open, 7));
        assert_eq!(across(&open, 20), across(&before, 20));
        assert!(open.data.iter().zip(&before.data).all(|(a, b)| a <= b));
        // Walls rising 100 %: the ground stands 6 m over the floor 7 m out, so the floor stops at
        // the water's room, and the wall rises no steeper than 1.45 to meet the ground.
        let (mut deep, ribbon) = valley(1.0);
        let before = deep.clone();
        let lakes = no_lakes(&deep);
        let stats = carve_valleys(&mut deep, &[ribbon], &lakes, &steady(), &pool);
        assert!(stats.mean_floor.0 < 7.5, "{stats:?}");
        assert!((across(&deep, 1) - (18.8 + 0.08)).abs() < 0.01);
        for d in 1..12 {
            let rise = (across(&deep, d + 1) - across(&deep, d)) / 4.0;
            assert!(rise < 1.46, "{d}: {rise}");
        }
        assert_eq!(across(&deep, 9), across(&before, 9));
    }

    #[test]
    fn lakes_keep_their_rims_and_mouths_their_beaches() {
        let pool = TaskPool::new(PoolConfig::with_workers(2));
        // A lake at 20 m over the samples x ≤ 40 m: no ground within reach goes under 20.5 m.
        let (mut field, ribbon) = valley(0.1);
        let mut lakes = no_lakes(&field);
        lakes.lakes.push(crate::hydrology::Lake {
            level: 20.0,
            cells: Vec::new(),
            depth: 1.0,
            outlet: 0,
        });
        for y in 30..50 {
            for x in 0..=10 {
                lakes.lake_of[field.index(x, y)] = 0;
            }
        }
        let before = field.clone();
        let stats = carve_valleys(&mut field, &[ribbon], &lakes, &steady(), &pool);
        assert!(stats.guarded > 0 && stats.in_lake > 0, "{stats:?}");
        for y in 0..81 {
            for x in 0..=20 {
                let (h, was) = (field.get(x, y), before.get(x, y));
                assert!(h >= was.min(20.5), "({x}, {y}): {h} from {was}");
            }
        }
        // The same river under 1.5 m over the sea: nothing is carved.
        let (mut field, mut ribbon) = valley(0.1);
        for p in &mut ribbon.points {
            p.level -= 18.5;
            p.unstepped -= 18.5;
        }
        let lakes = no_lakes(&field);
        let stats = carve_valleys(&mut field, &[ribbon], &lakes, &steady(), &pool);
        assert_eq!(stats.lowered, 0, "{stats:?}");
    }

    #[test]
    fn a_steep_valley_gets_gravel_scree_and_scrub_and_a_gentle_one_none() {
        const GRASS: u8 = 0;
        const ROCK: u8 = 1;
        let ground = ValleyGround {
            gravel: 2,
            scree: 3,
            scrub: 4,
            rock: ROCK,
            ..ValleyGround::default()
        };
        // Texels of 4 m over the field: rock where the walls rise (all of them at 0.8), grass on
        // the floor under 5 m out.
        let layers = || {
            Field2::from_fn(80, 4.0, |_, y| {
                let out = ((f64::from(y) + 0.5) * 4.0 - 160.0).abs();
                if out < 5.0 { GRASS } else { ROCK }
            })
        };
        // A river falling 6 % on a floor 7 m out and 0.6 m over the course (as `carve_valleys`
        // leaves a steep reach): its bed is gravel, the walls' foot scree, the rock above scrub.
        let (_, ribbon) = falling(0.8, 0.06);
        let over = |out: f64| 0.8 + 0.8 * (out - 2.0).max(0.0);
        let field = Field2::from_fn(81, 4.0, |x, y| {
            let out = (f64::from(y) * 4.0 - 160.0).abs() - 5.0;
            (36.0 - 0.06 * f64::from(x) * 4.0 - 0.2 + over(out)) as f32
        });
        let mut steep = layers();
        let before = steep.clone();
        let painted = paint_valley_ground(&mut steep, &field, &[ribbon], &[GRASS, ROCK], &ground);
        assert!(
            painted.gravel > 0 && painted.scree > 0 && painted.scrub > 0,
            "{painted:?}"
        );
        for (i, (&now, &was)) in steep.data.iter().zip(&before.data).enumerate() {
            let y = (i / 80) as f64;
            let out = ((y + 0.5) * 4.0 - 160.0).abs() - 5.0;
            let above = over(out);
            match now {
                2 => assert!(out <= 4.0, "gravel {out} m out"),
                3 => assert!(above < 8.0 + 0.5, "scree {above} m over the water"),
                4 => assert!(
                    was == ROCK && above < 42.0 + 0.5,
                    "scrub {above} m over the water"
                ),
                _ => assert_eq!(now, was),
            }
        }
        // A river falling 1 %: nothing.
        let (field, ribbon) = valley(0.8);
        let mut gentle = layers();
        let before = gentle.clone();
        let painted = paint_valley_ground(&mut gentle, &field, &[ribbon], &[GRASS, ROCK], &ground);
        assert_eq!(painted, ValleyPainted::default());
        assert_eq!(gentle, before);
    }
}
