//! Stage 6 of the terrain pipeline, its first rule: the ground's material layers from the
//! fields (`docs/research/terrain-genesis.md`, "Recommendation for Forge"), as the layer map
//! the terrain's layered material samples (D-028). Today's rule is slope and altitude: rock
//! where the ground is steep or high, grass elsewhere, and with a shore the sea below 0 m and
//! sand on the land's first metres above it; and the rivers of stage 4 painted over it at their
//! width (`paint_rivers`). The others (wet soil along the rivers, snow above a line) follow with
//! the fields they need.

use crate::field::Field2;
use crate::hydrology::{self, Lakes, Rivers};

/// Paints `rivers` (traced on a field of `spacing` metres) into `layers` as `layer`: every
/// texel whose centre lies within half a river's width of its course, the width from the
/// catchment (`hydrology::width`) and at least `min_width` metres, so that a stream narrower
/// than a texel still draws a steady line rather than one that breaks up. Returns the texels
/// painted.
pub fn paint_rivers(
    layers: &mut Field2<u8>,
    rivers: &Rivers,
    spacing: f64,
    layer: u8,
    min_width: f32,
) -> usize {
    let cell = layers.spacing as f32;
    let last = layers.size as i64 - 1;
    let cell_area = spacing * spacing;
    let mut painted = 0;
    for river in &rivers.rivers {
        for (k, pair) in river.points.windows(2).enumerate() {
            let (a, b) = ([pair[0][0], pair[0][1]], [pair[1][0], pair[1][1]]);
            let area = f64::from(river.area[k].max(river.area[k + 1])) * cell_area;
            let half = 0.5 * (hydrology::width(area) as f32).max(min_width);
            // The texels of the segment's box, grown by the half width.
            let lo = |v: f32| (((v - half) / cell).floor() as i64).clamp(0, last);
            let hi = |v: f32| (((v + half) / cell).ceil() as i64).clamp(0, last);
            for ty in lo(a[1].min(b[1]))..=hi(a[1].max(b[1])) {
                for tx in lo(a[0].min(b[0]))..=hi(a[0].max(b[0])) {
                    let p = [(tx as f32 + 0.5) * cell, (ty as f32 + 0.5) * cell];
                    if segment_distance(p, a, b) <= half {
                        let (tx, ty) = (tx as u32, ty as u32);
                        let texel = &mut layers.data[(ty * layers.size + tx) as usize];
                        if *texel != layer {
                            *texel = layer;
                            painted += 1;
                        }
                    }
                }
            }
        }
    }
    painted
}

/// Paints the lakes (traced on a field of `size × size` samples `spacing` metres apart) into
/// `layers` as `layer`: every texel whose nearest sample lies under a lake at least `min_area`
/// m² wide and above `above` metres (a sea flooded by the priority flood is not a lake).
/// Returns the texels painted.
pub fn paint_lakes(
    layers: &mut Field2<u8>,
    lakes: &Lakes,
    (size, spacing): (u32, f64),
    layer: u8,
    min_area: f64,
    above: f32,
) -> usize {
    let wanted: Vec<bool> = lakes
        .lakes
        .iter()
        .map(|l| l.area(spacing) >= min_area && l.level > above)
        .collect();
    let cell = layers.spacing;
    let last = f64::from(size - 1);
    let mut painted = 0;
    for ty in 0..layers.size {
        let sy = ((f64::from(ty) + 0.5) * cell / spacing).round().min(last) as u32;
        for tx in 0..layers.size {
            let sx = ((f64::from(tx) + 0.5) * cell / spacing).round().min(last) as u32;
            let lake = lakes.lake_of[(sy * size + sx) as usize];
            if lake != u32::MAX && wanted[lake as usize] {
                let texel = &mut layers.data[(ty * layers.size + tx) as usize];
                if *texel != layer {
                    *texel = layer;
                    painted += 1;
                }
            }
        }
    }
    painted
}

/// The distance from `p` to the segment `a`–`b`.
fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length2 = dx * dx + dy * dy;
    let t = if length2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (ex, ey) = (p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
    (ex * ex + ey * ey).sqrt()
}

/// Which layer goes where.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerRule {
    /// The layer of the gentle, low ground.
    pub grass: u8,
    /// The layer of the steep or high ground.
    pub rock: u8,
    /// Rise over run above which the ground is rock (0.45: about 24°).
    pub rock_slope: f32,
    /// Metres above which the ground is rock whatever its slope.
    pub rock_above: f32,
    /// Metres over which the slope is measured (Horn's kernel reaches that far each way, in
    /// whole samples, at least one): the same rule at any spacing, where a finer grid's local
    /// steepness turned more of the ground to rock (4 m against 8 m, #96).
    pub slope_over: f64,
    /// The shore's layers, for a field whose sea is at 0 m (the island's).
    pub shore: Option<Shore>,
}

/// The layers where the land meets the sea (issue #96).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shore {
    /// The layer of the sea floor, at and below 0 m: the sea's stand-in until the water is
    /// drawn (D-038).
    pub sea: u8,
    /// The layer of the beach.
    pub sand: u8,
    /// Metres above the sea below which gentle ground is sand (steep ground stays rock).
    pub sand_below: f32,
}

