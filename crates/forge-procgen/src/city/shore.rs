//! The shore as a smooth line: the site's water mask is cells (10 m in the examples), and drawn
//! as cells its edge is a staircase along every quay and beach (the owner's plan, 2026-10-11).
//! Here the sea and the lagoons are blurred over two cells and the rivers measured from their
//! centrelines, into one wetness per sample; the shore is its half-way contour, bilinear between
//! the samples, cut square by square (marching squares) into dry and wet pieces and the shore's
//! segments between them.

use glam::DVec2;

use super::{LAGOON, SEA, Site, geom};
use crate::field::Field2;

/// The tent filter's weights over five samples: the sea's blur.
const TENT: [f64; 5] = [1.0, 2.0, 3.0, 2.0, 1.0];

/// A site's wetness, 0 (dry) to 1 (water), on the water mask's samples: the sea's and the
/// lagoons' cells blurred, and each river's bed out to `margin` beyond its banks.
#[derive(Clone, Debug)]
pub struct Shore {
    /// The open water's (sea and lagoons) blurred share at each sample.
    sea: Field2<f32>,
    /// The rivers' wetness at each sample: one half on the bed's edge, rising a third per cell
    /// inward (a square's corners lie within 1.5 cells of a shore crossing it: linear there, so
    /// exact between the samples).
    river: Field2<f32>,
}

/// One square of the grid between four samples, cut along the shore.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Square {
    /// The dry pieces, counter-clockwise (positive [`geom::signed_area`]).
    pub dry: Vec<Vec<DVec2>>,
    /// The wet pieces, counter-clockwise.
    pub wet: Vec<Vec<DVec2>>,
    /// The shore's segments in the square, each with the unit direction towards the water.
    pub shore: Vec<([DVec2; 2], DVec2)>,
}

impl Shore {
    /// The site's shore, its rivers' beds widened by `margin` metres on each side (0 for their
    /// banks; more where a promenade covers the bed's edge).
    pub fn new(site: &Site, margin: f64) -> Self {
        let water = &site.water;
        let (n, s) = (water.size, water.spacing);
        let open = |x: i64, y: i64| {
            let c = |v: i64| v.clamp(0, i64::from(n) - 1) as u32;
            let code = water.get(c(x), c(y));
            f64::from(u8::from(code == SEA || code == LAGOON))
        };
        // Separable: along the rows, then down the columns.
        let rows = Field2::from_fn(n, s, |x, y| {
            (0..5)
                .map(|k| TENT[k] * open(i64::from(x) + k as i64 - 2, i64::from(y)))
                .sum::<f64>()
                / 9.0
        });
        let sea = Field2::from_fn(n, s, |x, y| {
            let at = |y: i64| rows.get(x, y.clamp(0, i64::from(n) - 1) as u32);
            ((0..5)
                .map(|k| TENT[k] * at(i64::from(y) + k as i64 - 2))
                .sum::<f64>()
                / 9.0) as f32
        });
        let mut river = Field2::<f32>::new(n, s);
        for r in &site.rivers {
            let reach = r.width * 0.5 + margin;
            let line = geom::resample(&r.centreline, 15.0);
            for w in line.windows(2) {
                let (lo, hi) = (
                    w[0].min(w[1]) - (reach + 2.0 * s),
                    w[0].max(w[1]) + (reach + 2.0 * s),
                );
                let cell = |v: f64| ((v / s).floor() as i64).clamp(0, i64::from(n) - 1) as u32;
                for y in cell(lo.y)..=cell(hi.y) {
                    for x in cell(lo.x)..=cell(hi.x) {
                        let p = DVec2::new(f64::from(x) * s, f64::from(y) * s);
                        let d = geom::distance_to_segment(p, w[0], w[1]);
                        let v = (0.5 + (reach - d) / (3.0 * s)).clamp(0.0, 1.0) as f32;
                        if v > river.get(x, y) {
                            river.set(x, y, v);
                        }
                    }
                }
            }
        }
        Self { sea, river }
    }

    /// Samples per side.
    pub fn size(&self) -> u32 {
        self.sea.size
    }

    /// Metres between samples.
    pub fn spacing(&self) -> f64 {
        self.sea.spacing
    }

