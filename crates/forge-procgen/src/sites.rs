//! Where the island's loose rocks lie (#130): a map of cells giving how likely a rock is in
//! each and of which rock, for a placement to draw the rocks from.
//!
//! Rocks gather where the ground puts them: at the foot of the steep ground and below it,
//! where they fall to (talus); on the granite's convex crests, where its corestones weather
//! out as tors; on the scree; on the karst's pavements, as loose blocks. A few cling to the
//! steep faces and fewer still lie on the grass. None lie on the beaches, in the rivers or the
//! lakes, or under the height the placement starts at. Patches of noise group them, so they
//! never read as an even sprinkle. Which rock follows the ground's geology (D-042,
//! [`GeologyRule`]).

use forge_core::hash::{hash_cell2, unit_f32};

use crate::beach::blur;
use crate::field::Field2;
use crate::layers::GeologyRule;
use crate::noise;
use crate::river::smoothstep;

/// The rule of [`rock_sites`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RockSiteRule {
    /// Metres across a cell (a whole number of the layer map's texels).
    pub cell: f64,
    /// Metres over the sea under which no rock lies.
    pub above: f32,
    /// Rise over run over which the ground is steep (faces, and what talus falls from).
    pub steep: f32,
    /// Metres over which the slope is measured.
    pub slope_over: f64,
    /// Metres around the steep ground its talus reaches.
    pub talus_reach: f64,
    /// Metres around which a crest stands out, and the metres it stands out by from none of
    /// them to all of them being tors.
    pub crest_reach: f64,
    /// See [`RockSiteRule::crest_reach`].
    pub crest_rise: (f64, f64),
    /// The weights of the sites (0 to 1).
    pub weights: SiteWeights,
    /// Metres across the patches that group the rocks.
    pub patch: f64,
    /// The share of a site's rocks left between its patches.
    pub between: f64,
    /// Rise over run from which the scattered rocks start, and over which all of them lie.
    pub scatter_slope: (f64, f64),
    /// The seed of the patches.
    pub seed: u64,
}

/// How likely a rock is on each kind of site, the most likely being 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteWeights {
    /// Below the steep ground.
    pub talus: f64,
    /// On the scree.
    pub scree: f64,
    /// On the steep faces.
    pub faces: f64,
    /// On the granite's crests.
    pub crests: f64,
    /// On the karst.
    pub karst: f64,
    /// Anywhere else on the land, from none on the flat to all of it on a slope of
    /// [`RockSiteRule::scatter_slope`]`.1` (the deep soil of the plains has no stones).
    pub scatter: f64,
}

impl Default for RockSiteRule {
    /// Cells of 8 m, rocks from 3 m up; steep from a slope of 0.55 over 8 m; talus within 40 m
    /// of it; crests standing 3 to 12 m over the ground 64 m around; patches 60 m across with
    /// a tenth of the rocks between them; the scattered ones from slopes of 0.05 to 0.25.
    fn default() -> Self {
        Self {
            cell: 8.0,
            above: 3.0,
            steep: 0.55,
            slope_over: 8.0,
            talus_reach: 40.0,
            crest_reach: 64.0,
            crest_rise: (3.0, 12.0),
            weights: SiteWeights {
                talus: 1.0,
                scree: 1.0,
                faces: 0.06,
                crests: 0.6,
                karst: 0.35,
                scatter: 0.004,
            },
            patch: 60.0,
            between: 0.1,
            scatter_slope: (0.05, 0.25),
            seed: 0x726f_636b_7369_7465,
        }
    }
}

/// The layers [`rock_sites`] reads.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteLayers {
    /// The scree.
    pub scree: u8,
    /// The karst.
    pub karst: u8,
    /// The layers no rock lies on (the beaches, the rivers, the lakes and their beds).
    pub none: Vec<u8>,
}

/// The kinds of site, in [`RockSiteStats::by_site`]'s order.
pub const SITE_NAMES: [&str; 6] = ["talus", "scree", "faces", "crests", "karst", "scatter"];

/// What [`rock_sites`] found: the cells' weights summed (in the map's units, 0 to 127 a cell)
/// per rock and per kind of site, the site of a cell being its likeliest.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RockSiteStats {
    /// On the granite.
    pub granite: f64,
    /// On the limestone.
    pub limestone: f64,
    /// Per kind of site ([`SITE_NAMES`]).
    pub by_site: [f64; 6],
}

