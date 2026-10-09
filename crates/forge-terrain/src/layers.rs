//! The island's ground layers (D-041, D-042): the layer map its painters make from the heights
//! and the water, and where its loose rocks lie (#130).

use std::sync::Arc;
use std::time::Instant;

use forge_procgen::Field2;
use forge_task::TaskPool;

use crate::heights::{island_heights, island_lakes};
use crate::keys::{derived_cache, layers_key};
use crate::water::{IslandWater, island_water};
use crate::world::world;

/// The island's ground layers (`CityMaterials::island_ground`, `forge_procgen::slope_layers`).
pub mod island_layer {
    /// Grass, on the gentle ground above the beaches.
    pub const GRASS: u8 = 0;
    /// Sand, on the land's first metres above the sea.
    pub const SAND: u8 = 1;
    /// The sea floor, under the sea's plane (wet sand).
    pub const SEABED: u8 = 2;
    /// Rock, where the ground is steep.
    pub const ROCK: u8 = 3;
    /// A river, painted over the others at its width (`forge_procgen::paint_rivers`).
    pub const STREAM: u8 = 4;
    /// Grass on the driest ground: the ridges (`forge_procgen::paint_moisture`).
    pub const DRY_GRASS: u8 = 5;
    /// Grass on the wettest ground: the valley bottoms.
    pub const LUSH_GRASS: u8 = 6;
    /// The rivers' banks: reeds, sedges and shrubs on the grass within a few of a river's
    /// widths (D-041's riparian strip, `forge_procgen::paint_banks`). Its id was the rivers'
    /// bed's, which the water draws per pixel since #114.
    pub const RIVERBANK: u8 = 7;
    /// A lake's bed of dark mud, under a metre or more of its water (unless `--no-water`,
    /// `forge_procgen::paint_lake_beds`).
    pub const LAKEBED: u8 = 8;
    /// Cobbles and gravel: the beds and floors of the steeper rivers' reaches, under their water
    /// and beside it (#118, `forge_procgen::paint_valley_ground`).
    pub const GRAVEL: u8 = 9;
    /// Scree: broken rock at the foot of the steep walls of those rivers' valleys (#118).
    pub const SCREE: u8 = 10;
    /// Scrub: low shrubs on the steep ground that holds soil, the wetter rock (#118,
    /// `forge_procgen::paint_scrub`).
    pub const SCRUB: u8 = 11;
    /// Pale silty sand on the tops of the rivers' deltas' fans, under the lakes' shallow water in
    /// front of their mouths (#120, `forge_procgen::paint_fans`). Not the beaches' sand, whose
    /// top follows the coast's contour.
    pub const LAKE_SAND: u8 = 12;
    /// Shingle: the pebbles of the beaches on the headlands and under steep land (#128,
    /// `forge_procgen::paint_beaches`).
    pub const SHINGLE: u8 = 13;
    /// Limestone: the rock of the low ground and the sea cliffs, the old reefs raised with the
    /// island (D-042, #129, `forge_procgen::paint_geology`). The rock over it, `ROCK`, is the
    /// hills' granite.
    pub const LIMESTONE: u8 = 14;
    /// Karst: the limestone's bare pavements of blocks and fissures, on its driest gentler
    /// ground (#129).
    pub const KARST: u8 = 15;
    /// Grus: the coarse sand the granite rots into, on its gentle ground near the bare
    /// rock (#135).
    pub const GRUS: u8 = 16;
    /// How many layers there are.
    pub const COUNT: u8 = 17;
}

/// The island's ground layers and where its loose rocks lie (#130): what the CPU paints for
/// `build_island`, in `make_island_layers`.
pub struct IslandLayers {
    /// The layer map: a layer id ([`island_layer`]) a texel.
    pub layers: Field2<u8>,
    /// The rock sites' map, its numbers and the milliseconds it took (`--no-rock-sites`: none).
    pub sites: Option<(Field2<u8>, forge_procgen::RockSiteStats, u128)>,
}

