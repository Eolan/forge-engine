//! The island's beach sand round the walker (issue #197, D-007's layer round the player): a
//! deformable layer (`forge_physics::deform`) 12 m square, a point every 2 cm, that the walker's
//! footfalls press where the ground is drawn as sand. It follows the walker in steps of the
//! ground's 2 m cells, keeping what it holds of its prints.
//!
//! It is drawn as a ground window (`forge_render`): a mesh of a vertex per point, which the skin
//! pass raises to the drawn ground plus the layer, shaded by the island's own layered row where
//! it stands, the terrain's fragments under it not drawn. With each height go the ground's
//! smooth slopes less the ones its facets give, so that untouched it shades as the terrain does.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use forge_geom::city::{CellWindow, heightfield_window_mesh};
use forge_geom::{SkinnedMesh, TriMesh};
use forge_physics::deform::{Layer, Pad, Soft, shifted};
use forge_procgen::Field2;
use forge_render::{HeightField, MoverTransform};
use forge_task::TaskPool;
use glam::{DVec2, Quat, Vec2, Vec3};

use crate::island_walk::Footfall;
use crate::{DrawnGround, SAND_BELOW, SAND_WANDER, island_layer};

/// Points along each side: 12 m at 2 cm.
pub(crate) const POINTS: u32 = 601;
/// Metres between two points.
pub(crate) const CELL: f32 = 0.02;
/// How far the ground in the window may stand above or below its middle, metres: its mesh's
/// clusters' spheres are grown by it, and a window over steeper ground is not drawn.
pub(crate) const REACH: f32 = 3.0;
/// The window's side, metres.
const SIDE: f64 = (POINTS - 1) as f64 * CELL as f64;
/// How far the walker may stray from the window's middle before it steps after it, metres.
const STRAY: f64 = 2.0;
/// The beach's dry sand: 5 cm over the ground and loose, a foot's 34 kPa sinking it 2.2 cm
/// (the yard's damp sand, `Soft::SAND`, took 8 mm), most of what it pushes heaped in a rim,
/// standing at 35° (dry sand's angle of repose).
const SOFT: Soft = Soft {
    depth: 0.05,
    least: 0.004,
    stiffness: 1.5e6,
    packing: 0.25,
    repose: 0.7,
};
/// A footprint's half sizes across and along, metres, and its pressure: 70 kg on its ellipse.
const FOOT: Vec2 = Vec2::new(0.05, 0.13);
const FOOT_PRESSURE: f32 = 70.0 * 9.81 / (std::f32::consts::PI * 0.05 * 0.13);
/// Where the window's mover waits while it is not drawn: far under the island.
const PARKED: Vec3 = Vec3::new(0.0, -5000.0, 0.0);

/// The window, its layer and the ground under it.
pub(crate) struct SandWindow {
    ground: Arc<DrawnGround>,
    layers: Arc<Field2<u8>>,
    /// Half the field's extent: the scene's frame is the field's less this.
    half: f64,
    layer: Layer,
    /// Per point: the drawn ground, the scene's y.
    base: Vec<f32>,
    /// Per point: the drawn ground's smooth slope (x, z), from the normals the tiles' vertices
    /// carry, between their triangles' corners.
    smooth: Vec<[f32; 2]>,
    /// Per point: whether the ground is drawn as sand there.
    sand: Vec<bool>,
    /// Whether the window stands somewhere, and the ground at its middle (where its mover is).
    placed: bool,
    middle: f32,
    /// Whether it is drawn: it holds sand, over ground within `REACH` of its middle.
    shown: bool,
    /// Frames its heights still go up: the one it changed and the next, whose previous
    /// heights must match (the skin pass runs only when they come).
    uploads: u8,
    field: Vec<f32>,
    /// Milliseconds its steps and its uploads' fields took since the last title, footfalls
    /// pressed.
    step_ms: Vec<f64>,
    field_ms: Vec<f64>,
    pressed: u32,
}

