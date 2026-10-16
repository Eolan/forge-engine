//! Pitched roofs' geometry (the kit draws them in `city-plan`; the anomaly check counts the tiled
//! masses whose footprint will not take one): which edges slope and which end in gables, and the
//! outline the sloping edges reach at a run.

use glam::DVec2;

use super::CityPlan;
use super::facades::{self, FaceKind};
use super::geom;
use super::mass::Mass;

/// The longest wall a gable stands on, metres: about a building's depth. Longer walls slope.
pub const GABLE_MAX: f64 = 18.0;
/// The longest run of a pitched roof from its eaves, metres (a rise of 4.4 m); a roof that would
/// rise further stops in a terrace.
pub const RUN_MAX: f64 = 8.0;

/// For each mass, whether the neighbour across each of its edges is as tall (within half a
/// metre): a gable there is hidden by the neighbour's roof.
pub fn tall(plan: &CityPlan, masses: &[Mass]) -> Vec<Vec<bool>> {
    facades::neighbour_heights(plan, masses)
        .iter()
        .zip(masses)
        .map(|(h, m)| h.iter().map(|&n| n >= m.height - 0.5).collect())
        .collect()
}

/// Which edges of a footprint slope (1) and which end in a gable (0): a wall onto a neighbour
/// as `tall` (whose roof hides the gable's wall) or within the building, across the main front
/// (its longest street wall, else its longest court wall) and at most [`GABLE_MAX`] long,
/// carries a gable, so the houses of a row share one ridge along the street. The rest slope,
/// a wall above a lower neighbour too (the owner, 2026-10-15: the gable stood as a wedge over
/// its roof); with no gable the roof is hipped. `None` with no front.
pub fn pitches(fp: &[DVec2], edges: &[Option<FaceKind>], tall: &[bool]) -> Option<Vec<f64>> {
    let n = fp.len();
    let edge = |i: usize| fp[(i + 1) % n] - fp[i];
    let open = |i: usize| matches!(edges[i], Some(FaceKind::Street | FaceKind::Court));
    let longest = |kind: FaceKind| {
        (0..n)
            .filter(|&i| edges[i] == Some(kind))
            .max_by(|&a, &b| edge(a).length().total_cmp(&edge(b).length()))
    };
    let front = longest(FaceKind::Street).or_else(|| longest(FaceKind::Court))?;
    let along = edge(front).normalize_or_zero();
    Some(
        (0..n)
            .map(|i| {
                let across = edge(i).normalize_or_zero().dot(along).abs() < 0.5;
                let length = edge(i).length();
                if !open(i) && tall[i] && (0.5..=GABLE_MAX).contains(&length) && across {
                    0.0
                } else {
                    1.0
                }
            })
            .collect(),
    )
}

/// The outline `fp` with each edge `i` moved in by `w[i] * d`: a pitched roof's top at the run
/// `d`. `None` once an edge turns over or two cross (past the ridge).
pub fn ridge(fp: &[DVec2], w: &[f64], d: f64) -> Option<Vec<DVec2>> {
    let n = fp.len();
    let line = |i: usize| {
        let t = (fp[(i + 1) % n] - fp[i]).normalize_or_zero();
        (fp[i] + DVec2::new(-t.y, t.x) * (w[i] * d), t)
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let ((p, t), (q, u)) = (line(prev), line(i));
        let denom = geom::cross(t, u);
        if denom.abs() < 1e-9 {
            // Edges in line meet only where they move alike.
            if w[i] != w[prev] {
                return None;
            }
            out.push(q);
            continue;
        }
        out.push(p + t * (geom::cross(q - p, u) / denom));
    }
    let turned = (0..n).any(|i| (out[(i + 1) % n] - out[i]).dot(line(i).1) <= 1e-6);
    let crossing = (0..n).any(|i| {
        ((i + 2)..n).any(|j| {
            (j + 1) % n != i
                && geom::segment_intersection(out[i], out[(i + 1) % n], out[j], out[(j + 1) % n])
                    .is_some()
        })
    });
    (!turned && !crossing).then_some(out)
}

/// The outline a pitched roof takes over footprint `fp` (its edges `edges`, their neighbours
/// `tall`) and each of its edges' pitch ([`pitches`]): the footprint, or, where one of its edges
/// turns over at once (a short side a lot's corner left), the footprint less the one corner
/// whose loss changes its area least, its two edges joined (the eaves then stand a little off
/// that wall, under their overhang). `None` where neither takes one: the kit draws a flat roof,
/// grey in a row of tiles, its neighbours' gables bare over it.
pub fn fit(
    fp: &[DVec2],
    edges: &[Option<FaceKind>],
    tall: &[bool],
) -> Option<(Vec<DVec2>, Vec<f64>)> {
    let takes = |fp: &[DVec2], edges: &[Option<FaceKind>], tall: &[bool]| {
        let w = pitches(fp, edges, tall)?;
        ridge(fp, &w, 0.5).map(|_| w)
    };
    if let Some(w) = takes(fp, edges, tall) {
        return Some((fp.to_vec(), w));
    }
    let n = fp.len();
    if n <= 4 {
        return None;
    }
    let area = geom::signed_area(fp);
    let mut tries: Vec<(f64, usize)> = (0..n)
        .map(|i| {
            let mut less = fp.to_vec();
            less.remove(i);
            ((geom::signed_area(&less) - area).abs(), i)
        })
        .collect();
    tries.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, i) in tries.into_iter().take(2) {
        // Corner `i` gone: its two edges (`i - 1` and `i`) one, which keeps the longer's kind.
        let prev = (i + n - 1) % n;
        let keep = if fp[i].distance(fp[prev]) >= fp[i].distance(fp[(i + 1) % n]) {
            prev
        } else {
            i
        };
        let mut less = fp.to_vec();
        less.remove(i);
        let mut e = edges.to_vec();
        let mut t = tall.to_vec();
        let (kind, high) = (edges[keep], tall[keep]);
        e[prev] = kind;
        t[prev] = high;
        // The later edges shift down by one, the joined edge standing at `prev` (the last when
        // corner 0 goes).
        e.remove(i);
        t.remove(i);
        if let Some(w) = takes(&less, &e, &t) {
            return Some((less, w));
        }
    }
    None
}

/// Whether footprint `fp` takes a pitched roof (its edges `edges`, their neighbours `tall`), on
/// its own outline or one corner less ([`fit`]): the kit draws a flat roof where it does not.
pub fn takes_pitched(fp: &[DVec2], edges: &[Option<FaceKind>], tall: &[bool]) -> bool {
    fit(fp, edges, tall).is_some()
}
