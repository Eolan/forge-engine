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
    /// The first map's village (D-062, #266) in place of the city: the example site at this
    /// size, metres (600 for the first map), each house from its rooms on its strip lot, old
    /// stone and tiled, the kit dressing them all.
    #[arg(long)]
    village: Option<f64>,
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
    /// The kit's railings' bars on one card each, a grating that cannot flicker (costs the
    /// overview about 0.45 ms for now, `research/trim-sheets.md` §2.9).
    #[arg(long)]
    railing_cards: bool,
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
    /// Board the tram nearest the camera this many seconds in (C boards and gets off by hand).
    #[arg(long)]
    ride: Option<f64>,
    /// The plan's people from this game hour (8.2 for the morning's walk to work), those walking
    /// round the camera drawn as figures.
    #[arg(long)]
    people: Option<f64>,
    /// The trams' and boats' service clock at the start, seconds (3600 by default).
    #[arg(long)]
    clock: Option<f64>,
}

fn main() -> anyhow::Result<()> {
    let matches = Cli::command()
        .name("city-plan")
        .about("A city's plan in 3D: the example city's masses, streets, parks and trams")
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let size = cli.village.unwrap_or(cli.size);
    let site = example::site(size, 10.0);
    // The village on its own parameters: lanes, one main road, no trams nor landmarks.
    let params = if cli.village.is_some() {
        city::village::params(size)
    } else {
        example::params(size)
    };
    let (plan, masses) = match cli.village {
        Some(_) => {
            let v = city::village::village(&site, &params, 1);
            eprintln!(
                "the village: {} houses on {} blocks, {} built blocks without",
                v.masses.len(),
                v.blocks.len(),
                v.without
            );
            // The views the map check shoots (tools/map-check.sh), each for `--view`: the first
            // house from its street 7 m out, and a walk along that street; the green nearest
            // the centre; the main road's middle, along it; the busiest junction, from above.
            let half = size * 0.5;
            let view = |p: DVec2, y: f64, look: DVec2, pitch: f64| {
                format!(
                    "--view={:.1},{y},{:.1},{:.1},{pitch}",
                    p.x - half,
                    p.y - half,
                    (-look.x).atan2(-look.y).to_degrees()
                )
            };
            if let Some(h) = v.houses().next() {
                let (a, b) = (h.lot[0], h.lot[1]);
                let t = (b - a).normalize_or_zero();
                let eye = (a + b) * 0.5 + DVec2::new(t.y, -t.x) * 7.0;
                eprintln!(
                    "the village's house: {}; the village's walk: {} --walk={:.2},{:.2}",
                    view(eye, 1.7, DVec2::new(-t.y, t.x), 0.0),
                    view(eye, 1.7, t, 0.0),
                    t.x * 1.4,
                    t.y * 1.4
                );
            }
            let centre = DVec2::splat(half);
            if let Some(green) = v
                .plan
                .blocks
                .iter()
                .filter(|b| b.use_ == forge_procgen::city::BlockUse::Park)
                .min_by(|a, b| {
                    a.centre
                        .distance(centre)
                        .total_cmp(&b.centre.distance(centre))
                })
            {
                let p = green.polygon.first().copied().unwrap_or(green.centre);
                let eye = p + (green.centre - p).normalize_or_zero() * 3.0;
                eprintln!(
                    "the village's green: {}",
                    view(eye, 1.7, green.centre - eye, -5.0)
                );
            }
            if let Some(e) = v
                .plan
                .roads
                .edges
                .iter()
                .filter(|e| e.class == forge_procgen::city::RoadClass::Main && e.points.len() >= 2)
                .max_by(|a, b| a.length().total_cmp(&b.length()))
            {
                let mid = forge_procgen::city::geom::point_at(&e.points, e.length() * 0.5);
                let ahead = forge_procgen::city::geom::point_at(&e.points, e.length() * 0.5 + 10.0);
                eprintln!("the village's main: {}", view(mid, 1.7, ahead - mid, 0.0));
            }
            let mut degree = vec![0usize; v.plan.roads.nodes.len()];
            for e in &v.plan.roads.edges {
                degree[e.a as usize] += 1;
                degree[e.b as usize] += 1;
            }
            if let Some((k, _)) =
                degree
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| **d >= 3)
                    .min_by(|(a, _), (b, _)| {
                        let at = |i: usize| v.plan.roads.nodes[i].pos.distance(centre);
                        at(*a).total_cmp(&at(*b))
                    })
            {
                let p = v.plan.roads.nodes[k].pos;
                let eye = p + DVec2::new(0.0, 18.0);
                eprintln!(
                    "the village's junction: {}",
                    view(eye, 12.0, p - eye, -30.0)
                );
            }
            // The longest garden wall along a street or lane (`city::walls`), from 2.5 m out on
            // the street a little before its start, along it.
            if let Some(w) = v
                .plan
                .walls
                .iter()
                .filter(|w| w.inside.is_some())
                .max_by(|a, b| a.length().total_cmp(&b.length()))
            {
                let t = (w.b - w.a).normalize_or_zero();
                let mut out = DVec2::new(-t.y, t.x);
                if w.inside.is_some_and(|p| out.dot(p - w.a) > 0.0) {
                    out = -out;
                }
                let eye = w.a + out * 2.5 - t * 3.0;
                eprintln!(
                    "the village's garden: {}",
                    view(eye, 1.7, t - out * 0.15, 0.0)
                );
            }
            (v.plan, v.masses)
        }
        None => {
            let plan = city::plan(&site, &params);
            let masses = mass::masses(&plan, &city::bible::StyleBible::default(), 11);
            (plan, masses)
        }
    };
    let trees = city::trees::trees(&site, &plan, 5);
    let lamps = city::lamps::lamps(&site, &plan);
    // Over the river south of the centre, looking north across it; the village from closer.
    let centre = params.centre;
    let (up, back, pitch) = if cli.village.is_some() {
        (110.0, 240.0, -0.42)
    } else {
        (180.0, 700.0, -0.32)
    };
    let camera = FlyCamera {
        position: Vec3::new(
            (centre.x - size * 0.5) as f32,
            up,
            (centre.y - size * 0.5) as f32 + back,
        ),
        yaw: 0.0,
        pitch,
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
    let half = size * 0.5;
    let detail = city_plan::Detail {
        grammar: &grammar,
        top: 0.9,
        centre: if cli.street {
            DVec2::new(
                f64::from(camera.position.x) + half,
                f64::from(camera.position.z) + half,
            )
        } else {
            centre
        },
        // The whole village from the kit.
        radius: if cli.village.is_some() {
            size
        } else {
            cli.detail
        },
        seed: 12,
        railing_cards: cli.railing_cards,
        lit_hour: cli.app.time_of_day.map(city_plan::hour_of_day),
        mix: forge_sim::population::Mix::default(),
    };
    let scenario = city_plan::PlanScenario::detailed(
        &site,
        &plan,
        &masses,
        &trees,
        &lamps,
        camera,
        if cli.village.is_some() {
            "the village"
        } else {
            "city plan"
        },
        (cli.detail > 0.0).then_some(&detail),
    );
    let scenario = match cli.clock {
        Some(clock) => scenario.with_clock(clock),
        None => scenario,
    };
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
    let scenario = match cli.people {
        Some(hour) => scenario.with_people(&plan, &masses, hour),
        None => scenario,
    };
    let scenario = if cli.walk_start {
        scenario.walking()
    } else {
        scenario
    };
    let scenario = match cli.ride {
        Some(after) => scenario.riding(after),
        None => scenario,
    };
    city_blocks::run(cli.app, "forge city-plan", Some(Box::new(scenario)))
}
