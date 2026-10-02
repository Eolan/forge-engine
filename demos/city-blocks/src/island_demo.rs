//! The `island` demo's golden shots and its tour (#96's step 3, `docs/demos/island.md`, "The
//! island demo"). Both are found in the island's own features, so they hold for any seed: the
//! largest mouth with bars, the highest of the largest lakes, the steepest river, the island
//! seen from the sea. `--shot NAME` frames one at its time of day; `--tour` flies through them.

use super::{Args, Field2, FlyCamera, IslandWater, Vec3, island_heights, island_water};

/// One of the island's golden shots: where the camera stands, where it looks, and when.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shot {
    /// Its name for `--shot`.
    pub name: &'static str,
    /// The time of day it is taken at (`--time-of-day`: 0 sunrise, 0.5 noon, 1 sunset).
    pub time: f32,
    /// The camera, metres in the scene's frame.
    pub position: Vec3,
    /// Where it looks, unit.
    pub forward: Vec3,
}

impl Shot {
    /// `--view`'s numbers for it: x, y, z, then the yaw and the pitch in degrees.
    pub fn view(&self) -> [f32; 5] {
        let (yaw, pitch) = yaw_pitch(self.forward);
        [
            self.position.x,
            self.position.y,
            self.position.z,
            yaw.to_degrees(),
            pitch.to_degrees(),
        ]
    }
}

/// The yaw and the pitch that turn [`FlyCamera`] to look along `forward`.
fn yaw_pitch(forward: Vec3) -> (f32, f32) {
    let f = forward.normalize_or(Vec3::NEG_Z);
    ((-f.x).atan2(-f.z), f.y.clamp(-1.0, 1.0).asin())
}

/// The way along the ground, unit, at `yaw` and `pitch` (radians).
fn forward_of(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        -yaw.sin() * pitch.cos(),
        pitch.sin(),
        -yaw.cos() * pitch.cos(),
    )
}

/// The island's data the shots are found in.
struct Island {
    height: Field2<f32>,
    water: IslandWater,
    /// Half the field's extent: the field's frame less this is the scene's.
    half: f32,
}

impl Island {
    fn new(args: &Args) -> Self {
        let height = island_heights(args);
        let water = island_water(&height);
        let half = (height.extent() * 0.5) as f32;
        Self {
            height,
            water,
            half,
        }
    }

    /// The ground as drawn at (x, z) in the scene's frame, metres.
    fn ground(&self, x: f32, z: f32) -> f32 {
        self.water.channels.height_at(
            &self.height,
            f64::from(x + self.half),
            f64::from(z + self.half),
        ) as f32
    }

    /// A ribbon point's position in the scene's frame and its way downstream.
    fn at(&self, p: &forge_procgen::RibbonPoint) -> (Vec3, Vec3) {
        (
            Vec3::new(
                p.position[0] - self.half,
                p.level,
                p.position[1] - self.half,
            ),
            Vec3::new(p.direction[0], 0.0, p.direction[1]),
        )
    }

    /// Dawn over the largest mouth with bars (#127), the sun rising out of the sea beyond it:
    /// 140 m up the river from the bars' middle, 60 m over the water, looking down the river to
    /// the sea. Without bars, the largest river's mouth.
    fn mouth(&self) -> Option<Shot> {
        let ribbons = &self.water.ribbons;
        let (back, down, level) = match ribbons.iter().rev().find(|r| !r.bars.is_empty()) {
            Some(r) => {
                let n = r.bars.len() as f64;
                let mid = r.bars.iter().fold([0.0; 2], |m, b| {
                    [m[0] + b.centre[0] / n, m[1] + b.centre[1] / n]
                });
                let b = r.bars[0];
                let length = r.bars.iter().fold(0.0_f64, |m, b| m.max(2.0 * b.half[0]));
                let down = Vec3::new(b.down[0] as f32, 0.0, b.down[1] as f32);
                let middle = Vec3::new(mid[0] as f32 - self.half, 0.0, mid[1] as f32 - self.half);
                let back = middle - down * (0.5 * length as f32 + 140.0);
                (back, down, b.level[0] as f32)
            }
            None => {
                let r = ribbons.last()?;
                let (at, down) = self.at(&r.points[forge_procgen::sea_mouth(&r.points)?]);
                (at - down * 140.0, down, at.y)
            }
        };
        Some(Shot {
            name: "mouth",
            time: 0.08,
            position: Vec3::new(back.x, level + 60.0, back.z),
            forward: forward_of(yaw_pitch(down).0, (-12.0_f32).to_radians()),
        })
    }

