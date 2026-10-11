//! The style bible as data (#246, `city-generation.md` §15): one record, read from TOML, that the
//! plan's stages take their numbers from and that a checker measures the result against, so a
//! city stays consistent as it grows. Each rule gives a measured value against its limit, and
//! the subjects that fail it are named (a lot, a block).

use serde::{Deserialize, Serialize};

use super::blocks::BlockUse;
use super::mass::{Mass, Roof};
use super::{CityParams, CityPlan, DistrictKind, geom};

/// A district kind's rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DistrictStyle {
    /// The kind it applies to.
    pub kind: DistrictKind,
    /// Floors, lowest and highest.
    pub floors: (u8, u8),
    /// A lot's area, m².
    pub lot_area: f64,
    /// The street spacing (the block's size), metres.
    pub street_spacing: f64,
    /// The walls' material, by name in the city's palette.
    pub walls: String,
}

/// A city's rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleBible {
    /// Its name.
    pub name: String,
    /// A building's height is at most this times the width of the street it fronts (Paris has
    /// held roughly 1 to 1.5 since 1784).
    pub height_to_width: f64,
    /// The share of roofs that are green.
    pub green_roofs: f64,
    /// A built block's green share: its courtyard, and 70 % of its green roofs (a simple
    /// green-area factor), over its area.
    pub green_area: f64,
    /// Every built block has a park of `park_min_area` within this, metres (3-30-300).
    pub park_reach: f64,
    /// The smallest park that counts, m².
    pub park_min_area: f64,
    /// The share of built blocks within 400 m of a tram stop.
    pub stop_target: f64,
    /// The streets' largest share of the land.
    pub street_share: f64,
    /// Each district kind's rules (kinds not listed keep the plan's own numbers).
    pub districts: Vec<DistrictStyle>,
}

impl Default for StyleBible {
    fn default() -> Self {
        Self {
            name: "default".into(),
            height_to_width: 1.5,
            green_roofs: 0.5,
            green_area: 0.3,
            park_reach: 300.0,
            park_min_area: 10_000.0,
            stop_target: 0.9,
            street_share: 0.25,
            districts: Vec::new(),
        }
    }
}

impl StyleBible {
    /// Reads a bible from TOML; fields it leaves out keep their defaults.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Writes it as TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Puts its numbers into a plan's parameters: the districts of each kind it lists, the parks'
    /// and the stops' rules.
    pub fn apply(&self, params: &mut CityParams) {
        for d in &mut params.districts {
            if let Some(style) = self.districts.iter().find(|s| s.kind == d.kind) {
                d.floors = style.floors;
                d.lot_area = style.lot_area;
                d.street_spacing = style.street_spacing;
            }
        }
        params.park_reach = self.park_reach;
        params.park_min_area = self.park_min_area;
        params.stop_target = self.stop_target;
    }

    /// The most floors a building may have on a street `width` metres wide.
    pub fn max_floors(&self, width: f64) -> u8 {
        let height = self.height_to_width * width;
        let floors = ((height - super::mass::GROUND_FLOOR) / super::mass::FLOOR).floor() + 1.0;
        floors.clamp(1.0, 60.0) as u8
    }
}

/// One rule's result.
#[derive(Clone, Debug, PartialEq)]
pub struct RuleResult {
    /// The rule.
    pub rule: &'static str,
    /// What was measured.
    pub value: f64,
    /// Its limit.
    pub limit: f64,
    /// Whether it holds.
    pub holds: bool,
    /// The subjects that fail it (the first few).
    pub failures: Vec<String>,
    /// How many fail it.
    pub failing: usize,
}

