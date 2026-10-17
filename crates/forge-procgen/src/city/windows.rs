//! Windows by room (the owner, 2026-10-16: "the living spaces like the bedrooms very likely have
//! at least a window. Living rooms have windows and / or bay windows, kitchen, if separated from
//! the living room are very likely to have at least one windows. Bathroom have a probability to
//! have a window. toilets a bit less"; "windows should match a corridor, a room, but not have a
//! wall right of their edge"; `research/house-layouts.md` rules 13–17). A house with its rooms'
//! plan gets its windows from its rooms, not from its facade's bays: each room's stretch of outside
//! wall, centred in it and clear of its partitions, so no partition meets a window.

use forge_core::SplitMix64;
use glam::DVec2;

use super::interior;
use super::mass::Mass;
use super::rooms::RoomKind;
use super::structure::{Opening, OpeningKind, PIER, Structure, WallRole};

/// A bathroom's window's chance in a house (`research/house-layouts.md` §3.2: 0.6), a service
/// room's (0.4).
const BATHROOM: f64 = 0.6;
const SERVICE: f64 = 0.4;
/// A stretch of outside wall long enough for two windows in a living room, metres.
const TWO: f64 = 3.6;
/// A small high window's width and height, metres (a bathroom's, a service room's).
const SMALL: (f64, f64) = (0.6, 0.7);

