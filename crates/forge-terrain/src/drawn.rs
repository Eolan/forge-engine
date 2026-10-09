//! The island's ground as its tiles draw it (#106): the field drawn finer on its cubic, the
//! rivers' channels carved in it, and away from the water the amplification's detail.

use std::sync::Arc;
use std::time::Instant;

use forge_geom::city::HeightfieldDetail;
use forge_procgen::Field2;
use forge_task::TaskPool;

use crate::heights::island_heights;
use crate::keys::{derived_cache, drawn_factor, drawn_key};
use crate::water::{IslandWater, island_water};
use crate::world::world;

/// The island's ground as its tiles draw it (#106): the samples, and the cells drawn finer
/// (the rivers' channels, the lakes' shores and the coast's contours in quads of a metre).
pub struct DrawnGround {
    /// Samples a side.
    pub size: u32,
    /// Metres between them.
    pub spacing: f64,
    /// The samples, row by row.
    pub heights: Arc<[f32]>,
    /// The cells drawn finer and their heights.
    pub detail: Arc<HeightfieldDetail>,
}

impl DrawnGround {
    /// The ground at (x, y) metres in the field's frame as its coarse cells draw it: split along
    /// their (i + 1, j) – (i, j + 1) diagonal (`forge_procgen::drawn_height`).
    pub fn height_at(&self, x: f64, y: f64) -> f64 {
        let n = self.size as usize;
        let last = f64::from(self.size - 2);
        let (gx, gy) = (x / self.spacing, y / self.spacing);
        let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
        let (tx, ty) = ((gx - cx).clamp(0.0, 1.0), (gy - cy).clamp(0.0, 1.0));
        let at = |i: usize, j: usize| f64::from(self.heights[j * n + i]);
        let (i, j) = (cx as usize, cy as usize);
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        if tx + ty <= 1.0 {
            a + tx * (b - a) + ty * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - ty) * (b - d)
        }
    }

    /// [`Self::height_at`] with the refined cells too (#196): in one, its fine quads split
    /// along the same diagonal, as `forge_geom::city::refined_heightfield_mesh` draws them. (A
    /// coarse cell beside a refined one is a fan whose triangles are its own two.)
    pub fn surface_at(&self, x: f64, y: f64) -> f64 {
        let side = self.size - 1;
        let (gx, gy) = (x / self.spacing, y / self.spacing);
        let last = f64::from(side - 1);
        let (cx, cy) = (gx.floor().clamp(0.0, last), gy.floor().clamp(0.0, last));
        let cell = cy as u32 * side + cx as u32;
        let Ok(s) = self.detail.cells.binary_search(&cell) else {
            return self.height_at(x, y);
        };
        let k = self.detail.split.max(1) as usize;
        let row = k + 1;
        let heights = &self.detail.heights[s * row * row..(s + 1) * row * row];
        let span = k as f64;
        let (fx, fy) = (
            ((gx - cx) * span).clamp(0.0, span),
            ((gy - cy) * span).clamp(0.0, span),
        );
        let (u, v) = (fx.floor().min(span - 1.0), fy.floor().min(span - 1.0));
        let (tx, ty) = (fx - u, fy - v);
        let at = |u: usize, v: usize| f64::from(heights[v * row + u]);
        let (u, v) = (u as usize, v as usize);
        let (a, b, c, d) = (at(u, v), at(u + 1, v), at(u, v + 1), at(u + 1, v + 1));
        if tx + ty <= 1.0 {
            a + tx * (b - a) + ty * (c - a)
        } else {
            d + (1.0 - tx) * (c - d) + (1.0 - ty) * (b - d)
        }
    }
}

/// The island's field amplified `factor` times finer (stage 5): `forge_procgen::amplify` at
/// each halving of the spacing, over the drainage traced at that spacing.
/// The field [`amplify_ahead`] made, for [`island_amplified`] to take (#201): kept until then
/// only, it is 268 MB at 2 m.
static AMPLIFIED: std::sync::Mutex<Option<(u64, Field2<f32>)>> = std::sync::Mutex::new(None);

