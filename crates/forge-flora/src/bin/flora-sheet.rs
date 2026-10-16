//! `cargo run --release -p forge-flora --bin flora-sheet -- --out reports/2026-10-16`: grows the
//! village's species and draws them side by side, from the side and from above
//! (`flora-species.png`), and the grower's inputs, age, wind, site and pruning
//! (`flora-inputs.png`); prints each species' numbers and its growth time (the best of three).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use forge_flora::draw::{self, Canvas, INK, PAPER, View};
use forge_flora::map::{self, HillsideMap};
use forge_flora::place::{self, Fields, Ground};
use forge_flora::{Conditions, Pruning, Species, TreeGraph, grow, mediterranean};
use glam::{DVec2, Vec2};

/// The narrowest cell, pixels: room for its labels.
const MIN_CELL: f32 = 250.0;

const FONT: &str = "assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf";

fn main() {
    let mut out = PathBuf::from("captures/flora");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(args.next().expect("--out takes a folder")),
            other => panic!("unknown argument {other}; --out DIR"),
        }
    }
    std::fs::create_dir_all(&out).expect("creating the output folder");
    let font = std::fs::read(FONT).ok();
    let all = mediterranean();
    println!(
        "{:<16} {:>5} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6} {:>8}",
        "species", "age", "nodes", "branch", "leaves", "height", "width", "base r", "grow ms"
    );
    let mut grown = Vec::new();
    for sp in &all {
        let c = Conditions::new(sp.maturity, 1);
        let mut best = f64::MAX;
        let mut g = None;
        for _ in 0..3 {
            let t = Instant::now();
            let tree = grow(sp, &c);
            best = best.min(t.elapsed().as_secs_f64() * 1e3);
            g = Some(tree);
        }
        let g = g.expect("grown");
        println!(
            "{:<16} {:>5.0} {:>6} {:>6} {:>6} {:>7.2} {:>7.2} {:>6.3} {:>8.1}",
            sp.id,
            sp.maturity,
            g.nodes.len(),
            g.branch_count(),
            g.leaves.len(),
            g.height(),
            g.crown_width(),
            g.base_radius(),
            best
        );
        grown.push(g);
    }
    let path = out.join("flora-species.png");
    species_sheet(&all, &grown, font.as_deref())
        .save(&path)
        .expect("writing the species sheet");
    println!("{}", path.display());
    let path = out.join("flora-inputs.png");
    inputs_sheet(&all, font.as_deref())
        .save(&path)
        .expect("writing the inputs sheet");
    println!("{}", path.display());
    hillside(&all, &out, font.as_deref());
    per_square_kilometre(&all);
}

/// One plant's cell on a sheet.
struct Cell<'a> {
    sp: &'a Species,
    g: TreeGraph,
    caption: String,
}

fn label(sp: &Species, g: &TreeGraph) -> [String; 3] {
    [
        sp.name.clone(),
        sp.latin.clone(),
        format!("{:.1} m tall, {:.1} m across", g.height(), g.crown_width()),
    ]
}

/// Draws a row of plants from the side over a ground line, then the same from above below it,
/// at `scale` pixels a metre; returns the row's width.
#[allow(clippy::too_many_arguments)]
fn row(
    canvas: &mut Canvas,
    font: Option<&[u8]>,
    cells: &[Cell],
    left: f32,
    top: f32,
    scale: f32,
    tallest: f32,
    widest: f32,
    tick: f32,
    with_top: bool,
) -> f32 {
    let ground = top + tallest * scale + 10.0;
    draw::ruler(canvas, font, left + 30.0, ground, scale, tallest, tick);
    let mut x = left + 50.0;
    for cell in cells {
        let (lo, hi) = cell.g.extent();
        let w = (hi.x - lo.x).max(hi.z - lo.z).max(widest * 0.25);
        let width = (w * scale + 30.0).max(MIN_CELL);
        let cx = x + width * 0.5;
        let colours = draw::colours(&cell.sp.id, cell.sp.foliage);
        canvas.line(
            Vec2::new(x, ground),
            Vec2::new(x + width, ground),
            [150, 140, 120],
            1.0,
        );
        let base = Vec2::new(cx - (lo.x + hi.x) * 0.5 * scale, ground);
        draw::plant(canvas, &cell.g, View::Side, base, scale, colours);
        let [name, latin, size] = label(cell.sp, &cell.g);
        let mut y = ground + 8.0;
        for (s, px) in [
            (name, 15.0),
            (latin, 12.0),
            (size, 12.0),
            (cell.caption.clone(), 12.0),
        ] {
            if !s.is_empty() {
                canvas.text(font, Vec2::new(x + 4.0, y), px, &s, INK);
                y += px + 4.0;
            }
        }
        if with_top {
            let top_y = ground + 80.0 + widest * 0.5 * scale;
            let centre = Vec2::new(
                cx - (lo.x + hi.x) * 0.5 * scale,
                top_y - (lo.z + hi.z) * 0.5 * scale,
            );
            draw::plant(canvas, &cell.g, View::Top, centre, scale, colours);
        }
        x += width;
    }
    x - left
}

