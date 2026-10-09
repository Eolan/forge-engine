//! `physics-lab` — Phase 3's first demo (issue #136, `docs/demos/physics-lab.md`): small scenes
//! of rigid bodies, characters, vehicles, liquids and soft bodies through `forge-physics` (Jolt
//! 5.6, D-009), each a test with its numbers and its determinism hash. The scenes are this
//! crate's (`lab`); `city-blocks`' shared app draws them and runs their frames through its scene
//! interface (#216, `city_blocks::scenario`), with its renderer and flags. `--lab` picks the scene
//! (`drop` by default). Space throws a ball from the camera, Enter starts the scene over.

#![forbid(unsafe_code)]

use clap::{CommandFactory, FromArgMatches, Parser};

mod lab;

/// The shared app's flags and the lab's.
#[derive(Parser, Debug)]
struct Cli {
    #[command(flatten)]
    app: city_blocks::Args,
    #[command(flatten)]
    lab: lab::LabArgs,
}

fn main() -> anyhow::Result<()> {
    let matches = Cli::command()
        .name("physics-lab")
        .about("The physics lab: rigid bodies through Jolt, one test scene at a time")
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let kind = cli.lab.scene.unwrap_or(lab::LabScene::Drop);
    city_blocks::run(
        cli.app,
        "forge physics-lab",
        Some(Box::new(lab::LabScenario::new(kind, cli.lab))),
    )
}