/// Measures a plan and its masses against a bible.
pub fn check(plan: &CityPlan, masses: &[Mass], bible: &StyleBible) -> Vec<RuleResult> {
    let mut out = Vec::new();
    let named = |list: Vec<String>| -> (Vec<String>, usize) {
        let n = list.len();
        (list.into_iter().take(5).collect(), n)
    };
    // Heights against street widths.
    let tall: Vec<String> = masses
        .iter()
        .filter(|m| m.height > bible.height_to_width * m.street_width + 1e-6)
        .map(|m| {
            format!(
                "lot {}: {:.1} m on a {:.0} m street",
                m.lot, m.height, m.street_width
            )
        })
        .collect();
    let share = 1.0 - tall.len() as f64 / masses.len().max(1) as f64;
    let (failures, failing) = named(tall);
    out.push(RuleResult {
        rule: "height at most the street's width times the ratio",
        value: share,
        limit: 1.0,
        holds: failing == 0,
        failures,
        failing,
    });
    // Green roofs.
    let roof_area = |m: &Mass| geom::signed_area(&m.footprint).abs();
    let total: f64 = masses.iter().map(roof_area).sum();
    let green: f64 = masses
        .iter()
        .filter(|m| m.roof == Roof::Green)
        .map(roof_area)
        .sum();
    let share = green / total.max(1.0);
    out.push(RuleResult {
        rule: "green roofs' share",
        value: share,
        limit: bible.green_roofs,
        holds: share + 0.05 >= bible.green_roofs,
        failures: Vec::new(),
        failing: 0,
    });
    // Green area per built block.
    let mut green_by_block = vec![0.0; plan.blocks.len()];
    for m in masses.iter().filter(|m| m.roof == Roof::Green) {
        let block = plan.lots[m.lot as usize].block as usize;
        green_by_block[block] += 0.7 * roof_area(m);
    }
    let mut thin = Vec::new();
    let mut built = 0;
    for (i, b) in plan.blocks.iter().enumerate() {
        if b.use_ != BlockUse::Built {
            continue;
        }
        built += 1;
        let courtyard = b
            .courtyard
            .as_ref()
            .map_or(0.0, |c| geom::signed_area(c).abs());
        let factor = (courtyard + green_by_block[i]) / b.area.max(1.0);
        if factor < bible.green_area {
            thin.push(format!("block {i}: {factor:.2}"));
        }
    }
    let share = 1.0 - thin.len() as f64 / f64::from(built.max(1));
    let (failures, failing) = named(thin);
    out.push(RuleResult {
        rule: "built blocks at their green area",
        value: share,
        limit: 1.0,
        holds: failing == 0,
        failures,
        failing,
    });
    // Parks and stops within reach, the streets' share: the plan's own numbers.
    let st = &plan.stats;
    out.push(RuleResult {
        rule: "built blocks with a park within reach",
        value: st.park_coverage,
        limit: 1.0,
        holds: st.park_coverage >= 0.99,
        failures: Vec::new(),
        failing: 0,
    });
    out.push(RuleResult {
        rule: "built blocks within 400 m of a tram stop",
        value: st.stop_coverage,
        limit: bible.stop_target,
        holds: st.stop_coverage + 1e-9 >= bible.stop_target,
        failures: Vec::new(),
        failing: 0,
    });
    out.push(RuleResult {
        rule: "streets' share of the land at most",
        value: st.street_share,
        limit: bible.street_share,
        holds: st.street_share <= bible.street_share,
        failures: Vec::new(),
        failing: 0,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::super::{example, mass, plan};
    use super::*;

    #[test]
    fn a_bible_reads_from_toml_and_keeps_its_defaults() {
        let text = "name = \"test\"\nheight_to_width = 1.2\n\n[[districts]]\nkind = \"Perimeter\"\nfloors = [5, 7]\nlot_area = 500.0\nstreet_spacing = 120.0\nwalls = \"plaster\"\n";
        let bible = StyleBible::from_toml(text).expect("reads");
        assert_eq!(bible.height_to_width, 1.2);
        assert_eq!(bible.green_roofs, StyleBible::default().green_roofs);
        assert_eq!(bible.districts[0].floors, (5, 7));
        let back = StyleBible::from_toml(&bible.to_toml().expect("writes")).expect("reads back");
        assert_eq!(back, bible);
    }

    #[test]
    fn the_checker_finds_and_names_a_deliberate_breach() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let bible = StyleBible::default();
        let masses = mass::masses(&city, &bible, 11);
        let report = check(&city, &masses, &bible);
        let heights = &report[0];
        assert!(
            heights.holds,
            "the masses obey the bible they were made by: {heights:?}"
        );
        // The same masses against a stricter ratio.
        let strict = StyleBible {
            height_to_width: 0.5,
            ..bible
        };
        let report = check(&city, &masses, &strict);
        assert!(!report[0].holds);
        assert!(
            report[0].failures[0].starts_with("lot "),
            "{:?}",
            report[0].failures
        );
    }
}
