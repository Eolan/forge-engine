//! Grown plants meshed for the renderer (D-063, #296; city-plan's forest until then): a plant's
//! branch graph ([`forge_flora::grow`]) as wood, crown lumps and leaf cards ([`grown_mesh`]), and
//! the opaque proxy the probes' rays meet in the cards' stead ([`grown_proxy`], #297). Sections:
//! 0 the bark, 1 the crown's cores, 2 the leaf cards (cut from the leaf atlas,
//! `forge_render::textures::leaves`), 3 the proxy's crown. [`grow_plant`] grows and meshes one.

use forge_flora::Conditions;
use forge_flora::graph::TreeGraph;
use forge_flora::species::{Foliage, Habit, LeafSpray, Species};
use glam::Vec3;

use crate::procedural::TriMesh;

/// A grown plant's meshes: the drawn one ([`grown_mesh`]) and the probes' proxy
/// ([`grown_proxy`]), with its height and its crown's width, metres.
pub struct GrownPlant {
    /// The plant as drawn: sections 0 to 2.
    pub mesh: TriMesh,
    /// What the probes' rays meet in its stead: sections 0 and 3.
    pub proxy: TriMesh,
    /// Its height, at least 10 cm.
    pub height: f32,
    /// Its crown's width, at least 10 cm.
    pub crown: f32,
}

/// Species `s` grown in `conditions` and meshed (`forge_flora::grow::individual` gives a forest's
/// individuals): its habit and foliage set the lumps' grid, its look the cards' spray, the
/// conditions' pruning the lifted crown.
pub fn grow_plant(s: &Species, conditions: &Conditions) -> GrownPlant {
    let shrub = s.habit == Habit::Shrub;
    let broad = s.foliage == Foliage::Broad;
    let graph = forge_flora::grow(s, conditions);
    let lift = conditions.pruning.lift as f32;
    GrownPlant {
        mesh: grown_mesh(&graph, shrub, broad, lift, s.look.spray),
        proxy: grown_proxy(&graph, shrub, broad, lift),
        height: graph.height().max(0.1),
        crown: graph.crown_width().max(0.1),
    }
}

/// A grown plant's mesh (`forge_flora`, D-063), a tree's few thousand triangles and a shrub's a
/// few hundred, the full graph's being 2 000 to 250 000: a tree's wood 5 cm thick or more as
/// tapered limbs (section 0; a shrub's twigs hide in its leaves), its leaf clusters gathered
/// on a grid of a sixth of its crown's width (an eighth of a broad-leaved tree's, a third of a
/// shrub's) or a tenth of its height, whichever is larger (a cypress's narrow column took 50 000
/// triangles), each cell's into one lump at their middle: a core of 20 triangles (section 1) in
/// leaf cards cut from `spray` (section 2, [`leaf_cards`]). The cells are keyed in order, so a
/// plant's mesh is the same from run to run.
pub fn grown_mesh(g: &TreeGraph, shrub: bool, broad: bool, lift: f32, spray: LeafSpray) -> TriMesh {
    let mut mesh = TriMesh::default();
    if !shrub {
        wood(&mut mesh, g);
    }
    let lumps = crown_lumps(g, shrub, broad, lift);
    let faces = icosahedron();
    // The crown's middle, which the cards' normals bend away from with their lump's.
    let middle = lumps.iter().fold(Vec3::ZERO, |s, l| s + l.0) / lumps.len().max(1) as f32;
    for (i, &(centre, radius)) in lumps.iter().enumerate() {
        // A core of two fifths of the lump, the shade the cards hide and the far crown keeps
        // (it simplifies as a closed mesh; the cards cannot), then the cards round it.
        lump_of(&mut mesh, centre, radius * CROWN_CORE, i as u32 % 8, &faces);
        leaf_cards(&mut mesh, centre, radius, middle, spray, shrub, i as u32);
    }
    let missing = mesh.positions.len() - mesh.uvs.len();
    mesh.uvs.extend(std::iter::repeat_n([0.0, 0.0], missing));
    mesh
}

