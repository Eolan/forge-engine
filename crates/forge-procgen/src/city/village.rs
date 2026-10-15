//! The first map's village (D-062, #266): a site a few hundred metres across planned as the
//! city is (streets, blocks, parks), each built block cut into strips of village lots
//! ([`super::parcels::strips`]) and a house from its rooms put on each
//! ([`super::houses::houses`]), the households drawn by the seed: so many people, so well off.
//! A block too thin or concave for the strips keeps no houses here; the map says how many.

use forge_core::SplitMix64;
use glam::DVec2;

use super::blocks::BlockUse;
use super::houses::{self, House};
use super::parcels::{self, ParcelType};
use super::rooms::Means;
use super::{CityParams, CityPlan, Site, plan};

/// A built block's lots, court and houses.
#[derive(Clone, Debug, PartialEq)]
pub struct VillageBlock {
    /// The block, an index into the plan's.
    pub block: usize,
    /// Its lots, each its street side first.
    pub lots: Vec<Vec<DVec2>>,
    /// Its court, the block inset by the strips' depth.
    pub court: Vec<DVec2>,
    /// Its houses, one a lot.
    pub houses: Vec<House>,
}

/// The village: its plan and its blocks' houses.
#[derive(Clone, Debug)]
pub struct Village {
    /// The site's plan: streets, blocks, parks.
    pub plan: CityPlan,
    /// The built blocks that took strips, with their houses.
    pub blocks: Vec<VillageBlock>,
    /// The built blocks that took no strips (too thin or concave for the depth).
    pub without: usize,
}

impl Village {
    /// All the houses.
    pub fn houses(&self) -> impl Iterator<Item = &House> {
        self.blocks.iter().flat_map(|b| b.houses.iter())
    }
}

/// A household drawn for lot `k` of block `b`: one to five people, most of modest means, a few
/// poor, a third comfortable, one in twenty rich.
fn household(seed: u64, b: usize, k: usize) -> (u32, Means) {
    let mut rng =
        SplitMix64::new(seed ^ (b as u64 * 0x9E37_79B9_7F4A_7C15) ^ (k as u64 * 0xC2B2_AE35));
    let people = 1 + (rng.range_f64(0.0, 1.0) * 5.0) as u32;
    let draw = rng.range_f64(0.0, 1.0);
    let means = if draw < 0.15 {
        Means::Poor
    } else if draw < 0.65 {
        Means::Modest
    } else if draw < 0.95 {
        Means::Comfortable
    } else {
        Means::Rich
    };
    (people, means)
}

/// The village on `site` by `params`: the plan, then each built block's strips and houses.
pub fn village(site: &Site, params: &CityParams, seed: u64) -> Village {
    let city = plan(site, params);
    let ty = ParcelType::VILLAGE;
    let mut blocks = Vec::new();
    let mut without = 0;
    for (b, block) in city.blocks.iter().enumerate() {
        if block.use_ != BlockUse::Built {
            continue;
        }
        let n = block.buildable.len();
        let Some((lots, court)) =
            parcels::strips(&block.buildable, &vec![true; n], &ty, seed ^ b as u64)
        else {
            without += 1;
            continue;
        };
        let houses = houses::houses(&lots, &ty, |k| household(seed, b, k));
        blocks.push(VillageBlock {
            block: b,
            lots,
            court,
            houses,
        });
    }
    Village {
        plan: city,
        blocks,
        without,
    }
}

#[cfg(test)]
mod tests {
    use super::super::example;
    use super::*;

    #[test]
    fn the_example_village_has_its_houses_on_strips() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village(&site, &example::params(size), 1);
        let houses = v.houses().count();
        let people: u32 = v.houses().map(|h| h.household.0).sum();
        println!(
            "{} blocks with strips, {} without, {houses} houses, {people} people",
            v.blocks.len(),
            v.without
        );
        assert!(v.blocks.len() >= 4, "{} blocks with strips", v.blocks.len());
        assert!(houses >= 60, "{houses} houses");
        for h in v.houses() {
            assert!(!h.plan.storeys.is_empty() && h.footprint.len() >= 3);
        }
        if let Ok(path) = std::env::var("VILLAGE_PNG") {
            super::super::map::write_village(&v, std::path::Path::new(&path), 0.25)
                .expect("the drawing");
        }
    }
}
