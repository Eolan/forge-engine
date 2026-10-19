//! A species as data: the published size of grown plants, the crown's shape young and old, and
//! the few growth parameters the grower reads ([`crate::grow`]). The village's species live in
//! `data/mediterranean.toml`; [`mediterranean`] reads them.

use forge_core::dmath;
use serde::Deserialize;

use crate::place::Habitat;

/// A tree or a shrub.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Habit {
    /// One trunk with a crown above a clear bole.
    Tree,
    /// Several stems from the ground, the crown down to the ground.
    Shrub,
}

/// What the leaf clusters are made of, for the renderer's cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Foliage {
    /// Needles in tufts (pines, rosemary's narrow leaves).
    Needles,
    /// Broad leaves (oaks, olive, cistus).
    Broad,
    /// Scale leaves in sprays (cypress).
    Scale,
}

/// A crown's envelope: its radius against its normalised height `t` from the crown's base (0)
/// to its top (1) is `R (1 − s²)^fullness`, where `s` runs from −1 at the base through 0 at
/// `widest` to 1 at the top.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Crown {
    /// Where the crown is widest, as a fraction of its depth from its base.
    pub widest: f64,
    /// The profile's exponent: 0.5 an ellipse, lower boxier (a column), higher more pointed.
    pub fullness: f64,
    /// The crown's base (the clear trunk) as a fraction of the plant's height.
    pub base: f64,
}

impl Crown {
    /// The envelope's radius at normalised crown height `t`, as a fraction of the widest.
    pub fn profile(&self, t: f64) -> f64 {
        if !(0.0..=1.0).contains(&t) {
            return 0.0;
        }
        let w = self.widest.clamp(0.02, 0.98);
        let s = if t < w {
            (t - w) / w
        } else {
            (t - w) / (1.0 - w)
        };
        let base = (1.0 - s * s).max(0.0);
        if base <= 0.0 {
            0.0
        } else {
            dmath::powf(base, self.fullness)
        }
    }

    /// The two crowns mixed: `k` 0 gives `self`, 1 gives `other`.
    pub fn lerp(&self, other: &Crown, k: f64) -> Crown {
        let m = |a: f64, b: f64| a + (b - a) * k;
        Crown {
            widest: m(self.widest, other.widest),
            fullness: m(self.fullness, other.fullness),
            base: m(self.base, other.base),
        }
    }
}