/// The layer map of `height` at `texels × texels` cells over its whole extent, rows along +z
/// (the layout `placement::ground_layers` gives the city): each cell samples the field at its
/// centre, and Horn's gradient there, bilinear between the samples' so that a layer's border
/// follows the ground between samples instead of stepping with them.
pub fn slope_layers(height: &Field2<f32>, rule: &LayerRule, texels: u32) -> Field2<u8> {
    let extent = height.extent();
    let cell = extent / f64::from(texels);
    let reach = (rule.slope_over / height.spacing).round().max(1.0) as i32;
    let slopes = Field2::from_fn(height.size, height.spacing, |x, y| {
        let (gx, gy) = horn(height, x, y, reach);
        (gx * gx + gy * gy).sqrt()
    });
    Field2::from_fn(texels, cell, |x, y| {
        let (mx, my) = ((f64::from(x) + 0.5) * cell, (f64::from(y) + 0.5) * cell);
        let h = height.sample(mx, my);
        let slope = slopes.sample(mx, my);
        match rule.shore {
            Some(shore) if h <= 0.0 => shore.sea,
            _ if slope > rule.rock_slope || h > rule.rock_above => rule.rock,
            Some(shore) if h < shore.sand_below => shore.sand,
            _ => rule.grass,
        }
    })
}

