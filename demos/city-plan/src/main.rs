//! `city-plan`: the engine's invented example city (`forge_procgen::city::example`) in 3D, its
//! masses, streets, parks, trams and water drawn by `city-blocks`' renderer (D-059,
//! `docs/demos/city-plan.md`). The app's flags apply (`--frames`, `--capture`, `--view`…).

#![forbid(unsafe_code)]

use clap::{CommandFactory, FromArgMatches, Parser};
use forge_app::FlyCamera;
use forge_procgen::city::{self, example, mass};
use glam::{DVec2, Vec3};

/// The shared app's flags and the plan's.
#[derive(Parser, Debug)]
struct Cli {
    #[command(flatten)]
    app: city_blocks::Args,
    /// The example's side, metres.
    #[arg(long, default_value_t = 4000.0)]
    size: f64,
    /// Start on the pavement of the avenue nearest the centre, at eye height.
    #[arg(long)]
    street: bool,
    /// Turn the street view this many degrees to the left (90 faces the far side's buildings,
    /// -90 the near side's).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    turn: f32,
    /// Build the masses within this many metres of the view from the style's grammar and kit
    /// (D-039); 0 draws every mass with facade textures.
    #[arg(long, default_value_t = 400.0)]
    detail: f64,
    /// A style set's rules in the grammar's text form; the engine's Mediterranean example
    /// when not given.
    #[arg(long)]
    rules: Option<std::path::PathBuf>,
    /// `x,z,lead`: start the trams' clock so that one passes nearest the world's point (x, z)
    /// `lead` seconds in (a capture's frame over 60).
    #[arg(long, value_delimiter = ',', allow_hyphen_values = true)]
    tram: Option<Vec<f64>>,
    /// Walk in first person from the start where the camera stands (Enter toggles it; the
    /// app's `--walk vx,vz` scripts the walk).
    #[arg(long)]
    walk_start: bool,
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
    let trees = city::trees::trees(&site, &plan, 5);
    let lamps = city::lamps::lamps(&site, &plan);
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
    let camera = if cli.street {
        city_plan::street_camera(&plan, centre).map_or(camera, |mut c| {
            c.yaw += cli.turn.to_radians();
            c
        })
    } else {
        camera
    };
    // The kit round the street view, or round the centre seen from the air.
    let text = match &cli.rules {
        Some(path) => std::fs::read_to_string(path)?,
        None => city::facades::MEDITERRANEAN.to_owned(),
    };
    let grammar = forge_procgen::grammar::Grammar::parse(&text)?;
    let half = cli.size * 0.5;
    let detail = city_plan::Detail {
        grammar: &grammar,
        start: "Facade",
        top: 0.9,
        centre: if cli.street {
            DVec2::new(
                f64::from(camera.position.x) + half,
                f64::from(camera.position.z) + half,
            )
        } else {
            centre
        },
        radius: cli.detail,
        seed: 12,
    };
    let scenario = city_plan::PlanScenario::detailed(
        &site,
        &plan,
        &masses,
        &trees,
        &lamps,
        camera,
        "city plan",
        (cli.detail > 0.0).then_some(&detail),
    );
    let scenario = match cli.tram.as_deref() {
        Some(&[x, z, lead]) => {
            let at = DVec2::new(x + half, z + half);
            match scenario.tram_near(at, lead) {
                Some(clock) => scenario.with_clock(clock),
                None => scenario,
            }
        }
        Some(_) => anyhow::bail!("--tram takes x,z,lead"),
        None => scenario,
    };
    let scenario = if cli.walk_start {
        scenario.walking()
    } else {
        scenario
    };
    city_blocks::run(cli.app, "forge city-plan", Some(Box::new(scenario)))
}