impl SandWindow {
    /// A window over `ground`, sand where `layers` (the island's layer map, `island_layer`) and
    /// the height draw it; placed when a walker comes.
    pub(crate) fn new(ground: Arc<DrawnGround>, layers: Arc<Field2<u8>>) -> Self {
        let half = 0.5 * f64::from(ground.size - 1) * ground.spacing;
        let n = (POINTS * POINTS) as usize;
        Self {
            ground,
            layers,
            half,
            layer: untouched(DVec2::ZERO),
            base: vec![0.0; n],
            smooth: vec![[0.0; 2]; n],
            sand: vec![false; n],
            placed: false,
            middle: 0.0,
            shown: false,
            uploads: 0,
            field: Vec::new(),
            step_ms: Vec::new(),
            field_ms: Vec::new(),
            pressed: 0,
        }
    }

    /// [`Self::mesh`] cooked for the skin pass, which the height field raises: made once a
    /// process, on the loading thread (#201).
    pub(crate) fn cooked_mesh() -> &'static SkinnedMesh {
        static COOKED: OnceLock<SkinnedMesh> = OnceLock::new();
        COOKED.get_or_init(|| SkinnedMesh::cook_displaced(&Self::mesh(), REACH))
    }

    /// The mesh the window is drawn with: a vertex per point, flat, from its first point.
    pub(crate) fn mesh() -> TriMesh {
        let n = POINTS;
        let mut mesh = TriMesh::default();
        for z in 0..n {
            for x in 0..n {
                mesh.positions.push([x as f32 * CELL, 0.0, z as f32 * CELL]);
                mesh.normals.push([0.0, 1.0, 0.0]);
            }
        }
        for z in 0..n - 1 {
            for x in 0..n - 1 {
                let v = z * n + x;
                // Counter-clockwise seen from above, split along the drawn ground's diagonal.
                mesh.indices
                    .extend_from_slice(&[v, v + n, v + 1, v + 1, v + n, v + n + 1]);
            }
        }
        mesh
    }

    /// Its height field, for `add_displaced_mesh`.
    pub(crate) fn height_field() -> HeightField {
        HeightField {
            origin: [0.0; 2],
            cell: CELL,
            size: [POINTS; 2],
            slopes: true,
        }
    }

    /// Follows the walker's feet (none: the window goes), stepping after them, and presses its
    /// footfalls since the last call into the sand.
    pub(crate) fn follow(&mut self, feet: Option<DVec2>, footfalls: &[Footfall]) {
        let Some(feet) = feet else {
            if self.placed {
                self.placed = false;
                self.shown = false;
            }
            return;
        };
        let middle = self.layer.origin() + DVec2::splat(0.5 * SIDE);
        let far = (feet - middle).abs();
        if !self.placed || far.x > STRAY || far.y > STRAY {
            self.step(feet);
        }
        let mut changed = false;
        for f in footfalls {
            if self.sand_at(f.at) {
                self.layer.press(Pad {
                    at: f.at,
                    heading: f.heading,
                    size: FOOT,
                    pressure: FOOT_PRESSURE,
                    sweep: 0.0,
                    wheel: 0.0,
                    tread: None,
                });
                self.pressed += 1;
                changed = true;
            }
        }
        if changed {
            self.uploads = 2;
        }
    }

    /// Moves the window to the 2 m cells round `feet`, keeping the layer where it overlaps the
    /// last place, the rest untouched; the ground under the new points read.
    fn step(&mut self, feet: DVec2) {
        let start = Instant::now();
        let spacing = self.ground.spacing;
        let snap = |c: f64| ((c - 0.5 * SIDE + self.half) / spacing).round() * spacing - self.half;
        let origin = DVec2::new(snap(feet.x), snap(feet.y));
        let n = i64::from(POINTS);
        // The points it moves, whole, or all of them (a first place: nothing to keep).
        let d = (origin - self.layer.origin()) / f64::from(CELL);
        let by = if self.placed {
            [d.x.round() as i64, d.y.round() as i64]
        } else {
            [n, n]
        };
        self.layer.move_to(origin);
        let size = [POINTS; 2];
        self.base = shifted(&self.base, size, by, 0.0);
        self.smooth = shifted(&self.smooth, size, by, [0.0; 2]);
        self.sand = shifted(&self.sand, size, by, false);
        // The points it took on: those whose old place lay off the old window.
        let fresh: Vec<usize> = (0..n * n)
            .filter(|&k| {
                let (x, z) = (k % n + by[0], k / n + by[1]);
                !((0..n).contains(&x) && (0..n).contains(&z))
            })
            .map(|k| k as usize)
            .collect();
        self.read_ground(&fresh);
        let k = |x: u32, z: u32| (z * POINTS + x) as usize;
        self.middle = self.base[k(POINTS / 2, POINTS / 2)];
        let (low, high) = self
            .base
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &b| (l.min(b), h.max(b)));
        self.shown =
            self.sand.iter().any(|&s| s) && high - self.middle < REACH && self.middle - low < REACH;
        self.placed = true;
        self.uploads = 2;
        self.step_ms.push(start.elapsed().as_secs_f64() * 1e3);
    }

    /// The drawn ground under points `fresh`: its height and smooth slope from the triangles the
    /// tiles draw (their vertices' normals between their corners), and whether it is sand.
    fn read_ground(&mut self, fresh: &[usize]) {
        if fresh.is_empty() {
            return;
        }
        let g = &self.ground;
        let spacing = g.spacing;
        let origin = self.layer.origin();
        let first = [
            ((origin.x + self.half) / spacing).round() as u32,
            ((origin.y + self.half) / spacing).round() as u32,
        ];
        let cells = (SIDE / spacing).round() as u32;
        let side = g.size - 1;
        let window = CellWindow {
            first: [first[0].min(side - 1), first[1].min(side - 1)],
            cells: [
                cells.min(side - first[0].min(side - 1)),
                cells.min(side - first[1].min(side - 1)),
            ],
        };
        let mesh = heightfield_window_mesh(g.size, spacing as f32, &g.heights, &g.detail, window);
        // The triangles by the cell their middle lies in.
        let [cx, cz] = window.cells.map(|c| c as usize);
        let mut by_cell: Vec<Vec<u32>> = vec![Vec::new(); cx * cz];
        let cell_of = |x: f64, z: f64| {
            let i = (((x + self.half) / spacing).floor() as i64 - i64::from(window.first[0]))
                .clamp(0, cx as i64 - 1) as usize;
            let j = (((z + self.half) / spacing).floor() as i64 - i64::from(window.first[1]))
                .clamp(0, cz as i64 - 1) as usize;
            j * cx + i
        };
        for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let p = tri.map(|v| mesh.positions[v as usize]);
            let (x, z) = (
                f64::from(p[0][0] + p[1][0] + p[2][0]) / 3.0,
                f64::from(p[0][2] + p[1][2] + p[2][2]) / 3.0,
            );
            by_cell[cell_of(x, z)].push(t as u32);
        }
        // The points on the job system, a run of them a job.
        let this = &*self;
        let mut read = vec![(0.0f32, [0.0f32; 2], false); fresh.len()];
        TaskPool::client().scope(|s| {
            for (points, out) in fresh.chunks(4096).zip(read.chunks_mut(4096)) {
                let (mesh, by_cell, cell_of) = (&mesh, &by_cell, &cell_of);
                s.spawn(move |_| {
                    for (&k, slot) in points.iter().zip(out) {
                        let (x, z) = (k as u32 % POINTS, k as u32 / POINTS);
                        let at = origin + DVec2::new(f64::from(x), f64::from(z)) * f64::from(CELL);
                        let (height, normal) = on_mesh(mesh, &by_cell[cell_of(at.x, at.y)], at)
                            .unwrap_or_else(|| {
                                let h = this.ground.surface_at(at.x + this.half, at.y + this.half);
                                (h as f32, Vec3::Y)
                            });
                        let slope = [-normal.x / normal.y, -normal.z / normal.y];
                        *slot = (height, slope, this.sand_ground(at, height));
                    }
                });
            }
        });
        for (&k, (height, slope, sand)) in fresh.iter().zip(read) {
            self.base[k] = height;
            self.smooth[k] = slope;
            self.sand[k] = sand;
        }
    }

    /// Whether the island's row draws sand at `at` with the ground `height` there, above the sea:
    /// a texel of the contour's own layers (sand or grass) under the sand's contour (the row's
    /// `LayerContour`, its wander left out), or the lakes' sand.
    fn sand_ground(&self, at: DVec2, height: f32) -> bool {
        let l = &self.layers;
        let extent = f64::from(l.size) * l.spacing;
        let texel = |c: f64| {
            ((c / extent + 0.5) * f64::from(l.size))
                .floor()
                .clamp(0.0, f64::from(l.size - 1)) as u32
        };
        let layer = l.get(texel(at.x), texel(at.y));
        // The contour's own layers are sand under it and grass over it, the sand's texels too;
        // the lakes' sand is sand at any height, and the sand's texels far over the contour
        // (the salt water's, #199), where the shader lets the texels decide: its reach twice,
        // the wander and the band (`contour_reach` in meshlet.slang).
        let far = SAND_BELOW + 2.0 * (SAND_WANDER + 0.08);
        height > -0.2
            && (layer == island_layer::LAKE_SAND
                || (layer == island_layer::SAND && height > far)
                || (matches!(
                    layer,
                    island_layer::SAND
                        | island_layer::GRASS
                        | island_layer::DRY_GRASS
                        | island_layer::LUSH_GRASS
                        | island_layer::RIVERBANK
                ) && height < SAND_BELOW))
    }

    /// Whether the window holds sand at `at`.
    fn sand_at(&self, at: DVec2) -> bool {
        let g = (at - self.layer.origin()) / f64::from(CELL);
        let (x, z) = (g.x.round(), g.y.round());
        let n = f64::from(POINTS);
        (0.0..n).contains(&x)
            && (0.0..n).contains(&z)
            && self.sand[(z as u32 * POINTS + x as u32) as usize]
    }

    /// The heights to send this frame, if they changed (this frame or the last): each point's
    /// over the window's mover, then its slopes to add, x then z (`HeightField::slopes`).
    pub(crate) fn field(&mut self) -> Option<&[f32]> {
        if self.uploads == 0 {
            return None;
        }
        self.uploads -= 1;
        let start = Instant::now();
        let n = POINTS as usize;
        let count = n * n;
        self.field.resize(3 * count, 0.0);
        let (heights, slopes) = self.field.split_at_mut(count);
        let drawn = self.layer.heights();
        let relief = self.layer.relief();
        for k in 0..count {
            heights[k] = self.base[k] - self.middle - SOFT.depth + drawn[k] + relief[k];
        }
        // The slopes the skin pass finds between the ground's points (a cell either side,
        // clamped to the window), taken off the smooth ones: it adds these back.
        let cell = f64::from(CELL);
        let base = &self.base;
        let at = |x: usize, z: usize| f64::from(base[z * n + x]);
        for z in 0..n {
            for x in 0..n {
                let k = z * n + x;
                let sx = (at((x + 1).min(n - 1), z) - at(x.saturating_sub(1), z)) / (2.0 * cell);
                let sz = (at(x, (z + 1).min(n - 1)) - at(x, z.saturating_sub(1))) / (2.0 * cell);
                slopes[2 * k] = self.smooth[k][0] - sx as f32;
                slopes[2 * k + 1] = self.smooth[k][1] - sz as f32;
            }
        }
        self.field_ms.push(start.elapsed().as_secs_f64() * 1e3);
        Some(&self.field)
    }

    /// The ground window the terrain leaves to it: min x, min z, max x, max z, the scene's
    /// frame (none while it is not drawn).
    pub(crate) fn rect(&self) -> Option<[f32; 4]> {
        self.shown.then(|| {
            let o = self.layer.origin();
            let end = o + DVec2::splat(SIDE);
            [o.x as f32, o.y as f32, end.x as f32, end.y as f32]
        })
    }

    /// Its mover: its first point at the ground's height at its middle; parked while not drawn.
    pub(crate) fn transform(&self) -> MoverTransform {
        let o = self.layer.origin();
        MoverTransform {
            position: if self.shown {
                Vec3::new(o.x as f32, self.middle, o.y as f32)
            } else {
                PARKED
            },
            rotation: Quat::IDENTITY,
            scale: 1.0,
        }
    }

    /// The title's part: its steps' and fields' time since the last title, footfalls pressed.
    pub(crate) fn title(&mut self) -> String {
        let most = |v: &[f64]| v.iter().copied().fold(0.0, f64::max);
        let title = format!(
            "sand window {} ({} steps, {:.1} ms at most; {} fields, {:.1} ms at most; {} footfalls)",
            if self.shown { "shown" } else { "hidden" },
            self.step_ms.len(),
            most(&self.step_ms),
            self.field_ms.len(),
            most(&self.field_ms),
            self.pressed
        );
        self.step_ms.clear();
        self.field_ms.clear();
        title
    }
}

