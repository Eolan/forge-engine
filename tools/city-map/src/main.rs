//! `city-map --size 4000 --out captures/city`: the example city's plan from
//! `forge-procgen::city` (an invented coast, river and districts), drawn as a PNG map with its
//! numbers and each stage's time, the way a plan is judged before any building exists
//! (`docs/research/city-generation.md`, stage 5; D-059).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use forge_procgen::city::{self, example};
use glam::DVec2;

#[derive(Parser, Debug)]
#[command(about = "The example city's plan as a PNG map, with its numbers")]
struct Args {
    /// The frame's side, metres.
    #[arg(long, default_value_t = 4000.0)]
    size: f64,
    /// The site masks' cell, metres.
    #[arg(long, default_value_t = 10.0)]
    cell: f64,
    /// Metres per pixel of the map.
    #[arg(long, default_value_t = 2.0)]
    scale: f64,
    /// Where to write `plan.png`.
    #[arg(long, default_value = "captures/city")]
    out: PathBuf,
    /// The faces map's middle, `x,y` metres on the plan (the plan's centre otherwise).
    #[arg(long, value_parser = parse_point)]
    faces_at: Option<DVec2>,
}

fn parse_point(s: &str) -> Result<DVec2, String> {
    let (x, y) = s.split_once(',').ok_or("expected x,y")?;
    let f = |v: &str| v.trim().parse::<f64>().map_err(|e| e.to_string());
    Ok(DVec2::new(f(x)?, f(y)?))
}

fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out).context("creating the output folder")?;
    let start = Instant::now();
    let site = example::site(args.size, args.cell);
    let params = example::params(args.size);
    println!("site: {:.0} ms", start.elapsed().as_secs_f64() * 1e3);
    let plan = city::plan(&site, &params);
    for (stage, ms) in &plan.timings {
        println!("{stage:>24}: {ms:8.1} ms");
    }
    println!("{:#?}", plan.stats);
    let path = args.out.join("plan.png");
    let t = Instant::now();
    let font = std::fs::read("assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf").ok();
    let labels = city::map::labels(&plan);
    city::map::write_labelled(&site, &plan, &path, args.scale, &labels, font.as_deref())
        .context("writing the map")?;
    println!(
        "map: {} in {:.0} ms",
        path.display(),
        t.elapsed().as_secs_f64() * 1e3
    );
    let checks = args.out.join("checks.png");
    city::map::write_checks(&site, &plan, &params, &checks, args.scale)
        .context("writing the checks")?;
    let t = Instant::now();
    let bible = city::bible::StyleBible::default();
    let masses = city::mass::masses(&plan, &bible, 11);
    let structures = city::structure::structures(&plan, &masses);
    let gross: f64 = structures.iter().map(|s| s.gross).sum();
    let net: f64 = structures.iter().map(|s| s.net).sum();
    println!(
        "masses: {} and their structure in {:.0} ms, {:.1} km² of floors gross, {:.1} net ({:.2})",
        masses.len(),
        t.elapsed().as_secs_f64() * 1e3,
        gross / 1e6,
        net / 1e6,
        net / gross
    );
    for r in city::bible::check(&plan, &masses, &bible) {
        println!(
            "{} {}: {:.3} (limit {:.3}){}",
            if r.holds { "ok  " } else { "FAIL" },
            r.rule,
            r.value,
            r.limit,
            if r.failing > 0 {
                format!("; {} failing, e.g. {}", r.failing, r.failures.join(", "))
            } else {
                String::new()
            }
        );
    }
    let whole = args.out.join("massing.png");
    let frame = (DVec2::ZERO, args.size);
    city::map::write_massing(&site, &plan, &masses, &whole, frame, args.scale, 0.7)
        .context("writing the massing")?;
    let near = args.out.join("massing-centre.png");
    let window = (params.centre - DVec2::splat(500.0), 1000.0);
    city::map::write_massing(&site, &plan, &masses, &near, window, 0.5, 0.7)
        .context("writing the close massing")?;
    let faces = city::facades::faces(&plan, &masses);
    city::map::write_faces(
        &site,
        &plan,
        &faces,
        &args.out.join("faces-centre.png"),
        (
            args.faces_at.unwrap_or(params.centre) - DVec2::splat(200.0),
            400.0,
        ),
        0.2,
    )
    .context("writing the faces")?;
    println!("checks, massing: {}", args.out.display());
    Ok(())
}
