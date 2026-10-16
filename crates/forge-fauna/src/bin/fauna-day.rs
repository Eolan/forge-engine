//! `cargo run --release -p forge-fauna --bin fauna-day -- --out reports/2026-10-16`: the example
//! village's animals over a day in June with its passers, their paths drawn on the plan
//! (`fauna-day.png`); prints the step's time, the distances, the flights from threats, and the
//! step's time for more and more animals (an hour from 18:00, the busiest).

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use forge_fauna::map::{self, DayMap};
use forge_fauna::sim::{DAY, HZ, ticks};
use forge_fauna::{Fauna, Mode, Parts, Passers, Plan, Roster, Threat, village};
use glam::Vec3;

const FONT: &str = "assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf";

fn main() {
    let mut out = PathBuf::from("captures/fauna");
    let mut seed = 1u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = PathBuf::from(args.next().expect("--out takes a folder")),
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed N"),
            other => panic!("unknown argument {other}; --out DIR, --seed N"),
        }
    }
    std::fs::create_dir_all(&out).expect("creating the output folder");
    let font = std::fs::read(FONT).ok();
    let t = Instant::now();
    let plan = Plan::new(Parts::example(seed));
    let plan_ms = t.elapsed().as_secs_f64() * 1e3;
    let species = village();
    let passers = Passers::example(&plan, 60, 20, seed);
    let mut f = Fauna::new(&plan, &species, &Roster::village(&species), seed, 0.0);
    println!(
        "plan {plan_ms:.0} ms: {} buildings, {} trees, {} browse shrubs, {} sites, {} patches; {} passers",
        plan.parts.buildings.len(),
        plan.parts.trees.len(),
        plan.parts.shrubs.len(),
        plan.sites.len(),
        plan.patches.len(),
        passers.list.len()
    );
    let mut m = DayMap::new(&plan, 3.0, 380, 200);
    let n = f.len();
    let mut last: Vec<Option<Vec3>> = vec![None; n];
    let mut moving = vec![[0.0f32; 24]; species.len()];
    let mut samples = vec![[0u32; 24]; species.len()];
    let mut seen = vec![0u32; n];
    let mut buf = Vec::new();
    let mut stepping = 0.0f64;
    let t0 = Instant::now();
    for k in 0..DAY {
        buf.clear();
        passers.at(f.seconds(), &plan, &mut buf);
        let ts = Instant::now();
        f.step(&buf);
        stepping += ts.elapsed().as_secs_f64();
        if k % u64::from(2 * HZ) == 0 {
            let hour = (f.seconds() / 3600.0) as usize % 24;
            let mut now = vec![(0u32, 0u32); species.len()];
            for i in 0..n {
                let s = f.groups[f.group[i] as usize].species as usize;
                now[s].1 += 1;
                if f.vel[i].length() > 0.1 {
                    now[s].0 += 1;
                }
            }
            for (s, (mv, all)) in now.into_iter().enumerate() {
                moving[s][hour] += mv as f32 / all.max(1) as f32;
                samples[s][hour] += 1;
            }
            for i in 0..n {
                let b = f.species_of(i).behaviour;
                if f.mode[i] == Mode::Hidden {
                    last[i] = None;
                    continue;
                }
                seen[i] += 1;
                // One swift a party: fifty of them along every street make a blanket.
                if b == forge_fauna::Behaviour::Swift && f.member[i] != 0 {
                    continue;
                }
                let p = f.pos[i];
                if let Some(q) = last[i]
                    && (p - q).length() < 40.0
                    && (p - q).length() > 0.01
                {
                    m.path(q, p, map::colour(b), map::alpha(b, p.z.max(q.z)));
                }
                last[i] = Some(p);
            }
        }
    }
    let wall = t0.elapsed().as_secs_f64();
    for (row, count) in moving.iter_mut().zip(&samples) {
        for (v, &k) in row.iter_mut().zip(count) {
            *v /= k.max(1) as f32;
        }
    }
    println!(
        "a day of {n} animals: {:.2} s stepping ({:.1} µs a step, {:.0} ns an animal), {:.1} s with the drawing; {} contacts",
        stepping,
        stepping / DAY as f64 * 1e6,
        stepping / DAY as f64 / n as f64 * 1e9,
        wall,
        f.contacts
    );
    println!("contacts by species: {:?}", f.contacts_by);
    let mut rows = Vec::new();
    println!(
        "{:<22} {:>5} {:>9} {:>9} {:>9} {:>8}",
        "species", "count", "km mean", "km max", "h seen", "flushes"
    );
    for (s, sp) in species.iter().enumerate() {
        let who: Vec<usize> = (0..n)
            .filter(|&i| f.groups[f.group[i] as usize].species as usize == s)
            .collect();
        if who.is_empty() {
            continue;
        }
        let km: Vec<f32> = who.iter().map(|&i| f.travelled[i] / 1000.0).collect();
        let mean = km.iter().sum::<f32>() / km.len() as f32;
        let max = km.iter().copied().fold(0.0, f32::max);
        let hours = who.iter().map(|&i| seen[i] as f32).sum::<f32>() / who.len() as f32 / 1800.0;
        let flushes = f.flushes.iter().filter(|x| x.species as usize == s).count();
        println!(
            "{:<22} {:>5} {:>9.2} {:>9.2} {:>9.1} {:>8}",
            sp.name,
            who.len(),
            mean,
            max,
            hours,
            flushes
        );
        rows.push((
            sp.behaviour,
            format!("{} ({}): {:.1} km", sp.name, who.len(), mean),
        ));
    }
    let by = |t: fn(&Threat) -> bool| f.flushes.iter().filter(|x| t(&x.threat)).count();
    println!(
        "flushes by threat: people {}, vehicles {}, dogs {}, cats {}",
        by(|t| matches!(t, Threat::Person)),
        by(|t| matches!(t, Threat::Vehicle { .. })),
        by(|t| matches!(t, Threat::Dog)),
        by(|t| matches!(t, Threat::Cat)),
    );
    let notes = vec![
        format!("seed {seed}, a June day, 400 m square"),
        format!("{} people and vehicles going by", passers.list.len()),
        "a mean km a day each beside the name".to_string(),
        "flight above 15 m faint; one swift a party,".to_string(),
        "below 20 m (their screaming parties)".to_string(),
    ];
    m.legend(
        font.as_deref(),
        "The village's animals, a day",
        &rows,
        &notes,
    );
    m.hours(font.as_deref(), &species, &moving);
    m.canvas.posterize(12.0);
    let path = out.join("fauna-day.png");
    m.canvas.save(&path).expect("writing the map");
    println!("wrote {}", path.display());

    // The step's time for more animals: an hour from 18:00, groups times 1, 4, 16.
    println!(
        "{:>7} {:>9} {:>11} {:>13}",
        "groups", "animals", "µs a step", "ns an animal"
    );
    for scale in [1u32, 4, 16] {
        let mut roster = Roster::village(&species);
        for g in &mut roster.groups {
            g.1 *= scale;
        }
        let mut f = Fauna::new(&plan, &species, &roster, seed, 18.0 * 3600.0);
        let steps = ticks(3600.0);
        let mut total = 0.0f64;
        for _ in 0..steps {
            buf.clear();
            passers.at(f.seconds(), &plan, &mut buf);
            let ts = Instant::now();
            f.step(&buf);
            total += ts.elapsed().as_secs_f64();
        }
        println!(
            "{:>7} {:>9} {:>11.1} {:>13.0}",
            f.groups.len(),
            f.len(),
            total / steps as f64 * 1e6,
            total / steps as f64 / f.len() as f64 * 1e9
        );
    }
}
