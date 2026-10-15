//! `population-bench --days 2`: the example city's plan (`forge_procgen::city::example`), its
//! masses as buildings, their homes, jobs, schools, shops, cafés and bars as places, the
//! population that fits (`forge_sim::population`, #250), run through game days headless. Prints
//! the census every game hour, the events and their cost, the bytes a person and the digest, and
//! what a game hour of 20 real minutes asks of the server (D-059).

#![forbid(unsafe_code)]

use std::time::Instant;

use anyhow::Result;
use clap::Parser;
use forge_core::Seed;
use forge_procgen::city::{self, BlockUse, example, geom};
use forge_sim::population::{
    self, Building, DAY, GAME_SPEED, Mix, Population, Sim, Uses, World, places_from_buildings,
};

#[derive(Parser, Debug)]
#[command(about = "A city's population through game days, headless")]
struct Args {
    /// The example's side, metres.
    #[arg(long, default_value_t = 4000.0)]
    size: f64,
    /// Game days to run.
    #[arg(long, default_value_t = 2)]
    days: u32,
    /// Every building's floors this many times over (a scale test: 13.5 gives about 4 M people
    /// in the example's 4 km); no maps then.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let site = example::site(args.size, 10.0);
    let params = example::params(args.size);
    let plan = city::plan(&site, &params);
    let masses = city::mass::masses(&plan, &city::bible::StyleBible::default(), 11);
    // Homes and jobs from each building's net floor area (D-060).
    let nets: Vec<f64> = city::structure::structures(&plan, &masses)
        .iter()
        .map(|s| s.net)
        .collect();
    let buildings: Vec<Building> = masses
        .iter()
        .zip(&nets)
        .map(|(m, &net)| {
            let c = geom::centroid(&m.footprint);
            Building {
                pos: [c.x as f32, c.y as f32],
                floor_area: (net * args.scale) as f32,
                active_ground: m.lively(),
                appeal: if m.open { 1.0 } else { 0.0 },
                // Scaled floor area, so by floor area: the bench's 2–4 M.
                homes: None,
                jobs: None,
            }
        })
        .collect();
    let parks: Vec<[f32; 2]> = plan
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Park)
        .map(|b| [b.centre.x as f32, b.centre.y as f32])
        .collect();
    let shots = Shots {
        site: &site,
        plan: &plan,
        out: std::path::PathBuf::from("captures/population"),
    };
    let report = run(
        &buildings,
        &parks,
        args.size as f32,
        args.days,
        (args.scale == 1.0).then_some(&shots),
    );
    println!("{report}");
    Ok(())
}

/// Where to draw the population at 08:00, 13:00 and 21:00 of the first day.
struct Shots<'a> {
    site: &'a city::Site,
    plan: &'a city::CityPlan,
    out: std::path::PathBuf,
}

/// A person's colour on the maps by what they are doing.
fn colour(doing: population::Doing) -> [u8; 3] {
    use population::Doing;
    match doing {
        Doing::Home => [70, 90, 160],
        Doing::Anchor => [255, 170, 60],
        Doing::Shop => [120, 220, 255],
        Doing::Out => [255, 80, 200],
        Doing::Park => [90, 230, 110],
        Doing::Travelling => [255, 255, 220],
    }
}

