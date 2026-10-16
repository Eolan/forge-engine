//! A grown plant as a branch graph: nodes with their parent and radius, leaf clusters on the
//! twigs, and the measures and checks the tests and the docs read (height, crown width, branch
//! count, taper, branches passing through each other).

use glam::Vec3;

/// No parent: the root.
pub const NO_PARENT: u32 = u32::MAX;

/// A node of the branch graph: the end of the segment from its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Node {
    /// Its position, metres from the plant's base (+Y up).
    pub pos: Vec3,
    /// Its parent's index, [`NO_PARENT`] for the root. Parents come before their children.
    pub parent: u32,
    /// The radius of the segment from its parent to it, metres (the root's: the stem's base).
    pub radius: f32,
    /// Its branch order: 0 the trunk or the stems, one more per lateral.
    pub order: u8,
    /// Whether it starts a branch (a lateral of its parent, or a stem from the root).
    pub starts_branch: bool,
}

/// A leaf cluster on a twig: one card's or one tuft's place for the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LeafCluster {
    /// Its centre, metres from the plant's base.
    pub pos: Vec3,
    /// The twig's direction there (unit).
    pub dir: Vec3,
    /// Its radius, metres.
    pub size: f32,
    /// The node it hangs from.
    pub node: u32,
}

/// A grown plant.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeGraph {
    /// The species' id.
    pub species: String,
    /// Its age, years.
    pub age: f32,
    /// The seed it grew from.
    pub seed: u64,
    /// The nodes, parents first; node 0 is the root at the base.
    pub nodes: Vec<Node>,
    /// The leaf clusters.
    pub leaves: Vec<LeafCluster>,
}

/// A segment of wood between a node and its parent.
#[derive(Clone, Copy, Debug)]
pub struct Segment {
    /// The node at its far end (its index).
    pub node: u32,
    /// Its start (the parent's position).
    pub a: Vec3,
    /// Its end.
    pub b: Vec3,
    /// Its radius.
    pub radius: f32,
}

impl TreeGraph {
    /// Its height: the highest node or leaf cluster's top, metres.
    pub fn height(&self) -> f32 {
        let wood = self.nodes.iter().map(|n| n.pos.y).fold(0.0, f32::max);
        let leaves = self
            .leaves
            .iter()
            .map(|l| l.pos.y + l.size)
            .fold(0.0, f32::max);
        wood.max(leaves)
    }

    /// Its crown's width: the mean of its extents along X and Z, wood and leaf clusters,
    /// metres (a crown's spread as a nursery or a flora gives it).
    pub fn crown_width(&self) -> f32 {
        let (lo, hi) = self.extent();
        ((hi.x - lo.x) + (hi.z - lo.z)) * 0.5
    }

    /// The box round its wood and leaves.
    pub fn extent(&self) -> (Vec3, Vec3) {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for n in &self.nodes {
            lo = lo.min(n.pos);
            hi = hi.max(n.pos);
        }
        for l in &self.leaves {
            lo = lo.min(l.pos - Vec3::splat(l.size));
            hi = hi.max(l.pos + Vec3::splat(l.size));
        }
        (lo, hi)
    }

    /// The number of branches: the stems and every lateral that grew.
    pub fn branch_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.starts_branch).count()
    }

    /// The trunk's base radius (the thickest segment from the root), metres.
    pub fn base_radius(&self) -> f32 {
        self.nodes
            .iter()
            .filter(|n| n.parent == 0)
            .map(|n| n.radius)
            .fold(0.0, f32::max)
    }

    /// The total length of wood, metres.
    pub fn wood_length(&self) -> f32 {
        self.segments().map(|s| s.a.distance(s.b)).sum()
    }

    /// The segments of wood.
    pub fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent != NO_PARENT)
            .map(|(i, n)| Segment {
                node: i as u32,
                a: self.nodes[n.parent as usize].pos,
                b: n.pos,
                radius: n.radius,
            })
    }

    /// The leaf clusters' mean position: the crown's centre.
    pub fn crown_centre(&self) -> Vec3 {
        if self.leaves.is_empty() {
            return Vec3::ZERO;
        }
        self.leaves.iter().map(|l| l.pos).sum::<Vec3>() / self.leaves.len() as f32
    }

    /// The nodes thicker than their parent's segment (none, by the pipe model).
    pub fn taper_faults(&self) -> Vec<u32> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent != NO_PARENT && n.parent != 0)
            .filter(|(_, n)| n.radius > self.nodes[n.parent as usize].radius * 1.000_01)
            .map(|(i, _)| i as u32)
            .collect()
    }

    /// The pairs of segments that pass through each other: a segment's axis inside another's
    /// wood (closer than the thicker one's radius), for segments more than `hops` steps apart
    /// along the graph (closer ones meet at a fork, where the wood joins).
    pub fn crossings(&self, hops: u32) -> Vec<(u32, u32)> {
        let segs: Vec<Segment> = self.segments().collect();
        let mut out = Vec::new();
        if segs.is_empty() {
            return out;
        }
        let reach = segs.iter().map(|s| s.a.distance(s.b)).fold(0.0, f32::max)
            + 2.0 * segs.iter().map(|s| s.radius).fold(0.0, f32::max);
        let grid = SegmentGrid::new(&segs, reach.max(0.05));
        for (i, s) in segs.iter().enumerate() {
            grid.near(s, |j| {
                if j <= i {
                    return;
                }
                let t = &segs[j];
                let d = segment_distance(s.a, s.b, t.a, t.b);
                if d < s.radius.max(t.radius) && hop_distance(self, s.node, t.node, hops) > hops {
                    out.push((s.node, t.node));
                }
            });
        }
        out
    }
}

