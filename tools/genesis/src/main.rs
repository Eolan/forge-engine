//! `genesis --seed 7 --spacing 16 --steps 150 --out captures/island`: the terrain genesis
//! pipeline of `forge-procgen` over a 16 km island, stage by stage with timings, and the PNG
//! previews of each stage in `--out` (`docs/research/terrain-genesis.md`, "What to build
//! first"). Default: 16 m samples (1025²), seconds; `--spacing 4` is the island's target
//! (4097², sixteen times the work), a minute or two on the job system.

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
}

fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.out).context("creating the output directory")?;
    let mut params = IslandParams::island_16km(Seed::new(args.seed), args.spacing);
    if let Some(uplift) = args.uplift {
        params.uplift = uplift;
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
    let mut flow = None;
    let mut timings = erosion::StepTimings::default();
    for s in 0..args.steps {
        let (f, t) = erosion::step_timed(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &erosion_params,
            &pool,
        );
        flow = Some(f);
        timings += t;
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
    let flow = flow.unwrap_or_else(|| {
        erosion::erode(
            &mut height,
            &fields.uplift,
            &fields.hardness,
            &fields.rain,
            &ErosionParams {
                steps: 0,
                ..erosion_params
            },
            &pool,
        )
    });
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
    println!(
        "stage 4, hydrology: {:.2} s; {rivers} river samples above 0.5 km² of catchment, {lakes} lake samples",
        start.elapsed().as_secs_f64()
    );

    let start = Instant::now();
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
        "previews in {} ({:.2} s): height {:.0}–{:.0} m",
        args.out.display(),
        start.elapsed().as_secs_f64(),
        lo,
        hi
    );
    Ok(())
}