/// A species: published sizes and the grower's parameters.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Species {
    /// A stable id (`"aleppo-pine"`): seeds derive from it.
    pub id: String,
    /// The common name.
    pub name: String,
    /// The botanical name.
    pub latin: String,
    /// Tree or shrub.
    pub habit: Habit,
    /// The published height range of grown plants, metres.
    pub height: [f64; 2],
    /// The published crown width (spread) range of grown plants, metres.
    pub crown_width: [f64; 2],
    /// The age at which a plant reaches 95 % of its grown height, years.
    pub maturity: f64,
    /// The leaves' kind.
    pub foliage: Foliage,
    /// The crown when young.
    pub young_crown: Crown,
    /// The crown when grown.
    pub crown: Crown,
    /// The length of one metamer (a node and the internode below it), metres.
    pub internode: f64,
    /// A lateral bud's angle from its parent's axis, degrees.
    pub branch_angle: f64,
    /// The phyllotaxis: the turn between successive lateral buds round an axis, degrees.
    #[serde(default = "default_divergence")]
    pub divergence: f64,
    /// Apical control, Borchert and Honda's λ: above 0.5 the main axis takes more of the
    /// vigour (a leader, an excurrent tree), below it the laterals (a spreading crown).
    pub apical_control: f64,
    /// The vigour per unit of light at the base, Pałubicki et al.'s α.
    #[serde(default = "default_vigour")]
    pub vigour: f64,
    /// The pull upwards on growth (orthotropism), against the shoot's own direction (1).
    pub tropism: f64,
    /// The pull towards the free space the shoot perceives.
    #[serde(default = "default_environment")]
    pub environment: f64,
    /// Random turn per metamer: a gnarled olive high, a cypress low.
    pub wander: f64,
    /// The pipe model's exponent: a parent's radius to the `n` is its children's summed.
    pub pipe_exponent: f64,
    /// The radius of the wood carrying one leaf cluster, metres: each twig tip of the graph
    /// stands for a bundle of real shoots, so this is set for the trunk's base to come out at
    /// the published trunk diameters through the pipe model.
    pub tip_radius: f64,
    /// A leaf cluster's radius, metres.
    pub leaf_cluster: f64,
    /// Wood carrying at most this many twig tips carries leaves: low for tufts at the
    /// branches' ends (pines), high for leaves along the branches (cypress).
    pub foliage_tips: u32,
    /// Cycles a branch may go without light before it is shed.
    pub shed_after: u32,
    /// The most metamers a shoot adds in one cycle.
    pub max_shoot: u32,
    /// Stems from the ground (shrubs), the range a plant draws from.
    #[serde(default = "default_stems")]
    pub stems: [u32; 2],
    /// The stems' lean from the vertical, degrees.
    #[serde(default)]
    pub stem_spread: f64,
    /// The occupancy radius, in internodes: markers this close to wood are taken.
    #[serde(default = "default_occupancy")]
    pub occupancy: f64,
    /// A branch below the crown's base with more nodes than this is a limb: the shade does not
    /// shed it (a holm oak's low limbs); smaller ones are shed as the crown rises.
    #[serde(default = "default_limb_nodes")]
    pub limb_nodes: u32,
    /// Markers per occupancy cube: below 1 an open crown (a pine's), above a dense one.
    #[serde(default = "default_density")]
    pub density: f64,
    /// The perception radius, in internodes: how far a bud sees free space.
    #[serde(default = "default_perception")]
    pub perception: f64,
    /// The perception cone's full angle, degrees.
    #[serde(default = "default_perception_angle")]
    pub perception_angle: f64,
    /// The deepest branch order grown (the trunk is 0).
    #[serde(default = "default_max_order")]
    pub max_order: u8,
    /// Where it grows on its own, for the placement ([`crate::place`]); none for a species only
    /// ever planted by hand.
    #[serde(default)]
    pub habitat: Option<Habitat>,
    /// The share of its height a planted tree's crown is lifted to, clear of the passers-by, as
    /// a street's or a square's are (a young plane was a bush on a stick); 0 for none.
    #[serde(default)]
    pub planted_lift: f64,
    /// How it is drawn (#296).
    #[serde(default)]
    pub look: Look,
}

/// How a species is drawn (#296): the spray its leaf cards are cut from, and two tones of its
/// bark and of its leaves, each plant taking its own mix of the two.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default)]
pub struct Look {
    /// The leaf atlas's spray its cards are cut from.
    pub spray: LeafSpray,
    /// Its bark's two tones, linear RGB.
    pub bark: [[f32; 3]; 2],
    /// Its leaves' two tones, linear RGB.
    pub leaves: [[f32; 3]; 2],
}

impl Default for Look {
    fn default() -> Self {
        Self {
            spray: LeafSpray::Small,
            bark: [[0.20, 0.15, 0.11], [0.36, 0.30, 0.24]],
            leaves: [[0.08, 0.19, 0.06], [0.16, 0.27, 0.07]],
        }
    }
}

/// The sprays drawn in the leaf atlas's quarters (`forge_render::textures::leaves`), in the order
/// [`LeafSpray::quarter`] counts them; a species' cards are cut from one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LeafSpray {
    /// Small oval leaves on a branching twig: holm and kermes oaks, the garrigue's broad shrubs.
    #[default]
    Small,
    /// Broad leaves on long stalks, few to a twig: planes and limes.
    Broad,
    /// Needles in tufts along a shoot: the pines, rosemary.
    Needles,
    /// Narrow pointed leaves in opposite pairs: olives, cistus.
    Narrow,
}

impl LeafSpray {
    /// The four sprays, in their quarters' order.
    pub const ALL: [LeafSpray; 4] = [Self::Small, Self::Broad, Self::Needles, Self::Narrow];