fn amplified_key(height: &Field2<f32>, factor: u32, seed: u64) -> u64 {
    height.digest()
        ^ u64::from(height.size)
        ^ u64::from(factor).rotate_left(40)
        ^ seed.rotate_left(20)
}

/// Takes the field [`amplify_ahead`] made for these arguments, or makes it.
fn island_amplified(height: &Field2<f32>, factor: u32, seed: u64, pool: &TaskPool) -> Field2<f32> {
    let key = amplified_key(height, factor, seed);
    let mut ready = AMPLIFIED.lock().expect("the amplified field");
    if ready.as_ref().is_some_and(|(made_for, _)| *made_for == key) {
        return ready.take().expect("the field made ahead").1;
    }
    drop(ready);
    make_island_amplified(height, factor, seed, pool)
}

/// Makes [`island_amplified`]'s field ahead, on the loading thread beside the water, which it
/// does not need (#201). It holds the lock while it works, so a caller waits for it rather than
/// making the field twice.
pub(crate) fn amplify_ahead(height: &Field2<f32>, factor: u32, seed: u64, pool: &TaskPool) {
    let key = amplified_key(height, factor, seed);
    let mut ready = AMPLIFIED.lock().expect("the amplified field");
    if !ready.as_ref().is_some_and(|(made_for, _)| *made_for == key) {
        *ready = Some((key, make_island_amplified(height, factor, seed, pool)));
    }
}

/// [`island_amplified`], made.
fn make_island_amplified(
    height: &Field2<f32>,
    factor: u32,
    seed: u64,
    pool: &TaskPool,
) -> Field2<f32> {
    let mut field = height.clone();
    for level in 0..factor.trailing_zeros() {
        let flow = forge_procgen::drain(&field, 0.0, pool);
        let params = forge_procgen::AmplifyParams::island(forge_core::Seed::new(
            seed ^ (0xa3f1_0000 + u64::from(level)),
        ));
        field = forge_procgen::amplify(&field, &flow.area, 0.0, &params, pool);
    }
    field
}

/// [`DrawnGround`], made once a process for the field and the factor: every tile asks for it.
/// At the field's spacing, the field's samples and its refined cells on the cells' planes; finer,
/// the field's cubic with the channels carved, its samples and its refined cells
/// (`Channels::fine`), and away from the water the amplification's detail (`--island-detail`).
pub fn island_drawn() -> Arc<DrawnGround> {
    static MADE: std::sync::Mutex<Option<(u64, Arc<DrawnGround>)>> = std::sync::Mutex::new(None);
    let height = island_heights();
    let factor = drawn_factor(world());
    let key = height.digest()
        ^ height.spacing.to_bits()
        ^ u64::from(height.size)
        ^ u64::from(factor).rotate_left(48)
        ^ u64::from(world().ground.detail.to_bits()).rotate_left(16);
    let mut made = MADE.lock().expect("the island's drawn ground");
    if let Some((made_for, drawn)) = made.as_ref()
        && *made_for == key
    {
        return drawn.clone();
    }
    let derived = derived_cache().get_or_make("island-drawn", drawn_key(world()), || {
        make_island_drawn(height, factor)
    });
    tracing::info!(
        from_cache = derived.from_cache,
        ms = derived.ms,
        "the island's drawn ground"
    );
    let drawn = Arc::new(derived.value);
    *made = Some((key, drawn.clone()));
    drawn
}

forge_core::stored!(DrawnGround {
    size,
    spacing,
    heights,
    detail
});

