//! The first map's village (D-062, #266): a site a few hundred metres across planned as the
//! city is (streets, blocks, parks), each built block cut into strips of village lots
//! ([`super::parcels::strips`]) and a house from its rooms put on each
//! ([`super::houses::houses`]), the households drawn by the seed: so many people, so well off.
//! A block too thin or concave for the strips keeps no houses here; the map says how many.
//! Each house is also one of the city's masses on its own lot, so the structures, the facades
//! and the kit draw it as they draw the city's buildings.

use forge_core::{Seed, SplitMix64};
use glam::DVec2;

use super::blocks::{self, BlockUse, Lot};
use super::geom::{self, PointGrid};
use super::houses::{self, House};
use super::mass::{Mass, Roof};
use super::parcels::{self, ParcelType};
use super::rooms::{self, Means};
use super::structure::Era;
use super::{
    BoundaryInfluence, CityParams, CityPlan, DistrictKind, DistrictSpec, Pattern, Site, plan,
};

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
    /// The site's plan: streets, blocks, parks; its lots the houses' (appended to the city's
    /// own, which no mass uses).
    pub plan: CityPlan,
    /// The built blocks that took strips, with their houses.
    pub blocks: Vec<VillageBlock>,
    /// The built blocks that took no strips (too thin or concave for the depth).
    pub without: usize,
    /// The houses as the city's masses, one a house in the order of [`Village::houses`], each
    /// on its own lot of the plan: what the structures, the facades and the kit read.
    pub masses: Vec<Mass>,
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

/// The village's own parameters for a site `size` across (D-062, #266): one village district
/// on the site's middle with lanes that wander, one main road, no trams, boats nor landmarks, a
/// green within 200 m of every block, the land beyond the village left to the forest.
pub fn params(size: f64) -> CityParams {
    let at = |x: f64, y: f64| DVec2::new(x * size, y * size);
    CityParams {
        seed: Seed::new(20_261_016),
        districts: vec![DistrictSpec {
            name: "village".into(),
            kind: DistrictKind::Village,
            anchor: at(0.5, 0.5),
            radius: 0.2 * size,
            pattern: Pattern::Organic(1.2),
            grid_angle: 0.3,
            street_spacing: 80.0,
            lot_area: 200.0,
            floors: (1, 3),
            depth: None,
            tiled_roofs: Some(1.0),
            gaps: None,
            run: None,
        }],
        main_spacing: size,
        boundary: BoundaryInfluence {
            strength: 1.5,
            reach: 0.03 * size,
        },
        max_bridge: 80.0,
        centre: at(0.5, 0.5),
        tram_lines: Vec::new(),
        stop_spacing: 450.0,
        stop_target: 0.0,
        max_extra_lines: 0,
        boat_gap: f64::INFINITY,
        landmarks: Vec::new(),
        park_reach: 200.0,
        park_min_area: 2_000.0,
        park_share_cap: 0.15,
        forest_above: f32::MAX,
        forest_slope: f32::MAX,
        reach: Some(1.6),
        max_block: None,
        forest: Some(7.0),
    }
}

/// The village on `site` by `params`: the plan, then each built block's strips and houses, and
/// the houses as masses.
pub fn village(site: &Site, params: &CityParams, seed: u64) -> Village {
    let mut city = plan(site, params);
    // Old stone throughout, whatever the example's districts: the structures and the facades
    // take a building's era from its lot's district.
    for d in &mut city.districts {
        d.kind = DistrictKind::Village;
    }
    let streets = blocks::street_grid(&city.roads, city.size);
    let ty = ParcelType::VILLAGE;
    let mut blocks = Vec::new();
    let mut masses = Vec::new();
    let mut without = 0;
    for b in 0..city.blocks.len() {
        let block = &city.blocks[b];
        if block.use_ != BlockUse::Built {
            continue;
        }
        let n = block.buildable.len();
        // The village's depth first; a thin block takes a shallower strip, down to a house with
        // no yard; one too thin for that keeps no houses. Each lot as wide as its household's
        // rooms need on two storeys at that depth (the owner's way round: the rooms first, the
        // lot cut to the house), never narrower than the type's least.
        let cut = [ty.depth, 11.0, 8.0].into_iter().find_map(|depth| {
            let ty = ParcelType { depth, ..ty };
            let max_depth = houses::plan_depth(&ty);
            let width = |k: usize| {
                let (people, means) = household(seed, b, k);
                let area: f64 = rooms::programme(people, means).iter().map(|r| r.area).sum();
                (rooms::HALL + area / (2.0 * max_depth)).clamp(ty.width.0, 16.0)
            };
            parcels::strips_with(&block.buildable, &vec![true; n], &ty, width)
                .map(|(lots, court, keys)| (ty, lots, court, keys))
        });
        let Some((ty, lots, court, keys)) = cut else {
            without += 1;
            continue;
        };
        let houses = houses::houses(&lots, &ty, |k| household(seed, b, keys[k]));
        for h in &houses {
            masses.push(mass(&mut city, b, h, &streets));
        }
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
        masses,
    }
}