/// [`grown_mesh`]'s plant as the probes' rays see it (#297): its wood, and each lump of its
/// crown a closed shell of [`CROWN_PROXY`] of the lump's radius in the cards' stead (section 3,
/// the leaves' colour, opaque). A probe ray crossing a crown tested each leaf card it met against
/// the leaf atlas (a candidate each), and the probes' light is too soft to show a leaf.
pub fn grown_proxy(g: &TreeGraph, shrub: bool, broad: bool, lift: f32) -> TriMesh {
    let mut mesh = TriMesh::default();
    if !shrub {
        wood(&mut mesh, g);
    }
    let faces = icosahedron();
    for (i, &(centre, radius)) in crown_lumps(g, shrub, broad, lift).iter().enumerate() {
        let first = mesh.sections.len();
        lump_of(
            &mut mesh,
            centre,
            radius * CROWN_PROXY,
            i as u32 % 8,
            &faces,
        );
        mesh.sections[first..].fill(3);
    }
    mesh.uvs = vec![[0.0, 0.0]; mesh.positions.len()];
    mesh
}

/// A tree's wood 5 cm thick or more as tapered limbs, section 0 ([`grown_mesh`]).
fn wood(mesh: &mut TriMesh, g: &TreeGraph) {
    for s in g.segments() {
        let node = &g.nodes[s.node as usize];
        let parent = g.nodes[node.parent as usize].radius.max(s.radius);
        if s.radius >= 0.05 && s.a.distance(s.b) > 1e-3 {
            limb(mesh, s.a, s.b, parent, s.radius);
        }
    }
}

/// A grown crown's lumps ([`grown_mesh`]), their centres and radii: its leaves gathered on a grid
/// of a sixth of its width (an eighth of a broad-leaved tree's, a third of a shrub's) or a tenth
/// of its height, whichever is larger, each cell's into one lump at their middle, kept over a
/// lifted crown's clear trunk.
pub fn crown_lumps(g: &TreeGraph, shrub: bool, broad: bool, lift: f32) -> Vec<(Vec3, f32)> {
    let width = g.crown_width();
    let cell = (width
        / if shrub {
            3.0
        } else if broad {
            8.0
        } else {
            6.0
        })
    .max(g.height() / 10.0)
    .max(0.3);
    let mut cells: std::collections::BTreeMap<(i32, i32, i32), (Vec3, u32, f32)> =
        Default::default();
    for l in &g.leaves {
        let k = (l.pos / cell).floor();
        let e = cells
            .entry((k.x as i32, k.y as i32, k.z as i32))
            .or_insert((Vec3::ZERO, 0, 0.0));
        e.0 += l.pos;
        e.1 += 1;
        e.2 = e.2.max(l.size);
    }
    let mean = g.leaves.len() as f32 / cells.len().max(1) as f32;
    cells
        .into_values()
        .map(|(sum, n, size)| {
            let radius = cell * (0.45 + 0.35 * (n as f32 / mean).sqrt().min(1.6)) + size * 0.3;
            // Kept over a lifted crown's clear trunk: a lump round a low cluster hung below it.
            let mut centre = sum / n as f32;
            centre.y = centre.y.max(lift + radius * 0.7);
            (centre, radius)
        })
        .collect()
}

/// A crown lump's core, as a share of the lump's radius ([`grown_mesh`]): three tenths, drawn
/// darker, the shade between the cards (at 0.6 in the leaves' colour, then at half with 12 cards
/// at most a lump, the near holm oaks showed it as balls; at two fifths, once the metered
/// exposure lit the wood, as faceted green stones among the leaves).
pub const CROWN_CORE: f32 = 0.3;

/// A crown proxy's lump ([`grown_proxy`]) as a share of the lump's radius: three fifths, the
/// light nearest the cards' (#297; the village at 15:00, a fixed exposure, against the probes
/// through the cards). Under the main street's holm oaks −0.15 EV and ꟻLIP 0.074, against −0.32
/// EV and 0.099 at 0.8, +0.04 EV and 0.080 at half; the other views within +0.07 EV. Without the
/// plants in the probes at all, the street's shade was +2.45 EV.
pub const CROWN_PROXY: f32 = 0.6;