    /// The morning over the highest of the three largest lakes: 30 m over its water, four
    /// samples past its south edge, looking north across it.
    fn lake(&self) -> Option<Shot> {
        let spacing = self.height.spacing as f32;
        let mut largest: Vec<&forge_procgen::LakeWater> = self.water.lakes.iter().collect();
        largest.sort_by_key(|l| std::cmp::Reverse(l.mask.iter().filter(|&&m| m).count()));
        let lake = largest
            .iter()
            .take(3)
            .max_by(|a, b| a.level.total_cmp(&b.level))?;
        let x = (lake.first[0] as f32 + 0.5 * lake.size[0] as f32) * spacing - self.half;
        let z = (lake.first[1] + lake.size[1] + 4) as f32 * spacing - self.half;
        Some(Shot {
            name: "lake",
            time: 0.3,
            position: Vec3::new(x, lake.level + 30.0, z),
            forward: forward_of(0.0, (-15.0_f32).to_radians()),
        })
    }

    /// The afternoon from the sea: the whole island, from 1.75 km off its southern beach (found
    /// as `island_camera` finds it) and 300 m up.
    fn island(&self) -> Shot {
        let n = self.height.size;
        let beach = (0..n)
            .rev()
            .find(|&j| self.height.get(n / 2, j) > 1.0)
            .map_or(0.0, |j| j as f32 * self.height.spacing as f32 - self.half);
        Shot {
            name: "island",
            time: 0.7,
            position: Vec3::new(0.0, 300.0, beach + 1750.0),
            forward: forward_of(0.0, (-5.0_f32).to_radians()),
        }
    }

    /// Dusk up a steep valley, the sun setting at its head: 40 m down the river from its
    /// steepest point (of a river 5 m wide or more), 2 m over the ground, looking up its steps
    /// and pools (#122).
    fn valley(&self) -> Option<Shot> {
        let p = self
            .water
            .ribbons
            .iter()
            .flat_map(|r| r.points.iter())
            .filter(|p| p.half_width >= 2.5 && p.fade > 0.99)
            .max_by(|a, b| a.grade.total_cmp(&b.grade))?;
        let (at, down) = self.at(p);
        let position = at + down * 40.0;
        Some(Shot {
            name: "valley",
            time: 0.92,
            position: Vec3::new(
                position.x,
                self.ground(position.x, position.z) + 2.0,
                position.z,
            ),
            forward: forward_of(yaw_pitch(-down).0, 4.0_f32.to_radians()),
        })
    }

