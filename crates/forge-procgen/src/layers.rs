//! Stage 6 of the terrain pipeline, its first rule: the ground's material layers from the
//! fields (`docs/research/terrain-genesis.md`, "Recommendation for Forge"), as the layer map
//! the terrain's layered material samples (D-028). Today's rule is slope and altitude: rock
//! where the ground is steep or high, grass elsewhere, and with a shore the sea below 0 m and
//! sand on the land's first metres above it; the others (wet soil along the rivers, snow above
//! a line) follow with the fields they need.

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
    let slopes = Field2::from_fn(height.size, height.spacing, |x, y| {
        let (gx, gy) = height.gradient(x, y);
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
            shore: None,
        };
        // Texels of 2 m. The nearest sample's slope made rock of every texel from 76 to 92 m
        // (8 texels); interpolated, the slope passes 0.5 from 78.4 to 89.6 m (6 texels).
        let layers = slope_layers(&height, &rule, 80);
        let rock: Vec<u32> = (0..80).filter(|&x| layers.get(x, 40) == 4).collect();
        assert_eq!(rock, (39..=44).collect::<Vec<_>>());
    }
}