/// Leaf cards round a crown's lump at `centre`, `radius` across (D-013), section 2, cut from
/// `spray`'s quarter of the leaf atlas (`forge_render::textures::leaves`): 6 to 16 squares (a shrub's 4),
/// their middles 0.4 to 0.7 radii out along directions spread over the sphere and turned by
/// `seed`, facing out with a tilt, their sprays growing upwards. Their normals bend out from the
/// crown's `middle` and the lump's, so the crown is lit as a rounded mass of leaves.
pub fn leaf_cards(
    mesh: &mut TriMesh,
    centre: Vec3,
    radius: f32,
    middle: Vec3,
    spray: LeafSpray,
    shrub: bool,
    seed: u32,
) {
    let ([u0, v0], [u1, v1]) = spray.uv_box();
    // Leaves near their own size: a card's half side at most 0.6 m for broad leaves (a plane's
    // 20–25 cm), 0.4 m for small and narrow ones, 0.5 m for needles; a card of 0.75 lump radii
    // drew a plane's leaves 40–60 cm wide by the eye. More cards where they are smaller, so the
    // lump keeps its cover.
    let cap = match spray {
        LeafSpray::Broad => 0.6,
        LeafSpray::Needles => 0.5,
        LeafSpray::Small | LeafSpray::Narrow => 0.4,
    };
    let half = (radius * 0.75).min(cap);
    let count = if shrub {
        4
    } else {
        (6.0 * (radius * 0.75 / half).powi(2))
            .round()
            .clamp(6.0, 16.0) as u32
    };
    let random = |k: u32, salt: u32| {
        let h = (seed.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B) ^ salt)
            .wrapping_mul(0xC2B2_AE35);
        ((h ^ (h >> 15)) & 0xFFFF) as f32 / 65535.0
    };
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let spin = random(0, 1) * std::f32::consts::TAU;
    for k in 0..count {
        // A Fibonacci spiral over the sphere.
        let y = 1.0 - 2.0 * (k as f32 + 0.5) / count as f32;
        let ring = (1.0 - y * y).max(0.0).sqrt();
        let a = spin + golden * k as f32;
        let out = Vec3::new(ring * a.cos(), y, ring * a.sin());
        let tilt = out.any_orthonormal_vector();
        let tilt = (tilt * (random(k, 2) - 0.5) + out.cross(tilt) * (random(k, 3) - 0.5)) * 1.2;
        let normal = (out + tilt).normalize();
        // The spray grows upwards, turned a little about the card's normal.
        let level = Vec3::Y - normal * normal.dot(Vec3::Y);
        let up = if level.length_squared() > 1e-4 {
            level.normalize()
        } else {
            normal.any_orthonormal_vector()
        };
        let turn = (random(k, 4) - 0.5) * 1.4;
        let up = glam::Quat::from_axis_angle(normal, turn) * up;
        let side = up.cross(normal);
        let at =
            centre + Vec3::new(out.x, out.y * 0.85, out.z) * radius * (0.4 + 0.3 * random(k, 5));
        let corners = [
            (at - up * half - side * half, [u0, v1]),
            (at - up * half + side * half, [u1, v1]),
            (at + up * half + side * half, [u1, v0]),
            (at + up * half - side * half, [u0, v0]),
        ];
        let missing = mesh.positions.len() - mesh.uvs.len();
        mesh.uvs.extend(std::iter::repeat_n([0.0, 0.0], missing));
        let base = mesh.positions.len() as u32;
        for (p, uv) in corners {
            let bent = ((p - middle).normalize_or_zero() + (p - centre).normalize_or_zero())
                .normalize_or(normal);
            mesh.positions.push(p.to_array());
            mesh.normals.push(bent.to_array());
            mesh.uvs.push(uv);
        }
        // Counter-clockwise seen from the normal's side.
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        mesh.sections.extend([2, 2]);
    }
}