impl RockSiteStats {
    /// The share of the rocks on the limestone.
    pub fn limestone_share(&self) -> f64 {
        self.limestone / (self.granite + self.limestone).max(1e-9)
    }
}

/// The rock sites over `layers`' extent (rows along +y), on `height` (the sea at 0 m): a map
/// of cells [`RockSiteRule::cell`] across, each the likelihood of a rock in it in its low
/// seven bits (0 to 127) and its rock in the high bit (1: limestone, by `geology`; 0: granite).
pub fn rock_sites(
    height: &Field2<f32>,
    layers: &Field2<u8>,
    ids: &SiteLayers,
    geology: &GeologyRule,
    rule: &RockSiteRule,
) -> (Field2<u8>, RockSiteStats) {
    let per = (rule.cell / layers.spacing).round().max(1.0) as u32;
    let n = layers.size / per;
    let cell = layers.spacing * f64::from(per);
    let centre = |c: usize| {
        (
            (f64::from(c as u32 % n) + 0.5) * cell,
            (f64::from(c as u32 / n) + 0.5) * cell,
        )
    };
    let reach = (rule.slope_over / height.spacing).round().max(1.0);
    let slope_at = |x: f64, y: f64| {
        let d = reach * height.spacing;
        let gx = (height.sample(x + d, y) - height.sample(x - d, y)) as f64 / (2.0 * d);
        let gy = (height.sample(x, y + d) - height.sample(x, y - d)) as f64 / (2.0 * d);
        gx.hypot(gy)
    };
    let cells = (n * n) as usize;
    let h: Vec<f64> = (0..cells)
        .map(|c| {
            let (x, y) = centre(c);
            f64::from(height.sample(x, y))
        })
        .collect();
    let slope: Vec<f64> = (0..cells)
        .map(|c| {
            let (x, y) = centre(c);
            slope_at(x, y)
        })
        .collect();
    let steep: Vec<f64> = slope
        .iter()
        .map(|&s| f64::from(u8::from(s > f64::from(rule.steep))))
        .collect();
    let steep_h: Vec<f64> = steep.iter().zip(&h).map(|(s, h)| s * h).collect();
    let talus_cells = (rule.talus_reach / cell).round().max(1.0) as usize;
    let near = blur(&steep, n as usize, talus_cells);
    let near_h = blur(&steep_h, n as usize, talus_cells);
    let around = blur(
        &h,
        n as usize,
        (rule.crest_reach / cell).round().max(1.0) as usize,
    );
    let w = rule.weights;
    let mut stats = RockSiteStats::default();
    let mut map = Field2::new(n, cell);
    for c in 0..cells {
        let (x, y) = centre(c);
        if h[c] < f64::from(rule.above) {
            continue;
        }
        // The layer map's texels in the cell: any that takes no rock keeps the cell clear.
        let (cx, cy) = (c as u32 % n, c as u32 / n);
        let (mut scree, mut karst, mut none) = (0_u32, 0_u32, false);
        for t in 0..per * per {
            let layer = layers.get(cx * per + t % per, cy * per + t / per);
            scree += u32::from(layer == ids.scree);
            karst += u32::from(layer == ids.karst);
            none |= ids.none.contains(&layer);
        }
        if none {
            continue;
        }
        let share = |k: u32| f64::from(k) / f64::from(per * per);
        let limestone = geology.is_limestone(x, y, h[c] as f32);
        // Talus: below the steep ground near the cell (lower than its mean height there).
        let talus = if steep[c] == 0.0 && near[c] > 0.0 && h[c] < near_h[c] / near[c] - 1.0 {
            smoothstep(0.03, 0.3, near[c])
        } else {
            0.0
        };
        let crest = if limestone || steep[c] > 0.0 {
            0.0
        } else {
            smoothstep(rule.crest_rise.0, rule.crest_rise.1, h[c] - around[c])
        };
        let sites = [
            w.talus * talus,
            w.scree * share(scree),
            w.faces * steep[c],
            w.crests * crest,
            w.karst * share(karst),
            w.scatter * smoothstep(rule.scatter_slope.0, rule.scatter_slope.1, slope[c]),
        ];
        let (site, likely) = sites
            .iter()
            .copied()
            .enumerate()
            .fold(
                (5, 0.0),
                |best, (k, v)| if v > best.1 { (k, v) } else { best },
            );
        // Talus is the rock it fell from: the steep ground's above it.
        let limestone = if site == 0 {
            geology.is_limestone(x, y, (near_h[c] / near[c]) as f32)
        } else {
            limestone
        };
        let patch = noise::fbm(rule.seed, x / rule.patch, y / rule.patch, 3, 2.0, 0.5);
        let grouped = rule.between + (1.0 - rule.between) * smoothstep(-0.35, 0.45, patch);
        // Rounded by a dither, so a weight under one in 127 keeps its share of the cells.
        let dither = f64::from(unit_f32(hash_cell2(rule.seed ^ 3, cx as i32, cy as i32)));
        let density = (likely * grouped * 127.0 + dither)
            .floor()
            .clamp(0.0, 127.0) as u8;
        if density == 0 {
            continue;
        }
        map.data[c] = density | if limestone { 0x80 } else { 0 };
        let d = f64::from(density);
        if limestone {
            stats.limestone += d;
        } else {
            stats.granite += d;
        }
        stats.by_site[site] += d;
    }
    (map, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talus_lies_under_the_cliff_none_on_the_beach_and_the_rock_follows_the_height() {
        // Along x: a beach rising from the sea at x = 0 to 2 m at 100 m, a hillside rising 0.15 m
        // per metre to 600 m, a cliff of slope 1.5 for 60 m, then a high plain; 4 m samples
        // and 4 m texels, sand under 2 m.
        let ground = |x: f64| -> f64 {
            if x < 100.0 {
                x * 0.02
            } else if x < 600.0 {
                2.0 + (x - 100.0) * 0.15
            } else if x < 660.0 {
                77.0 + (x - 600.0) * 1.5
            } else {
                167.0 + (x - 660.0) * 0.02
            }
        };
        let height = Field2::from_fn(257, 4.0, |x, _| ground(f64::from(x) * 4.0) as f32);
        let (sand, scree, karst) = (1, 2, 3);
        let layers = Field2::from_fn(256, 4.0, |x, _| {
            if ground((f64::from(x) + 0.5) * 4.0) < 2.0 {
                sand
            } else {
                0
            }
        });
        let geology = GeologyRule {
            limestone_below: (90.0, 0.0),
            ragged: 0.0,
            ..GeologyRule::default()
        };
        let rule = RockSiteRule {
            between: 1.0,
            ..RockSiteRule::default()
        };
        let ids = SiteLayers {
            scree,
            karst,
            none: vec![sand],
        };
        let (map, stats) = rock_sites(&height, &layers, &ids, &geology, &rule);
        assert_eq!(map.size, 128);
        let at = |x: f64| map.get((x / 8.0) as u32, 64);
        let density = |x: f64| at(x) & 0x7f;
        // None on the beach or the high plain; a few on the hillside (a weight of a quarter, its
        // cells one in four); many at the cliff's foot; some on its face.
        assert_eq!(density(50.0), 0);
        assert!((90..120).all(|x| density(f64::from(x) * 8.0) == 0));
        let hillside = (20..70)
            .map(|x| u32::from(density(f64::from(x) * 8.0)))
            .sum::<u32>();
        assert!((5..25).contains(&hillside), "{hillside}");
        assert!(density(590.0) > 60, "{}", density(590.0));
        assert!((4..12).contains(&density(630.0)), "{}", density(630.0));
        // Limestone under 90 m, granite over it; but the talus at the cliff's foot, on the
        // limestone, is the granite it fell from.
        assert_eq!(at(590.0) & 0x80, 0);
        assert_eq!(at(800.0) & 0x80, 0);
        assert!((20..70).any(|x| at(f64::from(x) * 8.0) == 0x80 | 1));
        // The talus is the likeliest site below the cliff, the scatter's the most cells.
        assert!(stats.by_site[0] > 0.0 && stats.by_site[2] > 0.0);
        assert!(stats.granite > 0.0 && stats.limestone > 0.0);
        let share = stats.limestone_share();
        assert!(share > 0.0 && share < 1.0);
    }
}
