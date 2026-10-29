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
    /// Four pursuers on the street before the first house with insides go for its hall, by its
    /// door or over a window's sill (#317); the log names a view of its front.
    #[arg(long)]
    pursuers: bool,
    /// With `--village`: the village's test mission (#287): written from the first house with a
    /// stair, played from the street before its front door, walking: up to a bedroom, the parcel
    /// that then lies in the kitchen, back out to the street. The title shows the objective.
    #[arg(long)]
    mission: bool,
    /// With `--mission`: the walker walks it on its own, on the house's walk field (D-076).
    #[arg(long)]
    mission_autopilot: bool,
    /// With `--village`: no animals. The village's animals (#269: sparrows, pigeons, swifts and
    /// gulls, cats, and dogs walked by their owners, at the run's hour of a June day, fleeing the
    /// player) are on by default since 2026-10-24.
    #[arg(long)]
    no_fauna: bool,
    /// Kept so older command lines still run: the animals are on by default.
    #[arg(long, hide = true)]
    fauna: bool,
    /// One second in, hide every other clear pane and raise the rest by 0.4 m: the check of
    /// panes changed at run time (#325).
    #[arg(long)]
    glass_test: bool,
    /// One second in, hide the village's forest, its trees' stand-ins and shadows with them: the
    /// check of static instances changed at run time (#323).
    #[arg(long)]
    static_test: bool,
    /// One second in, the tree 12 m before the camera promoted to a mover and demoted a second
    /// later (#338): `hold` where it stands, `swing` leant 15° on the way, `snap` leant at once
    /// without a mover (the swing's reference), `none` the pool alone (the others' reference).
    #[arg(long, value_parser = ["none", "hold", "swing", "snap"])]
    promote_test: Option<String>,
    /// The houses' furniture fades away under this many pixels across on screen (#343, a ladder
    /// to nothing; 24: about 35 m for a chair at 1600×900). Off by default.
    #[arg(long)]
    fade_furniture: Option<f32>,
    /// The houses' furniture met by the probes' rays, as before #363 (to compare): out of them by
    /// default, its visibility test cut the room grids' light into curved patches.
    #[arg(long)]
    furniture_in_probes: bool,
    /// Board the tram nearest the camera this many seconds in (C boards and gets off by hand).
    #[arg(long)]
    ride: Option<f64>,
    /// The plan's people from this game hour (8.2 for the morning's walk to work), those walking
    /// round the camera drawn as figures.
    #[arg(long)]
    people: Option<f64>,
    /// With `--village`: print the village's anomalies (`city::anomalies`), each with a view to
    /// look at it from, and stop.
    #[arg(long)]
    anomalies: bool,
    /// The trams' and boats' service clock at the start, seconds (3600 by default).
    #[arg(long)]
    clock: Option<f64>,
}

/// What the kit's style puts over the roof line, metres (`Detail::top`).
const KIT_TOP: f64 = 0.9;
/// The kit's seed for its buildings' choices (`Detail::seed`).
const KIT_SEED: u64 = 12;

/// What the scene is made from: the site, its plan and masses, its trees and lamps, the camera
/// and the kit's centre.
struct Made {
    site: city::Site,
    plan: city::CityPlan,
    masses: Vec<mass::Mass>,
    trees: Vec<city::trees::Tree>,
    lamps: Vec<city::lamps::Lamp>,
    camera: FlyCamera,
    detail_centre: DVec2,
}