fn species_sheet(all: &[Species], grown: &[TreeGraph], font: Option<&[u8]>) -> Canvas {
    let cells = |shrubs: bool| -> Vec<Cell> {
        all.iter()
            .zip(grown)
            .filter(|(s, _)| (s.habit == forge_flora::species::Habit::Shrub) == shrubs)
            .map(|(s, g)| Cell {
                sp: s,
                g: g.clone(),
                caption: format!(
                    "published {:.1}-{:.1} m, {:.1}-{:.1} m",
                    s.height[0], s.height[1], s.crown_width[0], s.crown_width[1]
                ),
            })
            .collect()
    };
    let trees = cells(false);
    let shrubs = cells(true);
    let size = |cs: &[Cell]| {
        cs.iter().fold((0.0f32, 0.0f32), |(h, w), c| {
            let (lo, hi) = c.g.extent();
            (h.max(c.g.height()), w.max((hi.x - lo.x).max(hi.z - lo.z)))
        })
    };
    let (th, tw) = size(&trees);
    let (sh, sw) = size(&shrubs);
    let (ts, ss) = (17.0, 110.0);
    let width = 2000;
    let height = (90.0 + (th * ts + 10.0 + 80.0 + tw * ts) + 60.0) as u32;
    let mut canvas = Canvas::new(width, height.max(900), PAPER);
    canvas.text(
        font,
        Vec2::new(30.0, 20.0),
        22.0,
        "The village's species, grown (forge-flora, #269): from the side and from above",
        INK,
    );
    canvas.text(
        font,
        Vec2::new(30.0, 50.0),
        13.0,
        "Each at the age it reaches 95 % of its height, seed 1. Trees at 17 px a metre, shrubs at 110 px a metre.",
        INK,
    );
    let used = row(&mut canvas, font, &trees, 10.0, 90.0, ts, th, tw, 5.0, true);
    row(
        &mut canvas,
        font,
        &shrubs,
        used + 40.0,
        90.0,
        ss,
        sh.max(2.0),
        sw,
        0.5,
        true,
    );
    canvas
}

fn inputs_sheet(all: &[Species], font: Option<&[u8]>) -> Canvas {
    let by = |id: &str| forge_flora::species::by_id(all, id).expect("a village species");
    let pine = by("aleppo-pine");
    let oak = by("holm-oak");
    let olive = by("olive");
    let mut cells = Vec::new();
    for age in [10.0, 25.0, 50.0, 80.0] {
        cells.push(Cell {
            sp: pine,
            g: grow(pine, &Conditions::new(age, 4)),
            caption: format!("age {age:.0}"),
        });
    }
    let still = Conditions::new(oak.maturity, 4);
    cells.push(Cell {
        sp: oak,
        g: grow(oak, &still),
        caption: "still air".into(),
    });
    let mut windy = still;
    windy.wind = DVec2::new(0.8, 0.0);
    cells.push(Cell {
        sp: oak,
        g: grow(oak, &windy),
        caption: "wind 0.8 from the left".into(),
    });
    let mut poor = still;
    poor.site = 0.25;
    cells.push(Cell {
        sp: oak,
        g: grow(oak, &poor),
        caption: "poor site (0.25)".into(),
    });
    let wild = Conditions::new(olive.maturity, 4);
    cells.push(Cell {
        sp: olive,
        g: grow(olive, &wild),
        caption: "unpruned".into(),
    });
    let mut vase = wild;
    vase.pruning = Pruning {
        lift: 1.0,
        top: 4.5,
        open_centre: 0.8,
    };
    cells.push(Cell {
        sp: olive,
        g: grow(olive, &vase),
        caption: "pruned: vase, 1 m trunk, 4.5 m top".into(),
    });
    let tallest = cells.iter().map(|c| c.g.height()).fold(0.0, f32::max);
    let widest = cells
        .iter()
        .map(|c| {
            let (lo, hi) = c.g.extent();
            (hi.x - lo.x).max(hi.z - lo.z)
        })
        .fold(0.0, f32::max);
    let scale = 12.0;
    let total: f32 = cells
        .iter()
        .map(|c| {
            let (lo, hi) = c.g.extent();
            ((hi.x - lo.x).max(hi.z - lo.z).max(widest * 0.25) * scale + 30.0).max(MIN_CELL)
        })
        .sum();
    let width = (total + 80.0) as u32;
    let height = (90.0 + tallest * scale + 10.0 + 90.0) as u32;
    let mut canvas = Canvas::new(width, height, PAPER);
    canvas.text(
        font,
        Vec2::new(30.0, 20.0),
        22.0,
        "The grower's inputs: age, wind, site and pruning",
        INK,
    );
    canvas.text(
        font,
        Vec2::new(30.0, 50.0),
        13.0,
        "Aleppo pine at 10, 25, 50 and 80 years; holm oak in still air, in a wind, on a poor site; olive unpruned and pruned. 12 px a metre.",
        INK,
    );
    row(
        &mut canvas,
        font,
        &cells,
        10.0,
        90.0,
        scale,
        tallest,
        widest,
        5.0,
        false,
    );
    canvas
}