    /// Its quarter of the atlas: 0 top left, 1 top right, 2 bottom left, 3 bottom right.
    pub fn quarter(self) -> u32 {
        match self {
            Self::Small => 0,
            Self::Broad => 1,
            Self::Needles => 2,
            Self::Narrow => 3,
        }
    }

    /// The texture coordinates of its quarter's corners `(u0, v0)` and `(u1, v1)`, a margin of
    /// a hundredth of the atlas inside, the spray's stem at the middle of the `v1` edge.
    pub fn uv_box(self) -> ([f32; 2], [f32; 2]) {
        let q = self.quarter();
        let (u0, v0) = ((q % 2) as f32 * 0.5, (q / 2) as f32 * 0.5);
        ([u0 + 0.01, v0 + 0.01], [u0 + 0.49, v0 + 0.49])
    }
}

fn default_divergence() -> f64 {
    137.5
}
fn default_vigour() -> f64 {
    2.0
}
fn default_environment() -> f64 {
    0.7
}
fn default_stems() -> [u32; 2] {
    [1, 1]
}
fn default_occupancy() -> f64 {
    2.0
}
fn default_limb_nodes() -> u32 {
    60
}
fn default_density() -> f64 {
    1.2
}
fn default_perception() -> f64 {
    5.0
}
fn default_perception_angle() -> f64 {
    90.0
}
fn default_max_order() -> u8 {
    6
}

/// The share of the grown height a plant of `age` years has: a Chapman–Richards curve
/// `(1 − e^(−k·age))²` with `k` set so that the share is 0.95 at `maturity`.
pub fn growth_share(age: f64, maturity: f64) -> f64 {
    if age <= 0.0 {
        return 0.0;
    }
    let k = -dmath::ln(1.0 - dmath::powf(0.95_f64, 0.5)) / maturity.max(0.1);
    let g = 1.0 - dmath::exp(-k * age);
    g * g
}

impl Species {
    /// The crown at `share` of the grown height.
    pub fn crown_at(&self, share: f64) -> Crown {
        self.young_crown.lerp(&self.crown, share.clamp(0.0, 1.0))
    }
}

/// The species file's top level.
#[derive(Deserialize)]
struct File {
    species: Vec<Species>,
}

/// Reads a species file in the form of `data/mediterranean.toml`.
pub fn parse(text: &str) -> Result<Vec<Species>, toml::de::Error> {
    toml::from_str::<File>(text).map(|f| f.species)
}

/// The Mediterranean village's species (#266): Aleppo pine, holm oak, olive, Italian cypress,
/// rosemary, grey-leaved cistus and kermes oak.
pub fn mediterranean() -> Vec<Species> {
    parse(include_str!("../data/mediterranean.toml")).expect("the built-in species file parses")
}

/// The species with this id.
pub fn by_id<'a>(all: &'a [Species], id: &str) -> Option<&'a Species> {
    all.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_village_species_read() {
        let all = mediterranean();
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "aleppo-pine",
                "holm-oak",
                "olive",
                "italian-cypress",
                "rosemary",
                "cistus",
                "kermes-oak",
                "london-plane",
                "common-lime",
                "umbrella-pine"
            ]
        );
        for s in &all {
            assert!(s.height[0] < s.height[1] && s.crown_width[0] < s.crown_width[1]);
            assert!(s.internode > 0.0 && s.tip_radius > 0.0);
            assert!((0.0..=1.0).contains(&s.apical_control));
        }
    }

    #[test]
    fn the_growth_curve_reaches_its_share_at_maturity() {
        assert!((growth_share(80.0, 80.0) - 0.95).abs() < 1e-9);
        assert!(growth_share(10.0, 80.0) < growth_share(20.0, 80.0));
        assert_eq!(growth_share(0.0, 80.0), 0.0);
    }

    #[test]
    fn a_crown_is_widest_where_it_says() {
        let c = Crown {
            widest: 0.3,
            fullness: 0.5,
            base: 0.2,
        };
        assert!((c.profile(0.3) - 1.0).abs() < 1e-12);
        assert!(c.profile(0.0) < 1e-9 && c.profile(1.0) < 1e-9);
        assert!(c.profile(0.6) < c.profile(0.4));
    }
}