/// A triangle with a normal per corner, turned so it faces along their sum.
fn smooth_triangle(mesh: &mut TriMesh, mut p: [Vec3; 3], mut n: [Vec3; 3], section: u8) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(n[0] + n[1] + n[2]) < 0.0 {
        p.swap(1, 2);
        n.swap(1, 2);
    }
    let base = mesh.positions.len() as u32;
    for (q, m) in p.into_iter().zip(n) {
        mesh.positions.push(q.to_array());
        mesh.normals.push(m.to_array());
    }
    mesh.indices.extend([base, base + 1, base + 2]);
    mesh.sections.push(section);
}

/// A tapering seven-sided limb from `a` (radius `ra`) to `b` (radius `rb`), section 0.
pub fn limb(mesh: &mut TriMesh, a: Vec3, b: Vec3, ra: f32, rb: f32) {
    let axis = (b - a).normalize();
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let sides = 7;
    let ring = |k: u32| {
        let t = k as f32 / sides as f32 * std::f32::consts::TAU;
        u * t.cos() + v * t.sin()
    };
    for k in 0..sides {
        let (d0, d1) = (ring(k), ring(k + 1));
        let (p0, p1) = (a + d0 * ra, a + d1 * ra);
        let (q0, q1) = (b + d0 * rb, b + d1 * rb);
        smooth_triangle(mesh, [p0, p1, q1], [d0, d1, d1], 0);
        smooth_triangle(mesh, [p0, q1, q0], [d0, d1, d0], 0);
    }
}

/// A crown's lump round `centre`, about `radius` across, a little flattened, its surface
/// rippled by `seed`; section 1.
pub fn lump(mesh: &mut TriMesh, centre: Vec3, radius: f32, seed: u32) {
    lump_of(mesh, centre, radius, seed, &icosphere());
}

/// [`lump`] on the faces `faces` (unit directions): an [`icosahedron`]'s 20 for a shrub's.
pub fn lump_of(mesh: &mut TriMesh, centre: Vec3, radius: f32, seed: u32, faces: &[[Vec3; 3]]) {
    let s = seed as f32 * 1.7;
    let place = |d: Vec3| {
        let d = d.normalize();
        let bump = 1.0
            + 0.09 * (d.x * 7.3 + d.y * 3.1 + d.z * 5.7 + s).sin()
            + 0.05 * (d.x * 13.1 - d.z * 11.3 + s * 2.3).sin();
        (
            centre + Vec3::new(d.x, d.y * 0.85, d.z) * radius * bump,
            Vec3::new(d.x, d.y / 0.85, d.z).normalize(),
        )
    };
    for &tri in faces {
        let p = tri.map(place);
        smooth_triangle(mesh, p.map(|q| q.0), p.map(|q| q.1), 1);
    }
}

/// An icosahedron's 20 faces, their corners as directions (not of unit length).
pub fn icosahedron() -> Vec<[Vec3; 3]> {
    let t = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let v: [Vec3; 12] = [
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ];
    let faces: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    faces.iter().map(|f| [v[f[0]], v[f[1]], v[f[2]]]).collect()
}

/// An icosahedron's faces, each cut in four: 80 triangles of unit directions.
pub fn icosphere() -> Vec<[Vec3; 3]> {
    let mut out = Vec::with_capacity(80);
    for [a, b, c] in icosahedron() {
        let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
        out.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forest's grown plants keep to their budgets: a tree 40 000 triangles at most, a
    /// shrub 1 500 (the full graphs' run to 250 000), each proxy fewer than its plant.
    #[test]
    fn the_grown_plants_keep_their_budgets() {
        for (sp, s) in forge_flora::species::mediterranean().iter().enumerate() {
            let shrub = s.habit == Habit::Shrub;
            for k in 0..forge_flora::grow::INDIVIDUALS {
                let plant = grow_plant(s, &forge_flora::grow::individual(s, sp as u16, k));
                let (m, proxy) = (plant.mesh.triangle_count(), plant.proxy.triangle_count());
                let budget = if shrub { 1_500 } else { 40_000 };
                println!("{} {k}: {m} triangles, its proxy {proxy}", s.id);
                assert!(m > 0 && m <= budget, "{} {k}: {m} triangles", s.id);
                assert!(
                    proxy > 0 && proxy < m,
                    "{} {k}: a proxy of {proxy} triangles",
                    s.id
                );
            }
        }
    }
}
