//! A world's description (#211, D-053): every setting that decides what the island is, read
//! from a TOML file (the island's: `assets/worlds/island.toml`, [`island_file`]).
//!
//! - **The file** is merged over [`IslandWorld::default`], the code's values, so it may name
//!   only what differs. A key the description does not have is an error, never ignored: a typo
//!   can't pass silently. A setting that can be off is `false` when off (`forge_core::switch`).
//! - **A demo's flags** may override it for an experiment before [`set`] (`city-blocks`'
//!   `--island-*` flags).
//! - **The keys** of the island's stored products (#208) read the parts they depend on, so
//!   editing the file remakes only what the edit reaches.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use forge_core::Seed;
use forge_procgen::{
    BeachRule, ChannelParams, ErosionParams, GeologyRule, IslandParams, RibbonParams, RockSiteRule,
    SaltRule, ScrubRule, ValleyGround, ValleyParams,
};
use serde::{Deserialize, Serialize};

/// The island's world: what it is made from, stage by stage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IslandWorld {
    /// The island's shape and its uplift (`forge_procgen::IslandParams`): the seed, the samples
    /// and their spacing, the coast, the ridges, the coastal plain, the basins, the rain.
    pub island: IslandParams,
    /// The erosion that carves it (`forge_procgen::ErosionParams`).
    pub erosion: ErosionParams,
    /// Its rivers as ribbons of water (`forge_procgen::RibbonParams`): steps, deltas, bars,
    /// confluences and distributaries, each `false` to leave it out.
    pub rivers: RibbonParams,
    /// The channels carved for them (`forge_procgen::ChannelParams`): `sill = false` leaves
    /// the lakes' outlets without sills.
    pub channels: ChannelParams,
    /// The rivers' valleys (`forge_procgen::ValleyParams`): `false` leaves the rivers in the
    /// valleys the erosion cut.
    #[serde(with = "forge_core::switch")]
    pub valleys: Option<ValleyParams>,
    /// The ground as drawn.
    pub ground: Ground,
    /// The ground's layers and where its loose rocks lie.
    pub layers: Layers,
}

/// The ground as drawn: the field shaped for the sea and drawn finer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ground {
    /// Metres between the drawn ground's samples (#106): the field's spacing divided by 1, 2, 4
    /// or 8.
    pub drawn_spacing: f64,
    /// How much of the amplification's detail the drawn ground takes away from the water
    /// (#106, `forge_procgen::amplify` at each halving of the spacing): 1 all of it, 0 none.
    pub detail: f32,
    /// Passes of the binomial filter over the whole field (#106): the 8 m erosion leaves
    /// steps two samples apart on the slopes, which one pass takes out.
    pub smoothing: u32,
    /// The sea floor (`forge_procgen::sea_floor`): metres of depth it levels off at, and the
    /// metres from the coast that set its slope (60 over 1 500: 4 % at the shore).
    pub sea_floor: (f32, f32),
    /// The shore's smoothing (`forge_procgen::smooth_shore`, #106): the samples within this
    /// many metres of the sea's level (the sand's top among them), and the passes of the
    /// binomial filter.
    pub shore_smoothing: (f32, u32),
    /// Metres past the water's reach over which the amplification's detail fades in (#106):
    /// the channels, the lakes' shores and the beaches keep the ground the water was made for.
    pub detail_fade: (f32, f32),
}

/// The ground's layers (the layer map's rules) and where its loose rocks lie.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layers {
    /// Texels a side of the layer map: one every 4 m over the 16 km.
    pub texels: u32,
    /// The slope (rise over run) over which the ground is rock.
    pub rock_slope: f32,
    /// Metres over which the slope is measured, whatever the spacing.
    pub slope_over: f64,
    /// Metres above the sea under which the gentle ground is sand (the layer map's rule, and
    /// the ground's contour under each pixel).
    pub sand_below: f32,
    /// Metres the sand's top wanders up and down along the coast (#106).
    pub sand_wander: f32,
    /// The rivers' riparian strip (D-041): metres past the water, plus this many of the
    /// river's widths.
    pub riparian_strip: (f64, f64),
    /// Metres of a lake's water over the ground from which the map paints its bed of mud
    /// (#114).
    pub lakebed_under: f64,
    /// The salt water's sand (`forge_procgen::SaltRule`), or `false`.
    #[serde(with = "forge_core::switch")]
    pub salt: Option<SaltRule>,
    /// The beach types (#128, `forge_procgen::BeachRule`), or `false` for sand everywhere.
    #[serde(with = "forge_core::switch")]
    pub beaches: Option<BeachRule>,
    /// The scrub (`forge_procgen::ScrubRule`).
    pub scrub: ScrubRule,
    /// The valleys' ground (`forge_procgen::ValleyGround`).
    pub valley_ground: ValleyGround,
    /// The rock types (#129, D-042, `forge_procgen::GeologyRule`), or `false` for one rock.
    #[serde(with = "forge_core::switch")]
    pub geology: Option<GeologyRule>,
    /// Where the loose rocks lie (#130, `forge_procgen::RockSiteRule`), or `false`.
    #[serde(with = "forge_core::switch")]
    pub rock_sites: Option<RockSiteRule>,
}