/// The plan and its views, printed for the map check (`tools/map-check.sh` reads them from the
/// log); `None` once `--anomalies` has printed the village's, which runs no window.
fn made(cli: &Cli) -> anyhow::Result<Option<Made>> {
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
                // 2.5 m off the centre line, where a shared street's trees stand (one stood
                // 15 cm from the camera).
                let along = (ahead - mid).normalize_or_zero();
                let eye = mid + DVec2::new(-along.y, along.x) * 2.5;
                eprintln!("the village's main: {}", view(eye, 1.7, ahead - mid, 0.0));
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
            // Inside the first house with its rooms and a stair (`city::interior`): a metre behind
            // its street door, in the hall, looking into the house.
            let structures = city::structure::structures(&v.plan, &v.masses);
            // The anomalies, each from its point of view: 4 m up and looking down on what stands
            // outside, at eye height on what stands inside a house.
            if cli.anomalies {
                // Dressed as the kit dresses it, its windows fitted to their modules, then its
                // openings checked against them (D-066's holes and fills).
                let text = match &cli.rules {
                    Some(path) => std::fs::read_to_string(path)?,
                    None => city::facades::MEDITERRANEAN.to_owned(),
                };
                let grammar = forge_procgen::grammar::Grammar::parse(&text)?;
                let every: Vec<usize> = (0..v.masses.len()).collect();
                let dress = |s: &[city::structure::Structure]| {
                    city::facades::dress(&grammar, s, &v.masses, &every, KIT_TOP, KIT_SEED).0
                };
                let mut structures = structures.clone();
                let fills = city::facades::fills(&grammar, &structures, &dress(&structures));
                city::facades::fit_windows(&mut structures, &fills);
                let mut all = city::anomalies::village(&v, &structures);
                let fills = city::facades::fills(&grammar, &structures, &dress(&structures));
                all.extend(city::anomalies::openings(&structures, &fills));
                for a in &all {
                    let inside = matches!(
                        a.kind,
                        city::anomalies::Kind::PartitionInOpening
                            | city::anomalies::Kind::UnlitRoom
                            | city::anomalies::Kind::BedroomThroughRoom
                    );
                    let (y, pitch) = if inside {
                        (a.height.max(1.6), 0.0)
                    } else {
                        (a.height + 4.0, -25.0)
                    };
                    let look = if a.from.distance(a.at) < 0.1 {
                        DVec2::X
                    } else {
                        a.at - a.from
                    };
                    println!("{:?}: {}: {}", a.kind, a.note, view(a.from, y, look, pitch));
                }
                for kind in city::anomalies::Kind::ALL {
                    println!("{kind:?}: {}", city::anomalies::count(&all, kind));
                }
                return Ok(None);
            }
            if let Some((s, inner)) = v.masses.iter().zip(&structures).find_map(|(m, s)| {
                city::interior::house(m, s)
                    .filter(|inner| inner.stair.is_some())
                    .map(|inner| (s, inner))
            }) && let Some(door) = s
                .cores
                .iter()
                .find(|c| c.house)
                .and_then(|c| s.openings.get(c.door))
            {
                // Its front door, the core's: the first door on a street-facing wall was, in
                // some houses, the garden door onto a lane at the back.
                let w = &s.walls[door.wall];
                let t = (w.b - w.a).normalize_or_zero();
                let inward = DVec2::new(-t.y, t.x);
                let eye = w.a + t * door.along + inward * (w.thickness + 1.0);
                // A metre inside it, looking down the hall, the flight climbing towards the
                // camera from its foot at the back.
                eprintln!("the village's hall: {}", view(eye, 1.6, inward, -5.0));
                // From the back of the hall (6 m in at most), looking back at the open front
                // door: the sunlit street through the doorway beside the dim hall, local
                // exposure's case (#275, D-067).
                let at = w.a + t * door.along;
                let depth = inner
                    .hall
                    .iter()
                    .map(|&c| (c - at).dot(inward))
                    .fold(0.0, f64::max);
                let back = at + inward * (depth - 0.8).clamp(w.thickness + 1.0, 6.0);
                eprintln!("the village's door: {}", view(back, 1.6, -inward, -3.0));
                // On the first upper floor's landing, a step past the flight's arrival, looking
                // back over the opening and its guard (no view looked upstairs, and the opening
                // had none, 2026-10-16).
                if let (Some(stair), Some(&r), Some(st)) =
                    (inner.stair, inner.risers.first(), s.storeys.get(1))
                {
                    let run = f64::from(r.saturating_sub(1)) * city::cores::GOING;
                    let arrive = (stair[0] + stair[1]) * 0.5 + inner.climb * (run + 0.8);
                    eprintln!(
                        "the village's landing: {}",
                        view(arrive, st.level + 1.6, -inner.climb, -25.0)
                    );
                }
                // Into the first room off the hall on the ground floor: from the hall, half a
                // metre back from its doorway, looking at the room's middle.
                if let Some((p, room)) = inner
                    .partitions
                    .iter()
                    .filter(|p| p.storey == 0 && !p.doors.is_empty())
                    .find_map(|p| {
                        let t = (p.b - p.a).normalize_or_zero();
                        let (d0, d1) = p.doors[0];
                        let mid = p.a + t * ((d0 + d1) * 0.5);
                        inner
                            .rooms
                            .iter()
                            .filter(|r| r.storey == 0)
                            .find(|r| {
                                let c = city::geom::centroid(&r.outline);
                                (c - mid).length() < 6.0
                                    && city::geom::contains(&inner.hall, mid * 2.0 - c)
                            })
                            .map(|r| (mid, r))
                    })
                {
                    let c = city::geom::centroid(&room.outline);
                    let eye = p - (c - p).normalize_or_zero() * 0.5;
                    eprintln!("the village's room: {}", view(eye, 1.6, c - eye, -12.0));
                }
                // The way in: 3 m out on the street before the door, walking through it.
                let out = w.a + t * door.along - inward * 3.0;
                eprintln!(
                    "the village's way in: {} --walk={:.2},{:.2}",
                    view(out, 1.7, inward, -5.0),
                    inward.x * 1.4,
                    inward.y * 1.4
                );
            }
            // The lowest bathroom's window on a street, its glass obscured, from 6 m out at eye
            // height, looking up at it.
            if let Some((s, o)) = structures
                .iter()
                .flat_map(|s| {
                    s.openings
                        .iter()
                        .filter(|o| {
                            o.frosted && s.walls[o.wall].role == city::structure::WallRole::Street
                        })
                        .map(move |o| (s, o))
                })
                .min_by_key(|(_, o)| o.storey)
            {
                let w = &s.walls[o.wall];
                let t = (w.b - w.a).normalize_or_zero();
                let out = DVec2::new(t.y, -t.x);
                let at = w.a + t * o.along;
                let level = s.storeys.get(o.storey).map_or(0.0, |st| st.level);
                let rise = (level + o.sill + 0.35 - 1.7).max(0.0);
                let pitch = (rise.atan2(6.0).to_degrees() * 10.0).round() / 10.0;
                eprintln!(
                    "the village's frosted window: {}",
                    view(at + out * 6.0, 1.7, -out, pitch)
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
            // The longest low garden wall on a street (up to 1.1 m), from 2 m out on the street
            // at its middle, walked into: the walker vaults it (#316).
            if let Some(w) = v
                .plan
                .walls
                .iter()
                .filter(|w| w.inside.is_some() && w.height <= 1.1)
                .max_by(|a, b| a.length().total_cmp(&b.length()))
            {
                let t = (w.b - w.a).normalize_or_zero();
                let mut out = DVec2::new(-t.y, t.x);
                if w.inside.is_some_and(|p| out.dot(p - w.a) > 0.0) {
                    out = -out;
                }
                let eye = (w.a + w.b) * 0.5 + out * 2.0;
                eprintln!(
                    "the village's vault: {} --walk={:.2},{:.2}",
                    view(eye, 1.7, -out, -10.0),
                    -out.x * 1.4,
                    -out.y * 1.4
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
    // The city's camera crosses its 4 km at 40 m/s; the village's flies at a stroll's 4 (Shift 20,
    // Ctrl 100, Alt a quarter), to look the houses over without walking (the owner, 2026-10-16:
    // "make the camera slower for when I want to test without walking").
    let camera = FlyCamera {
        position: Vec3::new(
            (centre.x - size * 0.5) as f32,
            up,
            (centre.y - size * 0.5) as f32 + back,
        ),
        yaw: 0.0,
        pitch,
        speed: if cli.village.is_some() { 4.0 } else { 40.0 },
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
    let half = size * 0.5;
    let detail_centre = if cli.street {
        DVec2::new(
            f64::from(camera.position.x) + half,
            f64::from(camera.position.z) + half,
        )
    } else {
        centre
    };
    Ok(Some(Made {
        site,
        plan,
        masses,
        trees,
        lamps,
        camera,
        detail_centre,
    }))
}

fn main() -> anyhow::Result<()> {
    let matches = Cli::command()
        .name("city-plan")
        .about("A city's plan in 3D: the example city's masses, streets, parks and trams")
        .get_matches();
    let cli = Cli::from_arg_matches(&matches)?;
    let size = cli.village.unwrap_or(cli.size);
    // The village's anomalies printed, without a window.
    if cli.anomalies && cli.village.is_some() {
        made(&cli)?;
        return Ok(());
    }
    // The kit round the street view, or round the centre seen from the air.
    let text = match &cli.rules {
        Some(path) => std::fs::read_to_string(path)?,
        None => city::facades::MEDITERRANEAN.to_owned(),
    };
    let grammar = forge_procgen::grammar::Grammar::parse(&text)?;
    let half = size * 0.5;
    let lit_hour = cli.app.time_of_day.map(city_plan::hour_of_day);
    let grass = cli.app.grass.is_some();
    if cli.tram.as_deref().is_some_and(|t| t.len() != 3) {
        anyhow::bail!("--tram takes x,z,lead");
    }
    let title = if cli.village.is_some() {
        "the village"
    } else {
        "city plan"
    };
    // The scene made on the loading thread, the plan with it, beside the window's and the
    // device's creation (#274: the city's plan kept its window from opening for 0.66 s).
    let app = cli.app.clone();
    city_blocks::run_making(app, "forge city-plan", move || {
        let Made {
            site,
            plan,
            masses,
            trees,
            lamps,
            camera,
            detail_centre,
        } = match made(&cli) {
            Ok(Some(made)) => made,
            Ok(None) => return None,
            Err(error) => panic!("the plan: {error:#}"),
        };
        let detail = city_plan::Detail {
            grammar: &grammar,
            top: KIT_TOP,
            centre: detail_centre,
            // The whole village from the kit.
            radius: if cli.village.is_some() {
                size
            } else {
                cli.detail
            },
            seed: KIT_SEED,
            railing_cards: cli.railing_cards,
            grass,
            lit_hour,
            mix: forge_sim::population::Mix::default(),
        };
        let scenario = city_plan::PlanScenario::detailed(
            &site,
            &plan,
            &masses,
            &trees,
            &lamps,
            camera,
            title,
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
            _ => scenario,
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
        let scenario = if cli.glass_test {
            scenario.glass_test()
        } else {
            scenario
        };
        let scenario = if cli.static_test {
            scenario.static_test()
        } else {
            scenario
        };
        let scenario = match cli.fade_furniture {
            Some(px) => scenario.fade_furniture(px),
            None => scenario,
        };
        let scenario = if cli.furniture_in_probes {
            scenario.furniture_in_probes()
        } else {
            scenario
        };
        let scenario = match cli.promote_test.as_deref() {
            Some(mode) => scenario.promote_test(match mode {
                "hold" => city_plan::PromoteTest::Hold,
                "swing" => city_plan::PromoteTest::Swing,
                "snap" => city_plan::PromoteTest::Snap,
                _ => city_plan::PromoteTest::None,
            }),
            None => scenario,
        };
        let scenario = if cli.pursuers {
            scenario.with_pursuers(&plan, &masses)
        } else {
            scenario
        };
        if cli.mission && cli.village.is_none() {
            eprintln!("--mission plays on the village: add --village 600");
        }
        let scenario = if cli.mission && cli.village.is_some() {
            scenario.with_mission(&plan, &masses, cli.mission_autopilot)
        } else {
            scenario
        };
        let scenario = if (cli.fauna || !cli.no_fauna) && cli.village.is_some() {
            scenario.with_fauna(&site, &plan, &masses, lit_hour.unwrap_or(15.0))
        } else {
            scenario
        };
        let scenario = match cli.ride {
            Some(after) => scenario.riding(after),
            None => scenario,
        };
        let scenario: Box<dyn city_blocks::scenario::Scenario> = Box::new(scenario);
        Some(scenario)
    })
}