/// House `h` of block `b` as a mass on its lot, appended to the plan's lots: its storeys from
/// its plan at old stone's heights, a tiled roof, homes only.
fn mass(city: &mut CityPlan, b: usize, h: &House, streets: &PointGrid) -> Mass {
    let lot = city.lots.len() as u32;
    city.lots.push(Lot {
        block: b as u32,
        polygon: h.lot.clone(),
        area: geom::signed_area(&h.lot).abs(),
        frontage: true,
    });
    let mut footprint = h.footprint.clone();
    if geom::signed_area(&footprint) < 0.0 {
        footprint.reverse();
    }
    let floors = h.plan.storeys.len().clamp(1, 255) as u8;
    let storeys = Era::OldStone.storeys(floors, false);
    let height = storeys.iter().sum();
    let street_width = 2.0 * blocks::edge_half_width(streets, &city.roads, h.lot[0], h.lot[1]);
    Mass {
        lot,
        footprint,
        floors,
        storeys,
        height,
        roof: Roof::Tiled,
        street_width,
        open: false,
        active: false,
        offices: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::super::example;
    use super::super::structure::{self, OpeningKind, WallRole};
    use super::*;

    #[test]
    fn the_villages_own_plan_has_lanes_houses_and_no_trams() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village(&site, &params(size), 1);
        let built = v
            .plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Built)
            .count();
        let houses = v.houses().count();
        println!(
            "the village's own plan: {} nodes, {} edges, {} blocks ({built} built), {} with \
             strips, {} without, {houses} houses, {} people",
            v.plan.roads.nodes.len(),
            v.plan.roads.edges.len(),
            v.plan.blocks.len(),
            v.blocks.len(),
            v.without,
            v.houses().map(|h| h.household.0).sum::<u32>()
        );
        let trees = super::super::trees::trees(&site, &v.plan, 1);
        let forest = trees
            .iter()
            .filter(|t| t.setting == super::super::trees::Setting::Forest)
            .count();
        println!("{} trees, {forest} in the forest", trees.len());
        assert!(forest >= 500, "{forest} forest trees");
        assert!(v.plan.tram_lines.is_empty() && v.plan.landmarks.is_empty());
        assert!(built >= 4, "{built} built blocks");
        assert!(houses >= 40, "{houses} houses");
        if let Ok(path) = std::env::var("VILLAGE_OWN_PNG") {
            super::super::map::write_village(&v, std::path::Path::new(&path), 0.25)
                .expect("the drawing");
        }
    }

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
        // The masses and their structure (D-060) as the city's: each on its own lot, old stone,
        // a street wall each, most with a door and the storeyed ones with a stair.
        assert_eq!(v.masses.len(), houses);
        for (m, h) in v.masses.iter().zip(v.houses()) {
            assert_eq!(v.plan.lots[m.lot as usize].polygon, h.lot);
            assert!(geom::signed_area(&m.footprint) > 0.0);
            assert_eq!(usize::from(m.floors), h.plan.storeys.len());
        }
        let structures = structure::structures(&v.plan, &v.masses);
        assert_eq!(structures.len(), houses);
        let (mut doors, mut cores, mut storeyed, mut straight) = (0, 0, 0, 0);
        for s in &structures {
            assert_eq!(s.era, Era::OldStone);
            assert!(
                s.walls.iter().any(|w| w.role == WallRole::Street),
                "mass {} has no street wall",
                s.mass
            );
            doors += usize::from(s.openings.iter().any(|o| o.kind == OpeningKind::Door));
            cores += usize::from(!s.cores.is_empty());
            straight += usize::from(s.cores.iter().any(|c| c.house));
            storeyed += usize::from(s.storeys.len() >= 2);
        }
        let mut by_storeys = std::collections::BTreeMap::new();
        for h in v.houses() {
            *by_storeys.entry(h.plan.storeys.len()).or_insert(0) += 1;
        }
        println!("houses by storeys: {by_storeys:?}");
        println!(
            "{doors} houses with a door, {cores} with a stair core ({straight} with a house's \
             straight flight), {storeyed} storeyed"
        );
        assert!(
            cores * 10 >= storeyed * 9,
            "{cores} of {storeyed} storeyed houses have a stair"
        );
        assert!(
            doors * 10 >= houses * 9,
            "{doors} of {houses} houses have a door"
        );
        if let Ok(path) = std::env::var("VILLAGE_PNG") {
            super::super::map::write_village(&v, std::path::Path::new(&path), 0.25)
                .expect("the drawing");
        }
    }
}