/// An untouched window from `origin`: the sand's depth at every point, to its edges (the
/// ground goes on past them).
fn untouched(origin: DVec2) -> Layer {
    let mut layer = Layer::new(SOFT, origin, CELL, [POINTS; 2]);
    layer.set_heights(&vec![SOFT.depth; (POINTS * POINTS) as usize]);
    layer
}

/// The height and the normal (its vertices' between its corners) of the triangle of `mesh`
/// among `triangles` that holds `at` (x, z) seen from above.
fn on_mesh(mesh: &TriMesh, triangles: &[u32], at: DVec2) -> Option<(f32, Vec3)> {
    let mut best: Option<(f64, [f64; 3], u32)> = None;
    for &t in triangles {
        let v = [0, 1, 2].map(|c| mesh.indices[3 * t as usize + c] as usize);
        let p = v.map(|i| {
            DVec2::new(
                f64::from(mesh.positions[i][0]),
                f64::from(mesh.positions[i][2]),
            )
        });
        let d = (p[1] - p[0]).perp_dot(p[2] - p[0]);
        if d.abs() < 1e-12 {
            continue;
        }
        let l1 = (at - p[0]).perp_dot(p[2] - p[0]) / d;
        let l2 = (p[1] - p[0]).perp_dot(at - p[0]) / d;
        let l = [1.0 - l1 - l2, l1, l2];
        let worst = l[0].min(l[1]).min(l[2]);
        if best.is_none_or(|(w, ..)| worst > w) {
            best = Some((worst, l, t));
        }
    }
    let (worst, l, t) = best?;
    if worst < -1e-3 {
        return None;
    }
    let v = [0, 1, 2].map(|c| mesh.indices[3 * t as usize + c] as usize);
    let height = (0..3)
        .map(|c| l[c] * f64::from(mesh.positions[v[c]][1]))
        .sum::<f64>() as f32;
    let normal = (0..3)
        .map(|c| Vec3::from_array(mesh.normals[v[c]]) * l[c] as f32)
        .sum::<Vec3>()
        .normalize();
    Some((height, normal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_geom::city::HeightfieldDetail;

    /// A field of 33 × 33 samples 2 m apart (64 m), a gentle bumpy slope with a block of cells
    /// refined in two; its layer map in texels of 4 m, sand west of x = 0 and rock east of it.
    fn beach() -> (Arc<DrawnGround>, Arc<Field2<u8>>) {
        let size = 33u32;
        let height = |i: u32, j: u32| 0.5 + 0.03 * i as f32 + 0.04 * ((i * 7 + j * 3) % 5) as f32;
        let heights: Vec<f32> = (0..size * size)
            .map(|k| height(k % size, k / size))
            .collect();
        let (split, side) = (2u32, size - 1);
        let (mut cells, mut fine) = (Vec::new(), Vec::new());
        for j in 14..18 {
            for i in 10..14 {
                cells.push(j * side + i);
                let [a, b, c, d] =
                    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(di, dj)| height(i + di, j + dj));
                for v in 0..=split {
                    for u in 0..=split {
                        let (s, t) = (u as f32 / 2.0, v as f32 / 2.0);
                        let along = if s + t <= 1.0 {
                            a + s * (b - a) + t * (c - a)
                        } else {
                            d + (1.0 - s) * (c - d) + (1.0 - t) * (b - d)
                        };
                        let edge = u == 0 || u == split || v == 0 || v == split;
                        fine.push(if edge { along } else { along - 0.05 });
                    }
                }
            }
        }
        let ground = Arc::new(DrawnGround {
            size,
            spacing: 2.0,
            heights: heights.into(),
            detail: Arc::new(HeightfieldDetail {
                split,
                cells,
                heights: fine,
            }),
        });
        let mut layers = Field2::new(16, 4.0);
        for j in 0..16 {
            for i in 0..16 {
                let layer = if i < 8 {
                    island_layer::SAND
                } else {
                    island_layer::ROCK
                };
                layers.set(i, j, layer);
            }
        }
        (ground, Arc::new(layers))
    }

    #[test]
    fn the_sand_window_takes_footfalls_on_sand_shades_as_the_ground_and_keeps_its_prints() {
        let (ground, layers) = beach();
        let mut window = SandWindow::new(ground, layers);
        let foot = |x: f64, z: f64| Footfall {
            at: DVec2::new(x, z),
            heading: Vec2::NEG_Y,
        };
        // Placed round the walker on the 2 m cells; a footfall on the sand prints, one on the
        // rock does not.
        window.follow(
            Some(DVec2::new(-1.5, 0.3)),
            &[foot(-4.0, 1.0), foot(2.0, 1.0)],
        );
        let origin = window.layer.origin();
        assert_eq!(origin, DVec2::new(-8.0, -6.0));
        assert!(window.rect().is_some(), "shown over its sand");
        assert_eq!(window.pressed, 1);
        let untouched = SOFT.depth;
        let print = window.layer.height_at(DVec2::new(-4.0, 1.0));
        assert!(print < untouched - 0.015, "{print} under the foot");
        assert_eq!(window.layer.height_at(DVec2::new(2.0, 1.0)), untouched);
        // Its heights: on the drawn ground where untouched, under the mover at the middle's. Its
        // normals as the skin pass makes them (the slopes between points a cell either side,
        // plus the added ones): the tiles' smooth normals there, not their facets'.
        let middle = window.middle;
        let (base, smooth) = (window.base.clone(), window.smooth.clone());
        let field = window.field().expect("the first heights").to_vec();
        let n = POINTS as usize;
        let (heights, slopes) = field.split_at(n * n);
        let cell = CELL;
        let mut worst: f32 = 0.0;
        for (x, z) in [(10, 20), (300, 300), (450, 520), (5, 590), (210, 380)] {
            let k = z * n + x;
            let at = origin + DVec2::new(x as f64, z as f64) * f64::from(cell);
            if (at - DVec2::new(-4.0, 1.0)).length() < 0.5 {
                continue;
            }
            assert!((heights[k] + middle - base[k]).abs() < 1e-5);
            let sx = (heights[k + 1] - heights[k - 1]) / (2.0 * cell) + slopes[2 * k];
            let sz = (heights[k + n] - heights[k - n]) / (2.0 * cell) + slopes[2 * k + 1];
            worst = worst
                .max((sx - smooth[k][0]).abs())
                .max((sz - smooth[k][1]).abs());
        }
        assert!(worst < 1e-3, "slopes {worst} off the smooth ones");
        // The smooth slope differs from a facet's somewhere: the correction does something.
        assert!(slopes.iter().any(|s| s.abs() > 1e-3));
        // The walker 3 m on: the window steps two cells, the print where it lay on the ground.
        window.follow(Some(DVec2::new(1.6, 0.3)), &[]);
        assert_eq!(window.layer.origin(), DVec2::new(-4.0, -6.0));
        assert!((window.layer.height_at(DVec2::new(-4.0, 1.0)) - print).abs() < 1e-6);
        let k = 300 * n + 300;
        let at = window.layer.origin() + DVec2::splat(300.0 * f64::from(cell));
        assert!(
            (window.base[k]
                - window
                    .ground
                    .surface_at(at.x + window.half, at.y + window.half) as f32)
                .abs()
                < 1e-3
        );
        // The walker gone: the window too.
        window.follow(None, &[]);
        assert!(window.rect().is_none());
    }
}