    fn sample(&self, x: u32, y: u32) -> f64 {
        f64::from(self.sea.get(x, y).max(self.river.get(x, y)))
    }

    /// The wetness at `p`, bilinear between the samples (clamped to the frame).
    pub fn wetness(&self, p: DVec2) -> f64 {
        let (sea, river) = self.parts(p);
        sea.max(river)
    }

    /// The open water's and the rivers' wetness at `p`, bilinear.
    pub fn parts(&self, p: DVec2) -> (f64, f64) {
        let n = self.size();
        let s = self.spacing();
        let last = f64::from(n - 1);
        let (fx, fy) = ((p.x / s).clamp(0.0, last), (p.y / s).clamp(0.0, last));
        let (x0, y0) = (
            (fx as u32).min(n.saturating_sub(2)),
            (fy as u32).min(n.saturating_sub(2)),
        );
        let (tx, ty) = (fx - f64::from(x0), fy - f64::from(y0));
        let lerp = |f: &Field2<f32>| {
            let g = |x: u32, y: u32| f64::from(f.get(x, y));
            let top = g(x0, y0) + (g(x0 + 1, y0) - g(x0, y0)) * tx;
            let bottom = g(x0, y0 + 1) + (g(x0 + 1, y0 + 1) - g(x0, y0 + 1)) * tx;
            top + (bottom - top) * ty
        };
        (lerp(&self.sea), lerp(&self.river))
    }

    /// Whether `p` is in the water: its wetness is one half or more.
    pub fn is_wet(&self, p: DVec2) -> bool {
        self.wetness(p) >= 0.5
    }

    /// Whether the square between samples `(x, y)` and `(x + 1, y + 1)` is all dry (`Some(false)`),
    /// all wet (`Some(true)`) or crossed by the shore (`None`).
    pub fn whole(&self, x: u32, y: u32) -> Option<bool> {
        let wet =
            [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(dx, dy)| self.sample(x + dx, y + dy) >= 0.5);
        wet.iter().all(|&w| w == wet[0]).then_some(wet[0])
    }