impl Default for IslandWorld {
    /// The island as the code makes it: seed 7, 16 km at 8 m.
    fn default() -> Self {
        Self {
            island: IslandParams::island_16km(Seed::new(7), 8.0),
            erosion: ErosionParams::island(),
            rivers: RibbonParams::island(),
            channels: ChannelParams::default(),
            valleys: Some(ValleyParams::default()),
            ground: Ground {
                drawn_spacing: 2.0,
                detail: 1.0,
                smoothing: 1,
                sea_floor: (60.0, 1500.0),
                shore_smoothing: (3.5, 4),
                detail_fade: (4.0, 32.0),
            },
            layers: Layers {
                texels: 4096,
                rock_slope: 0.45,
                slope_over: 8.0,
                sand_below: 2.5,
                sand_wander: 0.3,
                riparian_strip: (6.0, 2.0),
                lakebed_under: 1.0,
                salt: Some(SaltRule::default()),
                beaches: Some(BeachRule::default()),
                scrub: ScrubRule::default(),
                valley_ground: ValleyGround::default(),
                geology: Some(GeologyRule::default()),
                rock_sites: Some(RockSiteRule::default()),
            },
        }
    }
}

impl IslandWorld {
    /// Reads `text` over the code's values.
    pub fn parse(text: &str) -> Result<Self> {
        let file: toml::Table = text.parse().context("not TOML")?;
        let mut merged =
            toml::Table::try_from(Self::default()).context("the code's world as TOML")?;
        merge(&mut merged, file);
        let world: Self = toml::Value::Table(merged).try_into()?;
        world.check()?;
        Ok(world)
    }

    /// Reads the file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("the world file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("the world file {}", path.display()))
    }

    /// The description as TOML, every setting named.
    pub fn to_toml(&self) -> Result<String> {
        let mut table = toml::Table::try_from(self)?;
        shorten_floats(&mut table);
        Ok(toml::to_string(&table)?)
    }

    /// What the makers assume of it.
    fn check(&self) -> Result<()> {
        let island = &self.island;
        let factor = (island.spacing / self.ground.drawn_spacing).round();
        if !(factor >= 1.0 && (self.channels.split as f64 % factor) == 0.0) {
            bail!(
                "ground.drawn_spacing must divide island.spacing ({} m) by 1, 2, 4 or 8",
                island.spacing
            );
        }
        if island.size < 2 || island.spacing <= 0.0 {
            bail!("island.size must be 2 or more and island.spacing positive");
        }
        Ok(())
    }
}

/// `over`'s keys into `base`: tables merged key by key, anything else replaced.
pub(crate) fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(inner)), toml::Value::Table(over)) => merge(inner, over),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Writes the `f32` settings as the `f32`s they are (`0.45`, not `0.44999998807907104`): a
/// float that an `f32` holds exactly takes the `f32`'s shortest text. An `f64` setting that
/// happens to be such a float reads back the same `f64` only if its text was already that
/// short, which the round-trip test checks for the island's.
pub(crate) fn shorten_floats(table: &mut toml::Table) {
    fn shorten(value: &mut toml::Value) {
        match value {
            toml::Value::Float(x) => {
                let single = *x as f32;
                if f64::from(single) == *x
                    && let Ok(short) = single.to_string().parse::<f64>()
                {
                    *x = short;
                }
            }
            toml::Value::Array(items) => items.iter_mut().for_each(shorten),
            toml::Value::Table(table) => table.iter_mut().for_each(|(_, v)| shorten(v)),
            _ => {}
        }
    }
    table.iter_mut().for_each(|(_, v)| shorten(v));
}

/// The world this process makes, once [`set`] has run; the code's values before (tests).
pub fn world() -> &'static IslandWorld {
    static DEFAULT: OnceLock<IslandWorld> = OnceLock::new();
    WORLD
        .get()
        .unwrap_or_else(|| DEFAULT.get_or_init(IslandWorld::default))
}

static WORLD: OnceLock<IslandWorld> = OnceLock::new();

/// Sets the process's world, once.
pub fn set(world: IslandWorld) {
    WORLD.set(world).expect("the world, set once");
}

/// The island's world file: `assets/worlds/island.toml` in the workspace.
pub fn island_file() -> PathBuf {
    crate::workspace_root().join("assets/worlds/island.toml")
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_procgen::Wind;

    #[test]
    fn the_world_reads_back_as_it_was_written() {
        let world = IslandWorld::default();
        let text = world.to_toml().unwrap();
        assert_eq!(IslandWorld::parse(&text).unwrap(), world);
        // Off settings too.
        let mut off = world.clone();
        off.rivers.steps = None;
        off.valleys = None;
        off.island.wind = Wind::from_compass("sw", 0.5);
        assert_eq!(IslandWorld::parse(&off.to_toml().unwrap()).unwrap(), off);
    }

    #[test]
    fn the_file_names_only_what_differs_and_nothing_unknown() {
        let world = IslandWorld::parse("[island]\nseed = 9\n[rivers]\nsteps = false\n").unwrap();
        let mut expected = IslandWorld::default();
        expected.island.seed = Seed::new(9);
        expected.rivers.steps = None;
        assert_eq!(world, expected);
        // A setting's partial table keeps the others.
        let world = IslandWorld::parse("[rivers.steps]\nfrom = 0.5\n").unwrap();
        assert_eq!(world.rivers.steps.unwrap().from, 0.5);
        assert_eq!(
            world.rivers.steps.unwrap().lip,
            RibbonParams::island().steps.unwrap().lip
        );
        // Unknown keys and wrong types are errors.
        for bad in [
            "[island]\nseeed = 9\n",
            "[nowhere]\nx = 1\n",
            "[rivers.steps]\nfrm = 0.5\n",
            "[ground]\ndetail = \"high\"\n",
            "[rivers]\nsteps = true\n",
        ] {
            assert!(IslandWorld::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_islands_world_file_reads() {
        IslandWorld::load(&island_file()).unwrap();
    }
}