    /// The shots, in the order of the day.
    fn shots(&self) -> Vec<Shot> {
        [
            self.mouth(),
            self.lake(),
            Some(self.island()),
            self.valley(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// The island's golden shots for `args` (logged at start).
pub(crate) fn island_shots(args: &Args) -> Vec<Shot> {
    Island::new(args).shots()
}

/// `--shot NAME`: its view in `--view` and its time in `--time-of-day`, unless given.
pub(crate) fn with_shot(mut args: Args) -> anyhow::Result<Args> {
    let Some(name) = args.shot.clone() else {
        return Ok(args);
    };
    anyhow::ensure!(
        args.island.is_some(),
        "--shot frames the island: add --island SEED"
    );
    let shots = island_shots(&args);
    let shot = shots.iter().find(|s| s.name == name).ok_or_else(|| {
        let names: Vec<&str> = shots.iter().map(|s| s.name).collect();
        anyhow::anyhow!("no shot {name:?}: the island's are {}", names.join(", "))
    })?;
    args.view.get_or_insert_with(|| shot.view().to_vec());
    args.time_of_day.get_or_insert(shot.time);
    Ok(args)
}

/// Samples a second of the tour's precomputed path.
const TOUR_RATE: f32 = 30.0;
/// Seconds the tour holds each shot.
const TOUR_HOLD: f32 = 2.5;
/// Metres the tour keeps over the highest ground within [`TOUR_REACH`] between its shots.
const TOUR_CLEARANCE: f32 = 30.0;
/// Metres around the path the clearance looks at.
const TOUR_REACH: f32 = 60.0;

/// `--tour`: a flight through the island's shots, from the steep valley up to the lake, across
/// the hills to the largest mouth, down its river out over the sea, and round to the island.
/// The path is a Catmull-Rom spline through the shots, eased to rest at each, lifted over the
/// ground between them; the camera looks along its way between the shots and turns to each
/// shot's view as it comes to rest. Precomputed, so a frame's camera depends only on its time.
pub(crate) struct Tour {
    /// The camera's position and way at [`TOUR_RATE`] samples a second.
    samples: Vec<(Vec3, Vec3)>,
}

impl Tour {
    pub(crate) fn new(args: &Args) -> anyhow::Result<Self> {
        anyhow::ensure!(
            args.island.is_some(),
            "--tour flies over the island: add --island SEED"
        );
        let island = Island::new(args);
        let mut keys: Vec<Shot> = [island.valley(), island.lake(), island.mouth()]
            .into_iter()
            .flatten()
            .collect();
        anyhow::ensure!(keys.len() >= 2, "the island has too few shots for a tour");
        // The farewell: 1.5 km on out over the sea past the last shot, 220 m up, looking back at
        // the island's middle.
        let last = *keys.last().expect("a shot");
        let out = Vec3::new(last.forward.x, 0.0, last.forward.z).normalize_or(Vec3::Z);
        let far = Vec3::new(last.position.x, 220.0, last.position.z) + out * 1500.0;
        keys.push(Shot {
            name: "farewell",
            time: last.time,
            position: far,
            forward: (Vec3::new(0.0, 60.0, 0.0) - far).normalize(),
        });
        Ok(Self::through(&keys, |x, z| island.ground(x, z)))
    }

    /// The tour through `keys`, over `ground` (x, z in the scene's frame).
    fn through(keys: &[Shot], ground: impl Fn(f32, f32) -> f32) -> Self {
        let point = |k: isize| {
            let k = k.clamp(0, keys.len() as isize - 1) as usize;
            keys[k].position
        };
        // Per segment: its duration (longer ones flown faster, at most a minute), then the
        // samples of the hold before it and of the flight.
        let mut path: Vec<(Vec3, f32, usize)> = Vec::new(); // position, the segment's share, from
        for s in 0..keys.len() - 1 {
            let length = keys[s].position.distance(keys[s + 1].position);
            let seconds = (4.0 + 0.35 * length.sqrt()).min(60.0);
            for _ in 0..(TOUR_HOLD * TOUR_RATE) as usize {
                path.push((keys[s].position, 0.0, s));
            }
            let n = (seconds * TOUR_RATE) as usize;
            let k = s as isize;
            for i in 0..n {
                let t = i as f32 / n as f32;
                // Smootherstep: at rest at both shots.
                let u = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
                let p = catmull_rom(point(k - 1), point(k), point(k + 1), point(k + 2), u);
                path.push((p, u, s));
            }
        }
        let end = keys.len() - 1;
        for _ in 0..(TOUR_HOLD * TOUR_RATE) as usize {
            path.push((keys[end].position, 1.0, end - 1));
        }
        // How far each sample must rise: over the highest ground near it, by the clearance in
        // the middle of a segment and never under 2 m over the ground under it; at the shots,
        // where the camera stands where they put it, not at all.
        let need: Vec<f32> = path
            .iter()
            .map(|&(p, u, _)| {
                let mut highest = ground(p.x, p.z);
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    highest = highest.max(ground(
                        p.x + TOUR_REACH * a.cos(),
                        p.z + TOUR_REACH * a.sin(),
                    ));
                }
                let middle = (std::f32::consts::PI * u).sin();
                ((highest + TOUR_CLEARANCE * middle) - p.y)
                    .max(ground(p.x, p.z) + 2.0 - p.y)
                    .max(0.0)
            })
            .collect();
        // Smoothed so the camera rises ahead of a hill and sinks after it: the most over 3 s
        // either way, then its mean over 1.5 s either way (still at least each sample's need),
        // faded out at the shots.
        let window = |w: f32| (w * TOUR_RATE) as isize;
        let most = sliding(&need, window(3.0), f32::max);
        let lift = sliding_mean(&most, window(1.5));
        let positions: Vec<Vec3> = path
            .iter()
            .zip(&lift)
            .map(|(&(p, u, _), &l)| {
                let at_shot = smoothstep(0.0, 0.15, u) * smoothstep(0.0, 0.15, 1.0 - u);
                p + Vec3::Y * l * at_shot
            })
            .collect();
        // The way: along the path between the shots, pitched a little down, turning to each
        // shot's own as it comes to rest there.
        let ways: Vec<Vec3> = (0..positions.len())
            .map(|i| {
                let (_, u, s) = path[i];
                let ahead =
                    positions[(i + 1).min(positions.len() - 1)] - positions[i.saturating_sub(1)];
                let (yaw, pitch) = yaw_pitch(ahead);
                let travel = forward_of(yaw, pitch.clamp(-0.35, 0.15) - 0.12);
                let (from, to) = (keys[s].forward, keys[(s + 1).min(end)].forward);
                let shot = if u < 0.5 { from } else { to };
                let w = 1.0 - smoothstep(0.0, 0.35, u.min(1.0 - u));
                if ahead.length() < 1e-3 {
                    shot
                } else {
                    travel.lerp(shot, w).normalize_or(shot)
                }
            })
            .collect();
        let ways = sliding_mean_vec(&ways, window(0.5));
        Self {
            samples: positions.into_iter().zip(ways).collect(),
        }
    }

    /// The tour's length, seconds.
    pub(crate) fn seconds(&self) -> f32 {
        self.samples.len() as f32 / TOUR_RATE
    }

    /// Puts `camera` where the tour is `time` seconds in, holding at its end.
    pub(crate) fn place(&self, camera: &mut FlyCamera, time: f32) {
        let x = (time * TOUR_RATE).clamp(0.0, (self.samples.len() - 1) as f32);
        let i = (x as usize).min(self.samples.len() - 2);
        let f = x - i as f32;
        let (a, b) = (self.samples[i], self.samples[i + 1]);
        camera.position = a.0.lerp(b.0, f);
        let (yaw, pitch) = yaw_pitch(a.1.lerp(b.1, f));
        camera.yaw = yaw;
        camera.pitch = pitch;
    }
}

/// The uniform Catmull-Rom spline through `b` and `c` at `t`, `a` and `d` its neighbours.
fn catmull_rom(a: Vec3, b: Vec3, c: Vec3, d: Vec3, t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * b)
        + (c - a) * t
        + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
        + (3.0 * b - a - 3.0 * c + d) * t3)
}

/// Hermite's smoothstep of `x` from `a` to `b`.
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `f` folded over each value's neighbours within `reach` either side.
fn sliding(values: &[f32], reach: isize, f: impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let n = values.len() as isize;
    (0..n)
        .map(|i| {
            ((i - reach).max(0)..=(i + reach).min(n - 1))
                .map(|j| values[j as usize])
                .fold(values[i as usize], &f)
        })
        .collect()
}

/// Each value's mean over its neighbours within `reach` either side.
fn sliding_mean(values: &[f32], reach: isize) -> Vec<f32> {
    let n = values.len() as isize;
    (0..n)
        .map(|i| {
            let (a, b) = ((i - reach).max(0), (i + reach).min(n - 1));
            (a..=b).map(|j| values[j as usize]).sum::<f32>() / (b - a + 1) as f32
        })
        .collect()
}

/// [`sliding_mean`] of unit vectors, normalised.
fn sliding_mean_vec(values: &[Vec3], reach: isize) -> Vec<Vec3> {
    let n = values.len() as isize;
    (0..n)
        .map(|i| {
            let (a, b) = ((i - reach).max(0), (i + reach).min(n - 1));
            (a..=b)
                .map(|j| values[j as usize])
                .sum::<Vec3>()
                .normalize_or(values[i as usize])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tour_rests_at_its_shots_and_keeps_over_the_ground() {
        // A ridge 200 m high across the way between two shots 2 km apart.
        let ridge = |x: f32, _z: f32| 200.0 * (1.0 - (x / 300.0).powi(2)).max(0.0);
        let shot = |x: f32, name| Shot {
            name,
            time: 0.5,
            position: Vec3::new(x, 20.0, 0.0),
            forward: Vec3::X,
        };
        let keys = [shot(-1000.0, "a"), shot(1000.0, "b")];
        let tour = Tour::through(&keys, ridge);
        let mut camera = FlyCamera::default();
        // At rest at each shot, looking its way.
        for (time, key) in [(0.0, &keys[0]), (tour.seconds(), &keys[1])] {
            tour.place(&mut camera, time);
            assert!(camera.position.distance(key.position) < 1e-3, "{camera:?}");
            assert!(camera.forward().distance(key.forward) < 1e-3);
        }
        // Over the ridge by the clearance, never under the ground.
        let mut over_ridge = f32::MIN;
        for i in 0..(tour.seconds() * 60.0) as usize {
            tour.place(&mut camera, i as f32 / 60.0);
            let p = camera.position;
            assert!(p.y >= ridge(p.x, p.z) + 1.9, "{p}");
            if p.x.abs() < 10.0 {
                over_ridge = over_ridge.max(p.y);
            }
        }
        assert!(over_ridge >= 200.0 + TOUR_CLEARANCE - 1.0, "{over_ridge}");
    }
}