/// [`island_drawn`], made.
fn make_island_drawn(height: Field2<f32>, factor: u32) -> DrawnGround {
    let start = Instant::now();
    let IslandWater {
        channels, lakes, ..
    } = island_water(&height);
    let pool = TaskPool::client();
    let (size, spacing) = (height.size, height.spacing);
    let mut detail_ms = 0;
    let drawn = if factor == 1 {
        let heights = channels.detail(&height, &pool);
        DrawnGround {
            size,
            spacing,
            heights: Arc::from(height.data),
            detail: Arc::new(HeightfieldDetail {
                split: channels.params().split,
                cells: channels.refined().to_vec(),
                heights,
            }),
        }
    } else {
        let fine = channels.fine(&height, factor, &pool);
        let mut ground = fine.height;
        if world().ground.detail > 0.0 {
            let detail_start = Instant::now();
            let amplified = island_amplified(&height, factor, world().island.seed.value(), &pool);
            // How far each sample stands from the water's reach: the samples of the refined
            // cells, the lakes', and those under the shore's band.
            let n = size as usize;
            let side = n - 1;
            let mut near: Vec<bool> = height
                .data
                .iter()
                .map(|&h| h < world().ground.shore_smoothing.0)
                .collect();
            for &c in channels.refined() {
                let (i, j) = (c as usize % side, c as usize / side);
                for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    near[(j + dj) * n + i + di] = true;
                }
            }
            for lake in &lakes {
                for j in lake.first[1]..lake.first[1] + lake.size[1] {
                    for i in lake.first[0]..lake.first[0] + lake.size[0] {
                        if lake.covers(i, j) {
                            near[j as usize * n + i as usize] = true;
                        }
                    }
                }
            }
            let distance = forge_procgen::site_distance(size, spacing, |i| near[i], &pool);
            let (fine_size, fine_spacing) = (ground.size as usize, ground.spacing);
            let strength = world().ground.detail;
            // What the detail adds where it is whole: its root mean square and its largest, a
            // row a task (one thread took a second over the 67 M samples, #201).
            let mut rows = vec![(0.0f64, 0u64, 0.0f32); fine_size];
            pool.par_map_into(&mut rows, 64, |j| {
                let (mut sum, mut count, mut largest) = (0.0f64, 0u64, 0.0f32);
                for i in 0..fine_size {
                    let d = distance.sample(i as f64 * fine_spacing, j as f64 * fine_spacing);
                    if d >= world().ground.detail_fade.1 {
                        let s = j * fine_size + i;
                        let r = amplified.data[s] - ground.data[s];
                        sum += f64::from(r * r);
                        count += 1;
                        largest = largest.max(r.abs());
                    }
                }
                (sum, count, largest)
            });
            let (sum, count, largest) = rows.iter().fold((0.0f64, 0u64, 0.0f32), |a, r| {
                (a.0 + r.0, a.1 + r.1, a.2.max(r.2))
            });
            tracing::info!(
                rms_m = %format_args!("{:.3}", (sum / count.max(1) as f64).sqrt()),
                largest_m = %format_args!("{largest:.2}"),
                samples = count,
                "the amplification's detail over the cubic, away from the water"
            );
            pool.par_chunks_mut(&mut ground.data, fine_size, |j, row| {
                let y = j as f64 * fine_spacing;
                for (i, h) in row.iter_mut().enumerate() {
                    let d = distance.sample(i as f64 * fine_spacing, y);
                    let t = ((d - world().ground.detail_fade.0)
                        / (world().ground.detail_fade.1 - world().ground.detail_fade.0))
                        .clamp(0.0, 1.0);
                    let w = strength * t * t * (3.0 - 2.0 * t);
                    *h += w * (amplified.data[j * fine_size + i] - *h);
                }
            });
            detail_ms = detail_start.elapsed().as_millis();
        }
        DrawnGround {
            size: ground.size,
            spacing: ground.spacing,
            heights: Arc::from(ground.data),
            detail: Arc::new(HeightfieldDetail {
                split: fine.split,
                cells: fine.cells,
                heights: fine.heights,
            }),
        }
    };
    tracing::info!(
        drawn_m = drawn.spacing,
        samples = drawn.size,
        detail = world().ground.detail,
        refined_cells = drawn.detail.cells.len(),
        fine_vertices = drawn.detail.heights.len(),
        detail_ms,
        ms = start.elapsed().as_millis(),
        "island ground drawn, the river channels carved"
    );
    drawn
}