/// [`IslandLayers`], made once a process for the island and the arguments that change it: the
/// loading thread makes it (#201), so the finishing step on the main thread finds it. With
/// `water` the water draws the rivers' and lakes' beds; without it, the map paints stand-ins.
pub fn island_layers(water: bool) -> Arc<IslandLayers> {
    static MADE: std::sync::Mutex<Option<(u64, Arc<IslandLayers>)>> = std::sync::Mutex::new(None);
    let height = island_heights();
    let flags = [
        water,
        world().layers.salt.is_none(),
        world().layers.beaches.is_none(),
        world().layers.geology.is_none(),
        world().layers.rock_sites.is_none(),
    ];
    let key = flags
        .iter()
        .enumerate()
        .fold(height.digest() ^ u64::from(height.size), |key, (k, &on)| {
            key ^ (u64::from(on) << (56 + k))
        });
    let mut made = MADE.lock().expect("the island's layers");
    if let Some((made_for, layers)) = made.as_ref()
        && *made_for == key
    {
        return layers.clone();
    }
    let derived = derived_cache().get_or_make("island-layers", layers_key(world(), water), || {
        make_island_layers(water, &height, world().layers.texels)
    });
    tracing::info!(
        from_cache = derived.from_cache,
        ms = derived.ms,
        "the island's layers"
    );
    let layers = Arc::new(derived.value);
    *made = Some((key, layers.clone()));
    layers
}

forge_core::stored!(IslandLayers { layers, sites });