/// The example hillside, placed and drawn (`flora-hillside.png`), with its numbers.
fn hillside(all: &[Species], out: &std::path::Path, font: Option<&[u8]>) {
    let size = 600.0;
    let ground = Ground::example(size, 2.0, 7);
    let fields = Fields::of(&ground);
    let example = place::example_rules(all, size);
    let placed = forge_flora::place(&ground, &fields, all, &example.rules, 3, &|_| false);
    println!("hillside: {size:.0} m, {} plants", placed.plants.len());
    for (k, sp) in all.iter().enumerate() {
        let n = placed
            .plants
            .iter()
            .filter(|p| usize::from(p.species) == k)
            .count();
        println!("  {:<16} {n:>6}", sp.id);
    }
    // A few grown individuals per species for the section.
    let grown: Vec<Vec<TreeGraph>> = all
        .iter()
        .map(|sp| {
            (0..3)
                .map(|k| grow(sp, &Conditions::new(sp.maturity, 100 + k)))
                .collect()
        })
        .collect();
    let x = size * 0.3;
    let v = place::example_valley(x, size);
    let labels = [
        ("north ridge: Aleppo pines", 6.0),
        (
            "the sunny slope, facing south: garrigue and pines",
            v - 150.0,
        ),
        ("the valley: holm oaks by the stream", v + 8.0),
        ("the shaded slope, facing north: holm oaks", v + 120.0),
        ("south ridge", size - 24.0),
    ]
    .map(|(t, y)| (t.to_string(), DVec2::new(14.0, y)));
    let section_x = size * 0.62;
    let canvas = map::hillside(&HillsideMap {
        ground: &ground,
        fields: &fields,
        species: all,
        placement: &placed,
        water: std::slice::from_ref(&example.stream),
        tracks: std::slice::from_ref(&example.track),
        grown: &grown,
        section_x,
        labels: &labels,
        font,
    });
    let path = out.join("flora-hillside.png");
    canvas.save(&path).expect("writing the hillside");
    println!("{}", path.display());
}

/// Placement's time over a square kilometre of the example ground at 2 m (the best of three).
fn per_square_kilometre(all: &[Species]) {
    let size = 1000.0;
    let mut best = [f64::MAX; 3];
    let mut count = 0;
    for _ in 0..3 {
        let t = Instant::now();
        let ground = Ground::example(size, 2.0, 7);
        best[0] = best[0].min(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        let fields = Fields::of(&ground);
        best[1] = best[1].min(t.elapsed().as_secs_f64() * 1e3);
        let example = place::example_rules(all, size);
        let t = Instant::now();
        let placed = forge_flora::place(&ground, &fields, all, &example.rules, 3, &|_| false);
        best[2] = best[2].min(t.elapsed().as_secs_f64() * 1e3);
        count = placed.plants.len();
    }
    println!(
        "a square kilometre at 2 m: ground {:.0} ms, fields {:.0} ms, placement {:.0} ms, {count} plants",
        best[0], best[1], best[2]
    );
}