/// Horn's gradient (∂/∂x, ∂/∂y) at sample (x, y) over `reach` samples each way, per metre
/// (`Field2::gradient` at a reach of one); the border uses the samples it has.
fn horn(height: &Field2<f32>, x: u32, y: u32, reach: i32) -> (f32, f32) {
    let n = height.size as i32 - 1;
    let at = |dx: i32, dy: i32| {
        let sx = (x as i32 + dx * reach).clamp(0, n) as u32;
        let sy = (y as i32 + dy * reach).clamp(0, n) as u32;
        height.get(sx, sy)
    };
    let dx = (at(1, -1) + 2.0 * at(1, 0) + at(1, 1)) - (at(-1, -1) + 2.0 * at(-1, 0) + at(-1, 1));
    let dy = (at(-1, 1) + 2.0 * at(0, 1) + at(1, 1)) - (at(-1, -1) + 2.0 * at(0, -1) + at(1, -1));
    let scale = 1.0 / (8.0 * reach as f32 * height.spacing as f32);
    (dx * scale, dy * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steep_and_high_ground_is_rock_the_rest_grass() {
        // A ramp rising 1 m per metre along x from x = 60 m, flat before it, 2 m samples.
        let height = Field2::from_fn(51, 2.0, |x, _| (x as f32 * 2.0 - 60.0).max(0.0));
        let rule = LayerRule {
            grass: 0,
            rock: 4,
            rock_slope: 0.45,
            rock_above: 1000.0,
            slope_over: 2.0,
            shore: None,
        };
        let layers = slope_layers(&height, &rule, 25);
        assert_eq!(layers.size, 25);
        assert_eq!(layers.spacing, 4.0);
        assert_eq!(layers.get(2, 12), 0);
        assert_eq!(layers.get(22, 12), 4);
        // The altitude rule alone.
        let high = LayerRule {
            rock_above: 10.0,
            rock_slope: 100.0,
            ..rule
        };
        let by_height = slope_layers(&height, &high, 25);
        assert_eq!(by_height.get(2, 12), 0);
        assert_eq!(by_height.get(22, 12), 4);
    }

    #[test]
    fn a_shore_puts_the_sea_below_zero_and_sand_on_the_first_metres() {
        // A gentle beach rising 0.1 m per metre along x from x = 40 m, sea before it, then a
        // cliff from x = 120 m; 2 m samples, 4 m texels.
        let height = Field2::from_fn(81, 2.0, |x, _| {
            let m = x as f32 * 2.0;
            if m < 120.0 {
                ((m - 40.0) * 0.1).max(0.0)
            } else {
                8.0 + (m - 120.0)
            }
        });
        let rule = LayerRule {
            grass: 0,
            rock: 4,
            rock_slope: 0.45,
            rock_above: 1000.0,
            slope_over: 2.0,
            shore: Some(Shore {
                sea: 2,
                sand: 1,
                sand_below: 3.0,
            }),
        };
        let layers = slope_layers(&height, &rule, 40);
        assert_eq!(layers.get(5, 20), 2, "the sea at 22 m");
        assert_eq!(layers.get(14, 20), 1, "sand at 58 m, 1.8 m up");
        assert_eq!(layers.get(25, 20), 0, "grass at 102 m, 6.2 m up");
        assert_eq!(layers.get(35, 20), 4, "rock on the cliff at 142 m");
    }

    #[test]
    fn the_slope_between_samples_is_interpolated() {
        // A 10 m step between the samples at 80 and 88 m amid flat ground: Horn's slope is
        // 0.625 at both and 0 at their neighbours (72 and 96 m).
        let height = Field2::from_fn(21, 8.0, |x, _| if x > 10 { 10.0 } else { 0.0 });
        let rule = LayerRule {
            grass: 0,
            rock: 4,
            rock_slope: 0.5,
            rock_above: 1000.0,
            slope_over: 8.0,
            shore: None,
        };
        // Texels of 2 m. The nearest sample's slope made rock of every texel from 76 to 92 m
        // (8 texels); interpolated, the slope passes 0.5 from 78.4 to 89.6 m (6 texels).
        let layers = slope_layers(&height, &rule, 80);
        let rock: Vec<u32> = (0..80).filter(|&x| layers.get(x, 40) == 4).collect();
        assert_eq!(rock, (39..=44).collect::<Vec<_>>());
    }

    #[test]
    fn the_slope_is_measured_over_the_same_metres_at_any_spacing() {
        // A 0.3 incline with ripples 16 m long and 2 m high: over 4 m each way they add up to
        // 0.5 of slope, over 8 m (a whole ripple) they cancel.
        let ground = |spacing: f64, size: u32| {
            Field2::from_fn(size, spacing, |x, _| {
                let m = x as f64 * spacing;
                (0.3 * m + 2.0 * (std::f64::consts::TAU * m / 16.0).sin()) as f32
            })
        };
        let rule = LayerRule {
            grass: 0,
            rock: 4,
            rock_slope: 0.45,
            rock_above: 1000.0,
            slope_over: 8.0,
            shore: None,
        };
        let rock_share = |height: &Field2<f32>, rule: &LayerRule| {
            let layers = slope_layers(height, rule, 64);
            layers.data.iter().filter(|&&l| l == 4).count() as f64 / layers.data.len() as f64
        };
        let (fine, coarse) = (ground(4.0, 65), ground(8.0, 33));
        assert_eq!(rock_share(&fine, &rule), 0.0);
        assert_eq!(rock_share(&coarse, &rule), 0.0);
        // Over the finer grid's own 4 m, the ripples make rock.
        let local = LayerRule {
            slope_over: 4.0,
            ..rule
        };
        assert!(rock_share(&fine, &local) > 0.25);
    }

    #[test]
    fn a_river_is_painted_at_its_width_and_no_wider() {
        use crate::hydrology::{Mouth, River};
        // A river along y = 50 m from x = 10 to 90 m, on a field of 10 m cells. Its catchment,
        // 4 000 cells of 100 m², is 0.4 km²: `width` gives 3.2 m, the minimum 8 m.
        let river = River {
            cells: vec![0, 1],
            points: vec![[10.0, 50.0, 0.0], [90.0, 50.0, 0.0]],
            area: vec![4000, 4000],
            order: 1,
            mouth: Mouth::Outlet(1),
        };
        let rivers = Rivers {
            rivers: vec![river],
            river_of: Vec::new(),
            order: Vec::new(),
        };
        // 2 m texels over 100 m.
        let mut layers = Field2::from_fn(50, 2.0, |_, _| 0_u8);
        let painted = paint_rivers(&mut layers, &rivers, 10.0, 7, 8.0);
        // Texel centres at 47, 49, 51 and 53 m lie within 4 m of the course; 45 and 55 do not.
        let across: Vec<u32> = (0..50).filter(|&y| layers.get(25, y) == 7).collect();
        assert_eq!(across, vec![23, 24, 25, 26]);
        // Along it, the ends round: the rows 1 m off the course reach 6.1–93.9 m (44 texels),
        // those 3 m off 7.4–92.6 m (42).
        assert_eq!(painted, 2 * 44 + 2 * 42);
        assert_eq!(layers.get(1, 25), 0);
    }

    #[test]
    fn lakes_are_painted_where_they_stand_and_small_or_sea_ones_are_not() {
        use crate::hydrology::Lake;
        // A 10 × 10 field of 10 m cells: a 3 × 3 lake at 20 m (9 cells, 900 m²), a one-cell
        // pond (100 m²), and a "lake" at the sea's level.
        let lake = |cells: Vec<u32>, level: f32| Lake {
            level,
            cells,
            depth: 1.0,
            outlet: 0,
        };
        let big: Vec<u32> = (2..5)
            .flat_map(|y| (2..5).map(move |x| y * 10 + x))
            .collect();
        let lakes = Lakes {
            lakes: vec![lake(big, 20.0), lake(vec![77], 20.0), lake(vec![90], 0.0)],
            lake_of: (0..100_u32)
                .map(|i| match i {
                    22..=24 | 32..=34 | 42..=44 => 0,
                    77 => 1,
                    90 => 2,
                    _ => u32::MAX,
                })
                .collect(),
        };
        // 5 m texels over the 100 m: each sample is nearest to about 2 × 2 of them.
        let mut layers = Field2::from_fn(20, 5.0, |_, _| 0_u8);
        let painted = paint_lakes(&mut layers, &lakes, (10, 10.0), 9, 500.0, 1.0);
        assert!(painted > 0);
        // The big lake's middle sample (33, at 30 m) is painted; the pond and the sea are not.
        assert_eq!(layers.get(6, 6), 9);
        assert_eq!(layers.get(14, 14), 0);
        assert_eq!(layers.get(0, 18), 0);
        assert!(layers.data.iter().filter(|&&l| l == 9).count() == painted);
    }
}
