//! Stage 6 of the terrain pipeline, its first rule: the ground's material layers from the
//! fields (`docs/research/terrain-genesis.md`, "Recommendation for Forge"), as the layer map
//! the terrain's layered material samples (D-028). Today's rule is slope and altitude alone:
//! rock where the ground is steep or high, grass elsewhere; the others (sand by the coast, wet
//! soil along the rivers, snow above a line) follow with the fields they need.

use crate::field::Field2;

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
}

/// The layer map of `height` at `texels × texels` cells over its whole extent, rows along +z
/// (the layout `placement::ground_layers` gives the city): each cell samples the field at its
/// centre and Horn's gradient at the nearest sample.
pub fn slope_layers(height: &Field2<f32>, rule: &LayerRule, texels: u32) -> Field2<u8> {
    let extent = height.extent();
    let cell = extent / f64::from(texels);
    let last = height.size - 1;
    Field2::from_fn(texels, cell, |x, y| {
        let (mx, my) = ((f64::from(x) + 0.5) * cell, (f64::from(y) + 0.5) * cell);
        let h = height.sample(mx, my);
        let sx = ((mx / height.spacing).round() as u32).min(last);
        let sy = ((my / height.spacing).round() as u32).min(last);
        let (gx, gy) = height.gradient(sx, sy);
        let slope = (gx * gx + gy * gy).sqrt();
        if slope > rule.rock_slope || h > rule.rock_above {
            rule.rock
        } else {
            rule.grass
        }
    })
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
}
