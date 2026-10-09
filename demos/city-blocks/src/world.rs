//! The island's world in the demo (#211): its description is `forge_terrain::world`'s, read
//! from the file `--world` names (`assets/worlds/island.toml` by default); the flags override it
//! for an experiment ([`apply_flags`]), and the log names each one.

use std::path::PathBuf;

use forge_core::Seed;
use forge_procgen::{RibbonParams, Wind};
pub(crate) use forge_terrain::world::{IslandWorld, set, world};

use crate::Args;

/// The world file `args` names (`--world`), or the island's.
pub(crate) fn path(args: &Args) -> PathBuf {
    args.world
        .clone()
        .unwrap_or_else(forge_terrain::world::island_file)
}

/// The flags that override the file, applied: returns each flag that changed it, for the log.
pub(crate) fn apply_flags(world: &mut IslandWorld, args: &Args) -> Vec<String> {
    let mut changed = Vec::new();
    let island = &mut world.island;
    if let Some(seed) = args.island
        && seed != island.seed.value()
    {
        island.seed = Seed::new(seed);
        changed.push(format!("--island {seed}"));
    }
    if let Some(spacing) = args.island_spacing {
        // The same extent at the new spacing.
        let extent = f64::from(island.size - 1) * island.spacing;
        island.size = (extent / spacing) as u32 + 1;
        island.spacing = spacing;
        changed.push(format!("--island-spacing {spacing}"));
    }
    for (field, flag, name) in [
        (&mut island.plain, args.island_plain, "island-plain"),
        (
            &mut island.plain_uplift,
            args.island_plain_uplift,
            "island-plain-uplift",
        ),
        (
            &mut island.plain_wander,
            args.island_plain_wander,
            "island-plain-wander",
        ),
        (&mut island.grade, args.island_grade, "island-grade"),
    ] {
        if let Some(value) = flag {
            *field = value;
            changed.push(format!("--{name} {value}"));
        }
    }
    if let Some(basins) = args.island_basins {
        island.basins = basins;
        changed.push(format!("--island-basins {basins}"));
    }
    if let Some(from) = &args.island_wind {
        let contrast = args
            .island_rain_contrast
            .or(island.wind.map(|w| w.contrast))
            .unwrap_or(1.0);
        island.wind = Wind::from_compass(from, contrast);
        if island.wind.is_none() {
            tracing::warn!(wind = %from, "unknown wind origin (n, ne, e, se, s, sw, w, nw): the rain stays flat");
        }
        changed.push(format!("--island-wind {from}"));
    } else if let Some(contrast) = args.island_rain_contrast
        && let Some(wind) = &mut island.wind
    {
        wind.contrast = contrast;
        changed.push(format!("--island-rain-contrast {contrast}"));
    }
    if let Some(steps) = args.island_steps {
        world.erosion.steps = steps;
        changed.push(format!("--island-steps {steps}"));
    }
    if let Some(ha) = args.island_channel_ha {
        world.erosion.channel_area = ha * 10_000.0;
        changed.push(format!("--island-channel-ha {ha}"));
    }
    let rivers = &mut world.rivers;
    if let Some(k) = args.river_k {
        let depth = rivers
            .regional
            .or(RibbonParams::island().regional)
            .map_or(0.0, |(_, depth)| depth);
        rivers.regional = (k > 0.0).then_some((k, depth));
        changed.push(format!("--river-k {k}"));
    }
    /// Turns `setting` off when `on`, noting `flag`.
    fn off<T>(setting: &mut Option<T>, on: bool, flag: &str, changed: &mut Vec<String>) {
        if on {
            *setting = None;
            changed.push(format!("--{flag}"));
        }
    }
    off(&mut rivers.steps, args.no_steps, "no-steps", &mut changed);
    off(
        &mut rivers.brooks,
        args.no_brooks,
        "no-brooks",
        &mut changed,
    );
    off(&mut rivers.delta, args.no_deltas, "no-deltas", &mut changed);
    off(&mut rivers.bars, args.no_bars, "no-bars", &mut changed);
    off(
        &mut rivers.confluence_scour,
        args.no_scour,
        "no-scour",
        &mut changed,
    );
    off(
        &mut rivers.confluence_bars,
        args.no_confluence_bars,
        "no-confluence-bars",
        &mut changed,
    );
    off(
        &mut rivers.distributaries,
        args.no_distributaries,
        "no-distributaries",
        &mut changed,
    );
    off(
        &mut world.channels.sill,
        args.no_sills,
        "no-sills",
        &mut changed,
    );
    off(
        &mut world.valleys,
        args.no_valleys,
        "no-valleys",
        &mut changed,
    );
    let layers = &mut world.layers;
    off(&mut layers.salt, args.no_salt, "no-salt", &mut changed);
    off(
        &mut layers.beaches,
        args.no_beach_types,
        "no-beach-types",
        &mut changed,
    );
    off(
        &mut layers.geology,
        args.no_rock_types,
        "no-rock-types",
        &mut changed,
    );
    off(
        &mut layers.rock_sites,
        args.no_rock_sites,
        "no-rock-sites",
        &mut changed,
    );
    if let Some(drawn) = args.island_drawn {
        world.ground.drawn_spacing = drawn;
        changed.push(format!("--island-drawn {drawn}"));
    }
    if let Some(detail) = args.island_detail {
        world.ground.detail = detail;
        changed.push(format!("--island-detail {detail}"));
    }
    changed
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn the_flags_override_the_world_and_are_named() {
        let args = Args::try_parse_from([
            "city-blocks",
            "--island",
            "7",
            "--no-salt",
            "--island-detail",
            "0.5",
        ])
        .expect("the test's arguments");
        let mut w = IslandWorld::default();
        let changed = apply_flags(&mut w, &args);
        assert_eq!(changed, ["--no-salt", "--island-detail 0.5"]);
        let mut expected = IslandWorld::default();
        expected.layers.salt = None;
        expected.ground.detail = 0.5;
        assert_eq!(w, expected);
    }
}