/// The windows of house `m` (a mass with its rooms' plan) from its rooms, in place of its facade's
/// bays: each living room, salon, dining room, kitchen, bedroom and study a window on each of its
/// stretches of outside wall (a living room, salon or dining room two on a stretch of [`TWO`] or
/// more), centred in the stretch and [`PIER`] off its ends; a bathroom or a service room a small
/// high one on its longest stretch, by chance; a store none; the hall none (the stair rises there).
/// The doors, shopfronts and passages stay. A house whose insides cannot be laid keeps its bays.
pub fn by_room(m: &Mass, s: &mut Structure, seed: u64) {
    if m.rooms.is_none() {
        return;
    }
    // The rooms where the plan lays them, before any window moves a partition.
    let kept: Vec<Opening> = s
        .openings
        .iter()
        .copied()
        .filter(|o| o.kind != OpeningKind::Window)
        .collect();
    let mut bare = s.clone();
    bare.openings = kept.clone();
    let Some(inner) = interior::house(m, &bare) else {
        return;
    };
    let t = s.era.table();
    // The facade's bays' axes on each wall (where its windows stood, every storey): a room's
    // window takes the axis nearest where it would centre, within the room, so the windows stand
    // over each other storey to storey (rule 6) wherever the rooms allow.
    let axes: Vec<Vec<f64>> = (0..s.walls.len())
        .map(|w| {
            let mut a: Vec<f64> = s
                .openings
                .iter()
                .filter(|o| o.wall == w && o.kind == OpeningKind::Window)
                .map(|o| o.along)
                .collect();
            a.sort_by(f64::total_cmp);
            a.dedup_by(|x, y| (*x - *y).abs() < 0.05);
            a
        })
        .collect();
    let mut out = kept;
    let mut rng = SplitMix64::new(seed ^ 0x0057_1D05);
    // The rooms, and the hall on each storey where no stair rises in it (a house of one storey):
    // its kind `None`.
    let mut spaces: Vec<(Option<RoomKind>, usize, Vec<DVec2>)> = inner
        .rooms
        .iter()
        .map(|r| (Some(r.kind), r.storey, r.outline.to_vec()))
        .collect();
    // The hall on every storey without a stair, and on the upper storeys of a house with one: a
    // landing along a side street showed it a blank wall the whole depth.
    let first = usize::from(inner.stair.is_some());
    spaces.extend((first..s.storeys.len()).map(|k| (None, k, inner.hall.to_vec())));
    // The flights' feet, where no window stands within 1.5 m (the owner, 2026-10-16: "window too
    // close to the stair bottom, should not happen").
    let feet: Vec<DVec2> = inner
        .stair
        .map(|st| (st[0] + st[1]) * 0.5)
        .into_iter()
        .collect();
    for (kind, k, outline) in &spaces {
        let (kind, k) = (*kind, *k);
        let Some(storey) = s.storeys.get(k) else {
            continue;
        };
        let draw = rng.next_f64();
        if kind == Some(RoomKind::Store) {
            continue;
        }
        let small = matches!(kind, Some(RoomKind::Bathroom | RoomKind::Service));
        let chance = match kind {
            Some(RoomKind::Bathroom) => BATHROOM,
            Some(RoomKind::Service) => SERVICE,
            _ => 1.0,
        };
        let many = matches!(
            kind,
            Some(RoomKind::Living | RoomKind::Salon | RoomKind::Dining)
        );
        // The room's stretches of outside wall it may open: along each street or court wall it
        // stands against, where that wall faces open air on its storey.
        let mut stretches: Vec<(usize, f64, f64)> = Vec::new();
        for (w, wall) in s.walls.iter().enumerate() {
            if !matches!(wall.role, WallRole::Street | WallRole::Court) {
                continue;
            }
            let tw = (wall.b - wall.a).normalize_or_zero();
            let inward = DVec2::new(-tw.y, tw.x);
            let along: Vec<f64> = outline
                .iter()
                .filter(|p| ((**p - wall.a).dot(inward) - wall.thickness).abs() < 0.25)
                .map(|p| (*p - wall.a).dot(tw))
                .collect();
            if along.len() < 2 {
                continue;
            }
            let lo = along.iter().copied().fold(f64::MAX, f64::min).max(0.0);
            let hi = along
                .iter()
                .copied()
                .fold(f64::MIN, f64::max)
                .min(wall.length());
            for &(olo, ohi) in wall.open.get(k).map_or(&[][..], |v| &v[..]) {
                let (a, b) = (lo.max(olo), hi.min(ohi));
                if b - a > 0.6 + 2.0 * PIER {
                    stretches.push((w, a, b));
                }
            }
        }
        // A space on the street shows it a window whatever it is (the owner, 2026-10-16: "an
        // architect would arrange the inside of structure so that windows are on the street side
        // instead of showing a blank wall"): a bathroom's small and high; a hall's on the street
        // only.
        let on_street = |&(w, _, _): &(usize, f64, f64)| s.walls[w].role == WallRole::Street;
        if kind.is_none() {
            stretches.retain(on_street);
        }
        if !stretches.iter().any(on_street) && draw >= chance {
            continue;
        }
        if small || !many {
            // A small room on its street stretch, else its longest; another on its longest and its
            // street stretch.
            stretches.sort_by(|x, y| (y.2 - y.1).total_cmp(&(x.2 - x.1)));
            let longest = stretches.first().copied();
            let street = stretches.iter().copied().find(on_street);
            stretches = if small {
                street.or(longest).into_iter().collect()
            } else {
                longest
                    .into_iter()
                    .chain(street.filter(|&st| Some(st) != longest))
                    .collect()
            };
        }
        for (w, a, b) in stretches {
            let length = b - a;
            let (width, sill, head) = if small {
                let sill = (storey.clear() - 0.5 - SMALL.1).max(1.4);
                (SMALL.0, sill, sill + SMALL.1)
            } else if k == 0 {
                (t.window, 1.0, storey.clear() - 0.4)
            } else {
                (t.window, t.sill, storey.clear() - 0.35)
            };
            let width = width.min(length - 2.0 * PIER);
            let width = (width * 10.0).floor() / 10.0;
            if width < 0.5 {
                continue;
            }
            let targets: Vec<f64> = if many && length >= TWO {
                vec![a + length * 0.25, a + length * 0.75]
            } else {
                vec![(a + b) * 0.5]
            };
            // Each on the bay axis nearest it that keeps the window a pier inside the stretch,
            // two windows never on one axis.
            let (lo_c, hi_c) = (a + width * 0.5 + PIER, b - width * 0.5 - PIER);
            let mut used: Vec<f64> = Vec::new();
            let centres: Vec<f64> = targets
                .iter()
                .map(|&target| {
                    let snap = axes[w]
                        .iter()
                        .copied()
                        .filter(|&x| x >= lo_c && x <= hi_c)
                        .filter(|&x| used.iter().all(|&u| (u - x).abs() > width + PIER))
                        .min_by(|x, y| (x - target).abs().total_cmp(&(y - target).abs()));
                    let c = snap.unwrap_or(target);
                    used.push(c);
                    c
                })
                .collect();
            for along in centres {
                // Clear of the doors and the shopfronts on that wall and storey, and of a
                // flight's foot.
                let (lo, hi) = (along - width * 0.5 - PIER, along + width * 0.5 + PIER);
                let wall = &s.walls[w];
                let at = wall.a + (wall.b - wall.a).normalize_or_zero() * along;
                let blocked = out
                    .iter()
                    .any(|o| o.wall == w && o.storey == k && o.span().0 < hi && o.span().1 > lo)
                    || feet.iter().any(|f| f.distance(at) < 1.5 + width * 0.5);
                if blocked {
                    continue;
                }
                out.push(Opening {
                    wall: w,
                    storey: k,
                    along,
                    width,
                    sill,
                    head,
                    kind: OpeningKind::Window,
                    // A bathroom's glass obscured, its privacy kept.
                    frosted: kind == Some(RoomKind::Bathroom),
                });
            }
        }
    }
    s.openings = out;
}