/// Makes the population of `buildings` and `parks` and runs it for `days`, returning the report;
/// draws it at three hours of the first day when `shots` says where.
fn run(
    buildings: &[Building],
    parks: &[[f32; 2]],
    size: f32,
    days: u32,
    shots: Option<&Shots>,
) -> String {
    let mut out = String::new();
    let t = Instant::now();
    let places = places_from_buildings(buildings, parks, &Uses::default());
    let world = World::new(places, size, 250.0);
    let made_places = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let pop: Population = population::generate(&world, &Mix::default(), Seed::new(2026));
    let generated = t.elapsed().as_secs_f64();
    let n = pop.len();
    // Home to work, straight, for the workers (docs/research/trip-distances.md: a median of
    // 3–4 km in a dense city, about 20 % under 2 km).
    let mut commutes: Vec<f32> = (0..n)
        .filter(|&p| pop.anchor[p] != population::NONE && pop.role[p] == population::Role::Worker)
        .map(|p| {
            let (a, b) = (
                world.places[pop.home[p] as usize].pos,
                world.places[pop.anchor[p] as usize].pos,
            );
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
        })
        .collect();
    commutes.sort_by(f32::total_cmp);
    if !commutes.is_empty() {
        let under = |km: f32| {
            100.0 * commutes.partition_point(|&d| d < km * 1000.0) as f32 / commutes.len() as f32
        };
        out += &format!(
            "home to work, straight: median {:.1} km, {:.0} % under 2 km, {:.0} % under 5 km\n",
            commutes[commutes.len() / 2] / 1000.0,
            under(2.0),
            under(5.0)
        );
    }
    out += &format!(
        "{} places, {n} people, {} bytes a person ({:.1} MB); places {:.0} ms, people {:.0} ms\n",
        world.places.len(),
        Population::bytes_per_person(),
        (n * Population::bytes_per_person()) as f64 / 1e6,
        made_places * 1e3,
        generated * 1e3
    );
    let t = Instant::now();
    let mut sim = Sim::new(&world, pop);
    let start = t.elapsed().as_secs_f64();
    out += &format!(
        "calendar {:.0} ms\nhour   home  at work/school  shops  cafés  bars  parks  travelling\n",
        start * 1e3
    );
    let t = Instant::now();
    for hour in 1..=days * 24 {
        sim.run_until(hour * 3600);
        if let Some(shots) = shots
            && [8, 13, 21].contains(&hour)
        {
            let _ = std::fs::create_dir_all(&shots.out);
            let points: Vec<_> = sim
                .positions()
                .into_iter()
                .map(|(p, d)| (p, colour(d)))
                .collect();
            let font = std::fs::read("assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf").ok();
            let caption = format!("{hour:02}:00, {n} people");
            let path = shots.out.join(format!("people-{hour:02}.png"));
            let _ = city::map::write_points(
                shots.site,
                shots.plan,
                &points,
                &path,
                4.0,
                font.as_deref().map(|f| (caption.as_str(), f)),
            );
        }
        if hour % 3 == 0 || hour <= 24 {
            let c = sim.census();
            let pc = |v: u32| 100.0 * v as f64 / n as f64;
            out += &format!(
                "{:>4}  {:>5.1}  {:>13.1}  {:>5.1}  {:>5.1}  {:>4.1}  {:>5.1}  {:>10.1}\n",
                hour,
                pc(c.home),
                pc(c.anchor),
                pc(c.shop),
                pc(c.food),
                pc(c.leisure),
                pc(c.park),
                pc(c.travelling)
            );
        }
    }
    let ran = t.elapsed().as_secs_f64();
    let per_day = sim.events as f64 / f64::from(days);
    out += &format!(
        "{} events over {days} days ({:.1} a person a day) in {:.2} s: {:.0} ns an event; a game day at 1 h = 20 min (8 real hours) asks {:.0} events a real second, {:.4} % of one core\n",
        sim.events,
        per_day / n as f64,
        ran,
        ran * 1e9 / sim.events as f64,
        per_day / f64::from(DAY / GAME_SPEED),
        100.0 * (ran / f64::from(days)) / f64::from(DAY / GAME_SPEED),
    );
    out += &format!("digest {:016x}", sim.digest());
    // A snapshot of the running state, restored over the base generated again (#250).
    let t = Instant::now();
    let bytes = sim.snapshot();
    let saved = t.elapsed().as_secs_f64();
    let base = population::generate(&world, &Mix::default(), Seed::new(2026));
    let t = Instant::now();
    let restored = Sim::restore(&world, base, &bytes).map(|r| r.digest());
    let loaded = t.elapsed().as_secs_f64();
    out += &format!(
        "\nsnapshot: {:.1} MB ({} bytes a person) in {:.0} ms, restored in {:.0} ms, {}",
        bytes.len() as f64 / 1e6,
        population::SNAPSHOT_BYTES,
        saved * 1e3,
        loaded * 1e3,
        match restored {
            Ok(d) if d == sim.digest() => "the same digest".to_owned(),
            Ok(_) => "DIFFERENT".to_owned(),
            Err(e) => format!("not restored: {e}"),
        }
    );
    // The same days in shards on worker threads (#250): the same digest, faster.
    for workers in [6] {
        let pop = population::generate(&world, &Mix::default(), Seed::new(2026));
        let mut shards = population::Shards::new(&world, pop, workers);
        let t = Instant::now();
        for hour in 1..=days * 24 {
            shards.run_until(hour * 3600);
        }
        let ran_w = t.elapsed().as_secs_f64();
        out += &format!(
            "\n{workers} workers: {:.2} s ({:.1} times one), digest {:016x} ({})",
            ran_w,
            ran / ran_w,
            shards.digest(),
            if shards.digest() == sim.digest() {
                "the same"
            } else {
                "DIFFERENT"
            }
        );
    }
    out
}
