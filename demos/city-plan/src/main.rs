//! `city-plan`: the engine's invented example city (`forge_procgen::city::example`) in 3D, its
//! masses, streets, parks, trams and water drawn by `city-blocks`' renderer (D-059,
//! `docs/demos/city-plan.md`). The app's flags apply (`--frames`, `--capture`, `--view`…).

#![forbid(unsafe_code)]

use clap::{CommandFactory, FromArgMatches, Parser};
use forge_app::FlyCamera;
use forge_procgen::city::{self, example, mass};
use glam::Vec3;

/// The shared app's flags and the plan's.
#[derive(Parser, Debug)]
struct Cli {
    #[command(flatten)]
    app: city_blocks::Args,
    /// The example's side, metres.
    #[arg(long, default_value_t = 4000.0)]
    size: f64,
}

fn main() -> anyhow::Result<()> {
    let matches = Cli::command()
        .name("city-plan")
        .about("A city's plan in 3D: the example city's masses, streets, parks and trams")
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let site = example::site(cli.size, 10.0);
    let params = example::params(cli.size);
    let plan = city::plan(&site, &params);
    let masses = mass::masses(&plan, &city::bible::StyleBible::default(), 11);
    // Over the river south of the centre, looking north across it.
    let centre = params.centre;
    let camera = FlyCamera {
        position: Vec3::new(
            (centre.x - cli.size * 0.5) as f32,
            180.0,
            (centre.y - cli.size * 0.5) as f32 + 700.0,
        ),
        yaw: 0.0,
        pitch: -0.32,
        speed: 40.0,
        ..FlyCamera::default()
    };
    let scenario = city_plan::PlanScenario::new(&site, &plan, &masses, camera, "city plan");
    city_blocks::run(cli.app, "forge city-plan", Some(Box::new(scenario)))
}
