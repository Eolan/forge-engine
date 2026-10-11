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
    city::map::write(&site, &plan, &path, args.scale).context("writing the map")?;
    println!(
        "map: {} in {:.0} ms",
        path.display(),
        t.elapsed().as_secs_f64() * 1e3
    );
    Ok(())
}