/// [`island_layers`], made: the slope rule's layers, then the moisture, the salt water's sand,
/// the beaches, the rivers' and lakes' layers, the valleys' ground and the geology, and the rock
/// sites from them.
fn make_island_layers(water: bool, height: &Field2<f32>, texels: u32) -> IslandLayers {
    let height = height.clone();
    let layers_start = Instant::now();
    let mut layers = forge_procgen::slope_layers(
        &height,
        &forge_procgen::LayerRule {
            grass: island_layer::GRASS,
            rock: island_layer::ROCK,
            rock_slope: world().layers.rock_slope,
            // Green to the peaks, as on a tropical island: rock where it is steep.
            rock_above: f32::INFINITY,
            // The slope over 8 m whatever the spacing, so 4 m draws the rock 8 m draws.
            slope_over: world().layers.slope_over,
            shore: Some(forge_procgen::Shore {
                sea: island_layer::SEABED,
                sand: island_layer::SAND,
                sand_below: world().layers.sand_below,
            }),
        },
        texels,
    );
    tracing::info!(
        texels,
        ms = layers_start.elapsed().as_millis(),
        "island layer map"
    );
    // The moisture, the rivers and the lakes from the drawn field's drainage.
    let rivers_start = Instant::now();
    let flow = forge_procgen::drain(&height, 0.0, &TaskPool::client());
    // First the grass by its moisture (the topographic wetness index, blurred over 32 m): the
    // driest quarter dry grass on the ridges, the wettest quarter lush in the valley bottoms.
    let wetness = forge_procgen::wetness(&height, &flow, 4);
    let (dried, greened) = forge_procgen::paint_moisture(
        &mut layers,
        &wetness,
        island_layer::GRASS,
        (island_layer::DRY_GRASS, 0.25),
        (island_layer::LUSH_GRASS, 0.25),
    );
    // The rivers run in the channels carved for them, which the island's mesh draws (#105).
    // With the water the map paints no bed: a pixel blends the four texels around it and the
    // lookup wanders by one, so a bed's 4 m texels showed up to 8 m onto the banks, wider than
    // most of the rivers (#114); the water turns the ground it covers into its bed itself, per
    // pixel (`fresh_water` in `water.slang`). Without it, their stand-in painted at the
    // water's width.
    let IslandWater {
        ribbons,
        channels,
        lakes: lake_waters,
    } = island_water(&height);
    // The salt water keeps the grass off the banks beside it (#199, the owner's note: "grass
    // don't like the salted water much"): sand up to 5 m over the sea at the water, falling to
    // the beach's top 40 m from it, on the sea's slopes and the rivers' reaches it fills.
    if let Some(salt) = &world().layers.salt {
        let salt_start = Instant::now();
        let salted = forge_procgen::paint_salt(
            &mut layers,
            &|x, y| channels.height_at(&height, x, y),
            &[
                island_layer::GRASS,
                island_layer::DRY_GRASS,
                island_layer::LUSH_GRASS,
            ],
            island_layer::SEABED,
            island_layer::SAND,
            f64::from(world().layers.sand_below),
            salt,
        );
        tracing::info!(
            salted,
            ms = salt_start.elapsed().as_millis(),
            "the salt water's sand (#199, --no-salt)"
        );
    }
    // The beaches by the coast (#128): shingle on the headlands and under steep land, pale
    // sand in the bays and by the rivers' mouths.
    if let Some(beaches) = &world().layers.beaches {
        let beaches_start = Instant::now();
        let settings = world().island;
        let mouths: Vec<[f64; 2]> = ribbons
            .iter()
            .filter_map(|r| {
                let p = r.points[forge_procgen::sea_mouth(&r.points)?].position;
                Some([f64::from(p[0]), f64::from(p[1])])
            })
            .collect();
        let beaches = forge_procgen::paint_beaches(
            &mut layers,
            &height,
            &|x, y| forge_procgen::island::island_hardness(&settings, x, y),
            &mouths,
            forge_procgen::BeachLayers {
                sand: island_layer::SAND,
                sea: island_layer::SEABED,
                shingle: island_layer::SHINGLE,
                // No black sand: the island's hard rock is no basalt (the owner, 2026-10-02).
                black: None,
            },
            beaches,
        );
        let km = |k: usize| format!("{:.1}", beaches.coast_m[k] / 1000.0);
        // A view of each: the block of 256 m with the most of it, from 70 m out at sea and 18 m
        // up, looking back at its nearest texel to the block's middle.
        let size = layers.size;
        let cell = layers.spacing;
        let half_m = height.extent() * 0.5;
        let views: Vec<String> = [island_layer::SAND, island_layer::SHINGLE]
            .iter()
            .filter_map(|&layer| {
                let block = 64;
                let blocks = size / block;
                let (bx, by) = (0..blocks * blocks)
                    .map(|b| (b % blocks, b / blocks))
                    .max_by_key(|&(bx, by)| {
                        (0..block * block)
                            .filter(|t| {
                                layers.get(bx * block + t % block, by * block + t / block) == layer
                            })
                            .count()
                    })?;
                let middle = [
                    (bx * block + block / 2) as f64,
                    (by * block + block / 2) as f64,
                ];
                let (tx, ty) = (0..block * block)
                    .map(|t| (bx * block + t % block, by * block + t / block))
                    .filter(|&(x, y)| layers.get(x, y) == layer)
                    .min_by(|a, b| {
                        let d = |p: (u32, u32)| {
                            (f64::from(p.0) - middle[0]).hypot(f64::from(p.1) - middle[1])
                        };
                        d(*a).total_cmp(&d(*b))
                    })?;
                let at = [(f64::from(tx) + 0.5) * cell, (f64::from(ty) + 0.5) * cell];
                // Out to sea: down the ground's slope there.
                let step = 8.0;
                let down = [
                    height.sample(at[0] - step, at[1]) - height.sample(at[0] + step, at[1]),
                    height.sample(at[0], at[1] - step) - height.sample(at[0], at[1] + step),
                ];
                let len = f64::from(down[0].hypot(down[1])).max(1e-6);
                let out = [f64::from(down[0]) / len, f64::from(down[1]) / len];
                let eye = [at[0] + 70.0 * out[0], at[1] + 70.0 * out[1]];
                let yaw = out[0].atan2(out[1]).to_degrees();
                let pitch = (-(18.0_f64).atan2(70.0)).to_degrees();
                Some(format!(
                    "{:.0},18,{:.0},{yaw:.1},{pitch:.1}",
                    eye[0] - half_m,
                    eye[1] - half_m
                ))
            })
            .collect();
        tracing::info!(
            sand_km = %km(0),
            shingle_km = %km(1),
            texels = ?beaches.texels,
            under_sea = beaches.under_sea,
            ms = beaches_start.elapsed().as_millis(),
            views = %views.join("  "),
            "the island's beaches: sand, shingle (#128, --view)"
        );
    }
    let painted = if water {
        0
    } else {
        forge_procgen::paint_beds(&mut layers, &ribbons, island_layer::STREAM, 0.5)
    };
    // Along them the banks' reeds and shrubs (D-041's riparian strip): the grasses within 6 m
    // plus two of the river's widths of its water (and under 2.5 m the sand's contour still
    // takes them, per pixel).
    let banks = forge_procgen::paint_banks(
        &mut layers,
        &ribbons,
        &[
            island_layer::GRASS,
            island_layer::DRY_GRASS,
            island_layer::LUSH_GRASS,
        ],
        island_layer::RIVERBANK,
        world().layers.riparian_strip,
    );
    // And its lakes of a hectare or more: with the water, their beds of silt wherever the
    // lakes' planes stand a metre or more over the ground (their shallows, like the rivers,
    // the ground under the water's own bed); without it, on the stream's layer.
    let lakes = island_lakes(&height, &flow);
    let lake_texels = if water {
        forge_procgen::paint_lake_beds(
            &mut layers,
            &height,
            &|x, y| channels.height_at(&height, x, y) + world().layers.lakebed_under,
            &lake_waters,
            island_layer::LAKEBED,
        )
    } else {
        forge_procgen::paint_lakes(
            &mut layers,
            &lakes,
            (height.size, height.spacing),
            island_layer::STREAM,
            10_000.0,
            0.5,
        )
    };
    // The rivers' deltas (#120): the pale sand they lay on the lakes' floors in front of their
    // mouths, over the mud, under the water.
    let fan_texels = if water {
        forge_procgen::paint_fans(
            &mut layers,
            &ribbons,
            &|x, y| channels.height_at(&height, x, y),
            island_layer::LAKE_SAND,
        )
    } else {
        0
    };
    // The bars in the large mouths at the sea (#127): the beach's sand.
    let bar_texels = if water {
        forge_procgen::paint_bars(&mut layers, &ribbons, island_layer::SAND)
    } else {
        0
    };
    // The bars the confluences lay along the bank past their corner (#119's polish): the sand of
    // the mouths' bars (the deltas' sand, made to lie under water, reads as a dark stain in the
    // sun).
    let confluence_bar_texels = if water {
        forge_procgen::paint_confluence_bars(
            &mut layers,
            &ribbons,
            &|x, y| channels.height_at(&height, x, y),
            island_layer::SAND,
        )
    } else {
        0
    };
    // The steep ground's scrub (#118): plants on the wetter rock, the hollows and the valleys'
    // sides, in patches; the dry spurs and the cliffs stay bare.
    let scrubbed = forge_procgen::paint_scrub(
        &mut layers,
        &height,
        &wetness,
        (island_layer::ROCK, island_layer::SCRUB),
        &world().layers.scrub,
    );
    // The steeper rivers' valleys (#118): gravel in their beds and beside their water, scree at
    // the foot of their walls, scrub on the rock of the walls above.
    let valleys = forge_procgen::paint_valley_ground(
        &mut layers,
        &height,
        &ribbons,
        &[
            island_layer::GRASS,
            island_layer::DRY_GRASS,
            island_layer::LUSH_GRASS,
            island_layer::RIVERBANK,
            island_layer::ROCK,
            island_layer::SCRUB,
        ],
        &forge_procgen::ValleyGround {
            gravel: island_layer::GRAVEL,
            scree: island_layer::SCREE,
            scrub: island_layer::SCRUB,
            rock: island_layer::ROCK,
            ..world().layers.valley_ground
        },
    );
    tracing::info!(
        rivers = ribbons.len(),
        texels = painted,
        bank_texels = banks,
        scrub_texels = %format_args!("{scrubbed} in hollows, {} on valley walls", valleys.scrub),
        gravel_texels = valleys.gravel,
        scree_texels = valleys.scree,
        lakes = lakes
            .lakes
            .iter()
            .filter(|l| l.area(height.spacing) >= 10_000.0 && l.level > 0.5)
            .count(),
        lake_texels,
        fan_texels,
        bar_texels,
        confluence_bar_texels,
        dry_texels = dried,
        lush_texels = greened,
        ms = rivers_start.elapsed().as_millis(),
        "island moisture, rivers and lakes"
    );
    // The rock by the island's geology (D-042, #129), after the rules that read the rock: the
    // hills' granite, the low ground's limestone, and karst on the limestone's dry ground, grus on
    // the granite's gentle ground near its bare rock (#135).
    if let Some(geology) = &world().layers.geology {
        let geology_start = Instant::now();
        let rocks = forge_procgen::paint_geology(
            &mut layers,
            &height,
            forge_procgen::GeologyLayers {
                rock: island_layer::ROCK,
                limestone: island_layer::LIMESTONE,
                grass: island_layer::GRASS,
                dry_grass: island_layer::DRY_GRASS,
                karst: island_layer::KARST,
                grus: island_layer::GRUS,
            },
            geology,
        );
        // A view of each: the block of 256 m with the most of it, from 150 m down the ground's
        // slope from its nearest texel to the block's middle and 50 m over it, looking back.
        let half_m = height.extent() * 0.5;
        let view = |layer: u8| -> Option<String> {
            let (block, cell) = (64, layers.spacing);
            let blocks = layers.size / block;
            let mut count = vec![0_u32; (blocks * blocks) as usize];
            for (t, &l) in layers.data.iter().enumerate() {
                if l == layer {
                    let (x, y) = (t as u32 % layers.size, t as u32 / layers.size);
                    count[((y / block) * blocks + x / block) as usize] += 1;
                }
            }
            let b = (0..count.len()).max_by_key(|&b| count[b])?;
            let (bx, by) = (b as u32 % blocks, b as u32 / blocks);
            let middle = (
                (bx * block + block / 2) as f64,
                (by * block + block / 2) as f64,
            );
            let (tx, ty) = (0..block * block)
                .map(|t| (bx * block + t % block, by * block + t / block))
                .filter(|&(x, y)| layers.get(x, y) == layer)
                .min_by(|a, b| {
                    let d = |p: (u32, u32)| {
                        (f64::from(p.0) - middle.0).hypot(f64::from(p.1) - middle.1)
                    };
                    d(*a).total_cmp(&d(*b))
                })?;
            let at = ((f64::from(tx) + 0.5) * cell, (f64::from(ty) + 0.5) * cell);
            let ground = f64::from(height.sample(at.0, at.1));
            let step = 8.0;
            let down = (
                f64::from(height.sample(at.0 - step, at.1) - height.sample(at.0 + step, at.1)),
                f64::from(height.sample(at.0, at.1 - step) - height.sample(at.0, at.1 + step)),
            );
            let len = down.0.hypot(down.1).max(1e-6);
            let out = (down.0 / len, down.1 / len);
            let yaw = out.0.atan2(out.1).to_degrees();
            let pitch = (-(50.0_f64).atan2(150.0)).to_degrees();
            Some(format!(
                "{:.0},{:.0},{:.0},{yaw:.1},{pitch:.1}",
                at.0 + 150.0 * out.0 - half_m,
                ground + 50.0,
                at.1 + 150.0 * out.1 - half_m
            ))
        };
        let views: Vec<String> = [
            island_layer::ROCK,
            island_layer::LIMESTONE,
            island_layer::KARST,
            island_layer::GRUS,
        ]
        .iter()
        .filter_map(|&l| view(l))
        .collect();
        tracing::info!(
            granite_texels = rocks.granite,
            limestone_texels = rocks.limestone,
            karst_texels = rocks.karst,
            grus_texels = rocks.grus,
            ms = geology_start.elapsed().as_millis(),
            views = %views.join("  "),
            "the island's rocks: granite, limestone, karst, grus (D-042, #129, #135, --view)"
        );
    }
    // Where the loose rocks lie, and which rock they are (#130): the map the placement draws
    // them from.
    let sites = world().layers.rock_sites.map(|rule| {
        let sites_start = Instant::now();
        let (map, stats) = forge_procgen::rock_sites(
            &height,
            &layers,
            &forge_procgen::SiteLayers {
                scree: island_layer::SCREE,
                karst: island_layer::KARST,
                none: vec![
                    island_layer::SAND,
                    island_layer::SEABED,
                    island_layer::STREAM,
                    island_layer::LAKEBED,
                    island_layer::GRAVEL,
                    island_layer::LAKE_SAND,
                    island_layer::SHINGLE,
                ],
            },
            &world().layers.geology.unwrap_or_default(),
            &rule,
        );
        (map, stats, sites_start.elapsed().as_millis())
    });
    IslandLayers { layers, sites }
}