/// The number of steps between two nodes along the graph, or `limit + 1` when more than
/// `limit`.
pub fn hop_distance(g: &TreeGraph, a: u32, b: u32, limit: u32) -> u32 {
    let mut ups = Vec::with_capacity(limit as usize + 1);
    let mut n = a;
    for k in 0..=limit {
        ups.push((n, k));
        let p = g.nodes[n as usize].parent;
        if p == NO_PARENT {
            break;
        }
        n = p;
    }
    let mut n = b;
    for k in 0..=limit {
        if let Some(&(_, ka)) = ups.iter().find(|(m, _)| *m == n) {
            return ka + k;
        }
        let p = g.nodes[n as usize].parent;
        if p == NO_PARENT {
            break;
        }
        n = p;
    }
    limit + 1
}

/// The shortest distance between two segments.
pub fn segment_distance(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> f32 {
    // Ericson, *Real-Time Collision Detection*, 5.1.9, in f64 for the near-parallel cases.
    let (p1, q1, p2, q2) = (p1.as_dvec3(), q1.as_dvec3(), p2.as_dvec3(), q2.as_dvec3());
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.dot(d1);
    let e = d2.dot(d2);
    let f = d2.dot(r);
    let eps = 1e-12;
    let (s, t);
    if a <= eps && e <= eps {
        return (p1 - p2).length() as f32;
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s0 = if denom > eps {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t0 = 0.0;
                s0 = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t0 = 1.0;
                s0 = ((b - c) / a).clamp(0.0, 1.0);
            }
            s = s0;
            t = t0;
        }
    }
    ((p1 + d1 * s) - (p2 + d2 * t)).length() as f32
}

/// Segments binned by their midpoints in cubes of `cell`, for neighbour queries.
pub(crate) struct SegmentGrid {
    lo: Vec3,
    cell: f32,
    dims: [i32; 3],
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl SegmentGrid {
    pub(crate) fn new(segs: &[Segment], cell: f32) -> Self {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for s in segs {
            let m = (s.a + s.b) * 0.5;
            lo = lo.min(m);
            hi = hi.max(m);
        }
        let dims = [0, 1, 2].map(|k| (((hi[k] - lo[k]) / cell).floor() as i32 + 1).max(1));
        let count = (dims[0] * dims[1] * dims[2]) as usize;
        let key = |m: Vec3| -> usize {
            let c = ((m - lo) / cell).floor().as_ivec3();
            let c = c.clamp(glam::IVec3::ZERO, glam::IVec3::from(dims) - 1);
            ((c.z * dims[1] + c.y) * dims[0] + c.x) as usize
        };
        let mut counts = vec![0u32; count + 1];
        for s in segs {
            counts[key((s.a + s.b) * 0.5) + 1] += 1;
        }
        for k in 1..counts.len() {
            counts[k] += counts[k - 1];
        }
        let mut fill = counts.clone();
        let mut items = vec![0u32; segs.len()];
        for (i, s) in segs.iter().enumerate() {
            let k = key((s.a + s.b) * 0.5);
            items[fill[k] as usize] = i as u32;
            fill[k] += 1;
        }
        Self {
            lo,
            cell,
            dims,
            starts: counts,
            items,
        }
    }

    /// Calls `f` with every segment whose midpoint lies in the cells round `s`'s midpoint
    /// (one cell out: segments whose midpoints are within one cell).
    pub(crate) fn near(&self, s: &Segment, mut f: impl FnMut(usize)) {
        let m = (s.a + s.b) * 0.5;
        let c = ((m - self.lo) / self.cell).floor().as_ivec3();
        for z in (c.z - 1).max(0)..=(c.z + 1).min(self.dims[2] - 1) {
            for y in (c.y - 1).max(0)..=(c.y + 1).min(self.dims[1] - 1) {
                for x in (c.x - 1).max(0)..=(c.x + 1).min(self.dims[0] - 1) {
                    let k = ((z * self.dims[1] + y) * self.dims[0] + x) as usize;
                    for &i in &self.items[self.starts[k] as usize..self.starts[k + 1] as usize] {
                        f(i as usize);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distances() {
        let d = segment_distance(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.5, 1.0, -1.0),
            Vec3::new(0.5, 1.0, 1.0),
        );
        assert!((d - 1.0).abs() < 1e-6);
        let parallel = segment_distance(
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(1.0, 0.5, 0.0),
        );
        assert!((parallel - 0.5).abs() < 1e-6);
    }
}
