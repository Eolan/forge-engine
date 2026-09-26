//! `genesis --seed 7 --spacing 16 --steps 150 --out captures/island`: the terrain genesis
//! pipeline of `forge-procgen` over a 16 km island, stage by stage with timings, and the PNG
//! previews of each stage in `--out` (`docs/research/terrain-genesis.md`, "What to build
//! first"), the water's fields (the coast distance, the sea's spectrum as a tile), and the
//! field's digest, the same on every machine (D-016). Default: 16 m samples (1025²), seconds; `--spacing 4` is the island's target
//! (4097², sixteen times the work), under a minute on the job system.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use forge_core::Seed;
use forge_procgen::field::Field2;
use forge_procgen::flow::priority_flood;
use forge_procgen::{ErosionParams, IslandParams, erosion, island_fields, preview};
use forge_task::{PoolConfig, TaskPool};

#[derive(Parser, Debug)]
#[command(about = "Terrain genesis: an island from a seed, with PNG previews")]
struct Args {
    /// The seed.
    #[arg(long, default_value_t = 7)]
    seed: u64,
    /// Metres between samples (16: a 1025² island; 4: the 4097² target).
    #[arg(long, default_value_t = 16.0)]
    spacing: f64,
    /// Erosion steps.
    #[arg(long, default_value_t = 150)]
    steps: u32,
    /// Erodibility per step.
    #[arg(long, default_value_t = ErosionParams::island().k)]
    k: f64,
    /// Hillslope diffusion, m² per step.
    #[arg(long, default_value_t = ErosionParams::island().diffusion)]
    diffusion: f64,
    /// Uplift at the island's heart, metres per step.
    #[arg(long)]
    uplift: Option<f64>,
    /// Write a preview every N steps (`height-NNNN.png`), besides the final ones.
    #[arg(long)]
    every: Option<u32>,
    /// Where the previews go.
    #[arg(long, default_value = "captures/island")]
    out: PathBuf,
    /// Worker threads besides the main one (default: one per hardware thread; 0 runs
    /// everything on the main thread, the result is the same).
    #[arg(long)]
    threads: Option<usize>,
    /// The prevailing wind's origin (n, ne, e, se, s, sw, w, nw; north is the top of the
    /// previews): orographic rain, refreshed from the relief every ten steps. Without it the
    /// rain is flat.
    #[arg(long)]
    wind_from: Option<String>,
    /// How far the orographic rain departs from flat (0 flat, 1 the model).
    #[arg(long, default_value_t = 1.0)]
    rain_contrast: f64,
}

/// The wind that blows from `from` (a compass point, north up in the previews).
fn wind_from(from: &str, contrast: f64) -> Result<forge_procgen::Wind> {
    // WIND_STEPS: 0 = +x (east), 1 = +x+y (south-east), 2 = +y (south), … clockwise.
    let towards = match from.to_ascii_lowercase().as_str() {
        "w" => 0,
        "nw" => 1,
        "n" => 2,
        "ne" => 3,
        "e" => 4,
        "se" => 5,
        "s" => 6,
        "sw" => 7,
        other => anyhow::bail!("unknown wind origin {other}: n, ne, e, se, s, sw, w or nw"),
    };
    Ok(forge_procgen::Wind { towards, contrast })
}

fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out).context("creating the output directory")?;
    let mut params = IslandParams::island_16km(Seed::new(args.seed), args.spacing);
    if let Some(uplift) = args.uplift {
        params.uplift = uplift;
    }
    if let Some(from) = &args.wind_from {
        params.wind = Some(wind_from(from, args.rain_contrast)?);
    }
    let erosion_params = ErosionParams {
        k: args.k,
        diffusion: args.diffusion,
        steps: args.steps,
        ..ErosionParams::island()
    };
    let pool = match args.threads {
        Some(workers) => TaskPool::new(PoolConfig::with_workers(workers)),
        None => TaskPool::server(),
    };
    println!(
        "island: seed {}, {}² samples at {} m ({:.1} km), {} steps, {} worker threads",
        args.seed,
        params.size,
        params.spacing,
        params.extent() / 1000.0,
        args.steps,
        pool.worker_count()
    );

    let start = Instant::now();
    let fields = island_fields(&params);
    let land = fields.shape.data.iter().filter(|&&s| s > 0.0).count();
    println!(
        "stages 1–2, mask and uplift: {:.2} s; land {:.1} % of the domain, uplift up to {:.2} m/step",
        start.elapsed().as_secs_f64(),
        100.0 * land as f64 / fields.shape.len() as f64,
        fields.uplift.min_max().1
    );
    preview::write_height(&fields.uplift, &args.out.join("uplift.png"))?;

    let start = Instant::now();
    let mut height: Field2<f32> = fields.shape.map(|_| 0.0);
    let mut run = erosion::Erosion::new();
    let mut timings = erosion::StepTimings::default();
    let mut rain = fields.rain.clone();
    let mut rain_seconds = 0.0;
    for s in 0..args.steps {
        let started = Instant::now();
        if forge_procgen::refresh_rain(&params, &height, erosion_params.sea_level, s, &mut rain) {
            rain_seconds += started.elapsed().as_secs_f64();
        }
        timings += erosion::step_timed(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &rain,
            &erosion_params,
            &pool,
            &mut run,
        );
        if let Some(every) = args.every
            && (s + 1) % every == 0
        {
            preview::write_hillshade(
                &height,
                &args.out.join(format!("hillshade-{:04}.png", s + 1)),
            )?;
        }
        if (s + 1) % 25 == 0 || s + 1 == args.steps {
            let (_, hi) = height.min_max();
            println!(
                "  step {:>4}: {:.1} s so far, highest {:.0} m",
                s + 1,
                start.elapsed().as_secs_f64(),
                hi
            );
        }
    }
    let flow = if args.steps == 0 {
        erosion::erode(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion_params,
            &pool,
        )
    } else {
        run.take_flow()
    };
    let per_step = start.elapsed().as_secs_f64() / f64::from(args.steps.max(1));
    println!(
        "stage 3, erosion: {:.1} s, {:.3} s a step (uplift {:.3}, drain {:.3}, incise {:.3}, diffuse {:.3})",
        start.elapsed().as_secs_f64(),
        per_step,
        timings.uplift.as_secs_f64() / f64::from(args.steps.max(1)),
        timings.drain.as_secs_f64() / f64::from(args.steps.max(1)),
        timings.incise.as_secs_f64() / f64::from(args.steps.max(1)),
        timings.diffuse.as_secs_f64() / f64::from(args.steps.max(1)),
    );
    if let Some(wind) = params.wind {
        // Windward and lee halves of the land, split across the wind through the centre.
        let (dx, dy) = forge_procgen::island::WIND_STEPS[usize::from(wind.towards) % 8];
        let half = f64::from(params.size - 1) * 0.5;
        let (mut wind_sum, mut wind_count, mut lee_sum, mut lee_count) = (0.0, 0, 0.0, 0);
        for i in 0..height.len() {
            if height.data[i] <= erosion_params.sea_level {
                continue;
            }
            let (x, y) = height.coords(i);
            let along =
                (f64::from(x) - half) * f64::from(dx) + (f64::from(y) - half) * f64::from(dy);
            if along < 0.0 {
                wind_sum += f64::from(rain.data[i]);
                wind_count += 1;
            } else {
                lee_sum += f64::from(rain.data[i]);
                lee_count += 1;
            }
        }
        println!(
            "  orographic rain from the {} ({rain_seconds:.2} s in all): windward half {:.2}, lee half {:.2}, {:.2}–{:.2}",
            args.wind_from
                .as_deref()
                .unwrap_or("?")
                .to_ascii_uppercase(),
            wind_sum / f64::from(wind_count.max(1)),
            lee_sum / f64::from(lee_count.max(1)),
            rain.min_max().0,
            rain.min_max().1
        );
        preview::write_height(&rain, &args.out.join("rain.png"))?;
    }

    // Stage 4, the first part: lakes where the flood raised the eroded field, rivers by area.
    let start = Instant::now();
    let filled = priority_flood(&height, erosion_params.sea_level);
    let lake_depth = Field2 {
        size: height.size,
        spacing: height.spacing,
        data: filled
            .data
            .iter()
            .zip(&height.data)
            .map(|(f, h)| f - h)
            .collect(),
    };
    let river_cells = (500_000.0 / (params.spacing * params.spacing)) as u32; // 0.5 km² of catchment
    let rivers = flow.area.iter().filter(|&&a| a > river_cells).count();
    let lakes = lake_depth.data.iter().filter(|&&d| d > 0.5).count();
    let network = forge_procgen::trace_rivers(&height, &flow, river_cells + 1);
    let trunks = network
        .rivers
        .iter()
        .filter(|r| matches!(r.mouth, forge_procgen::Mouth::Outlet(_)))
        .count();
    let longest = network
        .rivers
        .iter()
        .map(|r| r.length())
        .fold(0.0_f32, f32::max);
    let widest = network
        .rivers
        .iter()
        .flat_map(|r| r.area.iter())
        .map(|&a| forge_procgen::hydrology::width(f64::from(a) * params.spacing * params.spacing))
        .fold(0.0, f64::max);
    let ponds = forge_procgen::trace_lakes(&height, &filled, &flow, 0.5);
    println!(
        "stage 4, hydrology: {:.2} s; {rivers} river samples above 0.5 km² of catchment, {lakes} lake samples; {} rivers ({trunks} to the sea, {} km in all, the longest {:.1} km, order up to {}, up to {widest:.0} m wide); {} lakes, the largest {:.1} ha, the deepest {:.1} m",
        start.elapsed().as_secs_f64(),
        network.rivers.len(),
        (network.total_length() / 1000.0).round(),
        longest / 1000.0,
        network.max_order(),
        ponds.lakes.len(),
        ponds.largest_area(params.spacing) / 10_000.0,
        ponds.deepest()
    );

    // Stage 5 for the water: the coast distance, and the sea's spectrum as a tile.
    let start = Instant::now();
    let coast = forge_procgen::coast_distance(&height, erosion_params.sea_level, &pool);
    let coast_seconds = start.elapsed().as_secs_f64();
    let (_, inland) = coast.min_max();
    let ocean = forge_procgen::Ocean::new(forge_procgen::OceanParams::breeze(Seed::new(args.seed)));
    let (sea, numbers) = forge_procgen::ocean::report(&ocean, 0.0);
    println!(
        "stage 5, the water's fields: coast distance {coast_seconds:.2} s, {:.1} km inland at most; the sea's {}² tile ({} m, {} m/s wind) {:.3} s, significant height {:.2} m, {:.2}–{:.2} m, displacement up to {:.2} m, {:.1} % folded",
        inland / 1000.0,
        ocean.params.size,
        ocean.params.patch,
        ocean.params.wind_speed,
        numbers.seconds,
        numbers.significant_height,
        numbers.lowest,
        numbers.highest,
        numbers.displacement,
        100.0 * numbers.folded
    );

    let start = Instant::now();
    preview::write_height(&coast, &args.out.join("coast.png"))?;
    preview::write_height(&sea.height, &args.out.join("sea-height.png"))?;
    preview::write_hillshade(&sea.height, &args.out.join("sea-hillshade.png"))?;
    preview::write_height(&height, &args.out.join("height.png"))?;
    preview::write_hillshade(&height, &args.out.join("hillshade.png"))?;
    preview::write_flow(&flow, height.size, &args.out.join("flow.png"))?;
    preview::write_overview(
        &height,
        &flow,
        erosion_params.sea_level,
        river_cells,
        Some(&lake_depth),
        &args.out.join("overview.png"),
    )?;
    let (lo, hi) = height.min_max();
    println!(
        "previews in {} ({:.2} s): height {:.0}–{:.0} m, digest {:016x}",
        args.out.display(),
        start.elapsed().as_secs_f64(),
        lo,
        hi,
        height.digest()
    );
    Ok(())
}