    /// The square between samples `(x, y)` and `(x + 1, y + 1)` cut along the shore. Where two
    /// opposite corners are wet and the other two dry, the water joins across the middle.
    pub fn square(&self, x: u32, y: u32) -> Square {
        let s = self.spacing();
        let corners = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(dx, dy)| {
            (
                DVec2::new(f64::from(x + dx) * s, f64::from(y + dy) * s),
                self.sample(x + dx, y + dy),
            )
        });
        let wet = corners.map(|c| c.1 >= 0.5);
        // The shore's point on edge k (from corner k to the next), where it crosses.
        let crossing = |k: usize| {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            (wet[k] != wet[(k + 1) % 4]).then(|| {
                let t = ((0.5 - a.1) / (b.1 - a.1)).clamp(0.0, 1.0);
                a.0 + (b.0 - a.0) * t
            })
        };
        // A walk round the square keeping the corners of one kind and the crossings: one piece
        // (a corner's kind, the crossing flag per point).
        let walk = |kind: bool| {
            let mut piece = Vec::new();
            for k in 0..4 {
                if wet[k] == kind {
                    piece.push((corners[k].0, false));
                }
                if let Some(c) = crossing(k) {
                    piece.push((c, true));
                }
            }
            piece
        };
        let saddle = wet[0] == wet[2] && wet[1] == wet[3] && wet[0] != wet[1];
        let (mut dry, wet_pieces) = if saddle {
            // The water joins: one wet piece; the two dry corners apart.
            let dry_corners: Vec<usize> = (0..4).filter(|&k| !wet[k]).collect();
            let dry = dry_corners
                .iter()
                .map(|&k| {
                    let before = crossing((k + 3) % 4).expect("a crossing before");
                    let after = crossing(k).expect("a crossing after");
                    vec![(before, true), (corners[k].0, false), (after, true)]
                })
                .collect();
            (dry, vec![walk(true)])
        } else {
            let (d, w) = (walk(false), walk(true));
            (
                if d.len() >= 3 { vec![d] } else { Vec::new() },
                if w.len() >= 3 { vec![w] } else { Vec::new() },
            )
        };
        let mut shore = Vec::new();
        for piece in &dry {
            for i in 0..piece.len() {
                let (a, b) = (piece[i], piece[(i + 1) % piece.len()]);
                if a.1 && b.1 && a.0.distance(b.0) > 1e-6 {
                    let t = (b.0 - a.0).normalize();
                    let mut out = DVec2::new(-t.y, t.x);
                    let mid = (a.0 + b.0) * 0.5;
                    if self.wetness(mid + out * (0.25 * s)) < self.wetness(mid - out * (0.25 * s)) {
                        out = -out;
                    }
                    shore.push(([a.0, b.0], out));
                }
            }
        }
        let ring = |piece: Vec<(DVec2, bool)>| {
            let mut ring: Vec<DVec2> = piece.into_iter().map(|p| p.0).collect();
            ring.dedup_by(|a, b| a.distance(*b) < 1e-9);
            if geom::signed_area(&ring) < 0.0 {
                ring.reverse();
            }
            ring
        };
        let keep = |r: &Vec<DVec2>| r.len() >= 3 && geom::signed_area(r).abs() > 1e-9;
        Square {
            dry: dry.drain(..).map(ring).filter(keep).collect(),
            wet: wet_pieces.into_iter().map(ring).filter(keep).collect(),
            shore,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::Waterway;

    #[test]
    fn a_river_cut_square_by_square_keeps_its_area_and_its_banks() {
        let mut site = Site::flat(400.0, 10.0);
        // A river at a slant, 40 m wide: its banks cross the cells at every phase.
        site.rivers.push(Waterway {
            name: "test".into(),
            centreline: vec![DVec2::new(0.0, 120.0), DVec2::new(400.0, 260.0)],
            width: 40.0,
        });
        let shore = Shore::new(&site, 0.0);
        let mut wet_area = 0.0;
        let mut worst: f64 = 0.0;
        let line = &site.rivers[0].centreline;
        for y in 0..shore.size() - 1 {
            for x in 0..shore.size() - 1 {
                let sq = shore.square(x, y);
                let area = |pieces: &[Vec<DVec2>]| {
                    pieces.iter().map(|p| geom::signed_area(p)).sum::<f64>()
                };
                // The pieces fill the square exactly.
                assert!(
                    (area(&sq.dry) + area(&sq.wet) - 100.0).abs() < 1e-6,
                    "square {x} {y}"
                );
                wet_area += area(&sq.wet);
                for (seg, out) in &sq.shore {
                    for p in seg {
                        let d = geom::distance_to_segment(*p, line[0], line[1]);
                        worst = worst.max((d - 20.0).abs());
                    }
                    // Towards the water: the centreline is that way.
                    let mid = (seg[0] + seg[1]) * 0.5;
                    let towards = geom::distance_to_segment(mid + *out, line[0], line[1]);
                    assert!(towards < geom::distance_to_segment(mid, line[0], line[1]));
                }
            }
        }
        // The banks lie on the river's edges within centimetres, its area within a percent
        // (the frame cuts it at both ends).
        assert!(worst < 0.05, "a bank {worst} m off");
        let expected = 40.0 * DVec2::new(400.0, 140.0).length();
        assert!(
            (wet_area - expected).abs() < expected * 0.01,
            "{wet_area} against {expected}"
        );
    }

    #[test]
    fn the_sea_shore_is_smoother_than_its_cells() {
        // A coast at a shallow slant: the mask's staircase steps a cell every five.
        let mut site = Site::flat(400.0, 10.0);
        let n = site.water.size;
        for y in 0..n {
            for x in 0..n {
                if f64::from(y) > 20.0 + f64::from(x) * 0.2 {
                    site.water.set(x, y, SEA);
                }
            }
        }
        let shore = Shore::new(&site, 0.0);
        // The shore's points against the coast y = 201 + 0.2 x (half way between the last dry
        // and the first wet sample, on average): within a fifth of a cell; the cells' staircase is
        // up to half a cell off.
        let mut worst: f64 = 0.0;
        for y in 5..n - 6 {
            for x in 5..n - 6 {
                for (seg, out) in shore.square(x, y).shore {
                    assert!(out.y > 0.5, "the sea lies south");
                    for p in seg {
                        let coast = 201.0 + 0.2 * p.x;
                        worst = worst.max((p.y - coast).abs());
                    }
                }
            }
        }
        assert!(worst < 2.0, "the shore strays {worst} m from the coast");
    }
}
