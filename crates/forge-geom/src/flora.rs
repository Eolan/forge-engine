//! Grown plants meshed for the renderer (D-063, #296; city-plan's forest until then): a plant's
//! branch graph ([`forge_flora::grow`]) as wood, crown lumps and leaf cards ([`grown_mesh`]), and
//! the opaque proxy the probes' rays meet in the cards' stead ([`grown_proxy`], #297). Sections:
//! 0 the bark, 1 the crown's cores, 2 the leaf cards (cut from the leaf atlas,
//! `forge_render::textures::leaves`), 3 the proxy's crown, 4 the crown's shells on its ladder
//! ([`grown_shells`], D-075). [`grow_plant`] grows and meshes one.

use forge_flora::Conditions;
use forge_flora::graph::TreeGraph;
use forge_flora::species::{Foliage, Habit, LeafSpray, Species};
use glam::Vec3;

use crate::procedural::TriMesh;

/// A grown plant's meshes: the drawn one ([`grown_mesh`]), the probes' proxy ([`grown_proxy`])
/// and the next step on its ladder ([`grown_shells`]), with its height and its crown's width, metres.
pub struct GrownPlant {
    /// The plant as drawn: sections 0 to 2.
    pub mesh: TriMesh,
    /// What the probes' rays meet in its stead: sections 0 and 3.
    pub proxy: TriMesh,
    /// The plant at the middle of its ladder (D-075's L1, [`grown_shells`]): sections 0 and 4.
    pub shells: TriMesh,
    /// What the shadow rays from far points meet in its stead ([`grown_shadow_proxy`]): sections
    /// 0 and 3.
    pub shadow: TriMesh,
    /// Its crown's shells' mean diameter, metres: where they stand in for the cards, a lump
    /// should be a few pixels across (the ladder's crossing, D-075).
    pub lump: f32,
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
    let lumps = crown_lumps(&graph, shrub, broad, lift);
    let lump =
        2.0 * CROWN_SHELL * lumps.iter().map(|l| l.1).sum::<f32>() / lumps.len().max(1) as f32;
    GrownPlant {
        mesh: grown_mesh(&graph, shrub, broad, lift, s.look.spray),
        proxy: grown_proxy(&graph, shrub, broad, lift),
        shells: grown_shells(&graph, shrub, broad, lift),
        shadow: grown_shadow_proxy(&graph, shrub, broad, lift),
        lump: lump.max(0.05),
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
        wood(&mut mesh, g, WOOD_RADIUS);
    }
    let lumps = crown_lumps(g, shrub, broad, lift);
    let faces = icosahedron();
    // The crown's middle, which the cards' normals bend away from with their lump's.
    let middle = lumps.iter().fold(Vec3::ZERO, |s, l| s + l.0) / lumps.len().max(1) as f32;
    for (i, &(centre, radius)) in lumps.iter().enumerate() {
        // A core of a fifth of the lump, the shade the cards hide and the far crown keeps
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
    proxy_of(g, shrub, broad, lift, CROWN_PROXY)
}

/// [`grown_mesh`]'s plant as the shadow rays from far points see it (D-075 step 3): its wood, and
/// each lump of its crown a closed shell of [`CROWN_SHADOW`] of the lump's radius (section 3,
/// opaque), whose shadow stands for the cards' without a candidate to test.
pub fn grown_shadow_proxy(g: &TreeGraph, shrub: bool, broad: bool, lift: f32) -> TriMesh {
    proxy_of(g, shrub, broad, lift, CROWN_SHADOW)
}

/// A crown's far shadow proxy's lump ([`grown_shadow_proxy`]) as a share of the lump's radius:
/// the cards' shadow is as wide as their lumps but full of holes (#302's plan; the probes' three
/// fifths would shrink the far shadows at the switch).
pub const CROWN_SHADOW: f32 = 0.8;

/// The plant's wood and each lump of its crown a closed shell of `share` of its radius,
/// section 3.
fn proxy_of(g: &TreeGraph, shrub: bool, broad: bool, lift: f32, share: f32) -> TriMesh {
    let mut mesh = TriMesh::default();
    if !shrub {
        wood(&mut mesh, g, WOOD_RADIUS);
    }
    let faces = icosahedron();
    for (i, &(centre, radius)) in crown_lumps(g, shrub, broad, lift).iter().enumerate() {
        let first = mesh.sections.len();
        lump_of(&mut mesh, centre, radius * share, i as u32 % 8, &faces);
        mesh.sections[first..].fill(3);
    }
    mesh.uvs = vec![[0.0, 0.0]; mesh.positions.len()];
    mesh
}

/// [`grown_mesh`]'s plant at the middle of D-075's ladder (L1, #302): its wood, and each lump of
/// its crown a closed shell of [`CROWN_SHELL`] of the lump's radius, a bumped icosahedron of 20
/// triangles (section 4, a row of its own: a lump is a few pixels where it stands in for the
/// cards, [`GrownPlant::lump`]), fewer than the cards and their core:
/// closed, the cluster LOD simplifies it, as it cannot simplify the cards. Its normals bend as
/// the cards' do. Not #297's proxy for the probes, which keeps to three fifths of the lumps and
/// would shrink the crown in sight.
pub fn grown_shells(g: &TreeGraph, shrub: bool, broad: bool, lift: f32) -> TriMesh {
    let mut mesh = TriMesh::default();
    if !shrub {
        wood(&mut mesh, g, WOOD_RADIUS);
    }
    let faces = icosahedron();
    let lumps = crown_lumps(g, shrub, broad, lift);
    // The crown's middle, as the cards' ([`grown_mesh`]).
    let middle = lumps.iter().fold(Vec3::ZERO, |s, l| s + l.0) / lumps.len().max(1) as f32;
    for (i, &(centre, radius)) in lumps.iter().enumerate() {
        let first = mesh.sections.len();
        let first_vertex = mesh.positions.len();
        lump_of(
            &mut mesh,
            centre,
            radius * CROWN_SHELL,
            i as u32 % 8,
            &faces,
        );
        mesh.sections[first..].fill(4);
        // Bent out from the crown's middle and the lump's, as the cards' normals ([`leaf_cards`]):
        // the crown lit as one rounded mass, as near and as its impostor far (D-075). Each lump
        // lit as a sphere of its own read flatter than both (the owner's choice, 2026-10-22).
        for v in first_vertex..mesh.positions.len() {
            let p = Vec3::from_array(mesh.positions[v]);
            let bent = (p - middle).normalize_or_zero() + (p - centre).normalize_or_zero();
            if let Some(bent) = bent.try_normalize() {
                mesh.normals[v] = bent.to_array();
            }
        }
    }
    mesh.uvs = vec![[0.0, 0.0]; mesh.positions.len()];
    mesh
}

/// A crown's shell at the ladder's middle ([`grown_shells`]) as a share of its lump's radius: the
/// cards reach from 0.4 to 0.7 radii out with their own half size beyond: the whole radius makes
/// the trees' crowns 0.99–1.07 of the cards' width, the cypress's column 0.83 and the shrubs'
/// 0.78–0.95 (to tune per species by the crossing's look).
pub const CROWN_SHELL: f32 = 1.0;

/// The thinnest wood the grown plants draw, metres of radius ([`wood`]).
pub const WOOD_RADIUS: f32 = 0.05;
/// The thinnest a tree's trunk is drawn, metres of radius ([`wood`]): a tree's leader tapers to
/// its species' tip, 1.2 cm or more, while a shrub's stems (order 0 as well) are millimetres
/// thick, sticks under a pixel by the thousand on a hillside (a review, 2026-10-24).
pub const TRUNK_RADIUS: f32 = 0.01;

/// The sides of a limb's tube.
const LIMB_SIDES: u32 = 7;

/// A tree's wood of `min_radius` or more (the grown plants' [`WOOD_RADIUS`]), section 0
/// ([`grown_mesh`]; public for a host's own stand-ins): each axis of its graph (a node
/// and the child that carries on its way, not a branch) one tapered tube of seven sides,
/// closed. The owner saw holes in the trunks, the most at the bends (2026-10-21): each segment was
/// a tube of its own, open, its ends square to it and its ring turned anyhow, so a bend opened a
/// wedge on its outside and the next ring's corners left slots into the open tube.
/// - Each ring is turned from the last by the bend between them, so a straight run's line up.
/// - At a bend of under 90° the two segments share one ring, in the plane halving the bend:
///   both tubes' sides end there exactly (the mitre).
/// - A tip, or a bend too sharp to mitre, is closed by a short cone.
/// - A branch starts on its parent's axis as wide as its parent there, buried in it.
pub fn wood(mesh: &mut TriMesh, g: &TreeGraph, min_radius: f32) {
    let n = g.nodes.len();
    let parent = |i: usize| g.nodes[i].parent as usize;
    // The trunk (the main stem, order 0) down to [`TRUNK_RADIUS`]: a young tree's tapered under
    // `min_radius` a little way up, and its crown, lifted over the passers-by, floated over a
    // stump (the owner, 2026-10-24: "floating canopy, no trunk").
    let drawn = |i: usize| {
        let node = &g.nodes[i];
        node.parent != forge_flora::graph::NO_PARENT
            && (node.radius >= min_radius || (node.order == 0 && node.radius >= TRUNK_RADIUS))
            && node.pos.distance(g.nodes[node.parent as usize].pos) > 1e-3
    };
    // Each drawn node's axis from its parent, and each node's way on: its drawn child that is
    // no branch (the thickest, should there be two).
    let mut axis = vec![Vec3::Y; n];
    let mut next = vec![usize::MAX; n];
    for i in (0..n).filter(|&i| drawn(i)) {
        let p = parent(i);
        axis[i] = (g.nodes[i].pos - g.nodes[p].pos).normalize();
        if !g.nodes[i].starts_branch
            && (next[p] == usize::MAX || g.nodes[i].radius > g.nodes[next[p]].radius)
        {
            next[p] = i;
        }
    }
    // Where a drawn node's tube carries on into its way on through a shared ring.
    let mitred = |j: usize| drawn(j) && next[j] != usize::MAX && axis[j].dot(axis[next[j]]) > 0.0;
    // Each drawn node's ring direction across its axis: carried from its parent's through the
    // bend where it carries on its parent's way, else any.
    let mut across = vec![Vec3::X; n];
    for i in (0..n).filter(|&i| drawn(i)) {
        let p = parent(i);
        across[i] = if drawn(p) && next[p] == i {
            let turned = glam::Quat::from_rotation_arc(axis[p], axis[i]) * across[p];
            (turned - axis[i] * turned.dot(axis[i])).normalize()
        } else {
            axis[i].any_orthonormal_vector()
        };
    }
    // A ring's corner `k` across axis `along`, counted from `u`: its unit direction from the axis.
    let corner = |u: Vec3, along: Vec3, k: u32| {
        let t = k as f32 / LIMB_SIDES as f32 * std::f32::consts::TAU;
        u * t.cos() + along.cross(u) * t.sin()
    };
    // Node `j`'s shared ring: each corner of its own tube's end moved along its axis onto the
    // plane halving the bend into its way on, its normal between the two tubes'.
    let mitre = |j: usize| -> Vec<(Vec3, Vec3)> {
        let (a, b) = (axis[j], axis[next[j]]);
        let half = (a + b).normalize();
        let turn = glam::Quat::from_rotation_arc(a, b);
        let r = g.nodes[j].radius;
        (0..LIMB_SIDES)
            .map(|k| {
                let e = corner(across[j], a, k);
                let s = -r * e.dot(half) / a.dot(half);
                (g.nodes[j].pos + e * r + a * s, (e + turn * e).normalize())
            })
            .collect()
    };
    // A plain ring round `centre`, square to node `i`'s axis.
    let ring = |i: usize, centre: Vec3, r: f32| -> Vec<(Vec3, Vec3)> {
        (0..LIMB_SIDES)
            .map(|k| {
                let e = corner(across[i], axis[i], k);
                (centre + e * r, e)
            })
            .collect()
    };
    for i in (0..n).filter(|&i| drawn(i)) {
        let p = parent(i);
        let start = if mitred(p) && next[p] == i {
            mitre(p)
        } else {
            let r = g.nodes[p].radius.max(g.nodes[i].radius);
            ring(i, g.nodes[p].pos, r)
        };
        let end = if mitred(i) {
            mitre(i)
        } else {
            ring(i, g.nodes[i].pos, g.nodes[i].radius)
        };
        for k in 0..LIMB_SIDES as usize {
            let l = (k + 1) % LIMB_SIDES as usize;
            let (p0, p1, q0, q1) = (start[k], start[l], end[k], end[l]);
            smooth_triangle(mesh, [p0.0, p1.0, q1.0], [p0.1, p1.1, q1.1], 0);
            smooth_triangle(mesh, [p0.0, q1.0, q0.0], [p0.1, q1.1, q0.1], 0);
        }
        if !mitred(i) {
            // Closed by a cone half its radius long.
            let tip = g.nodes[i].pos + axis[i] * g.nodes[i].radius * 0.5;
            for k in 0..LIMB_SIDES as usize {
                let l = (k + 1) % LIMB_SIDES as usize;
                let n0 = (end[k].1 + axis[i]).normalize();
                let n1 = (end[l].1 + axis[i]).normalize();
                smooth_triangle(mesh, [end[k].0, end[l].0, tip], [n0, n1, axis[i]], 0);
            }
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
    // The leaves gathered round seeds a cell apart, not into the cells of a grid: on the grid
    // the lumps stood in layers a cell apart, and every round crown showed the same horizontal
    // bands (the owner, 2026-10-24). Each seed the first leaf not yet gathered, in the graph's
    // order (the same every run); each leaf goes to the nearest seed within a cell, those
    // further out seeding their own.
    let reach = cell * 0.6;
    let mut grid: std::collections::HashMap<(i32, i32, i32), Vec<u32>> = Default::default();
    let key = |p: Vec3| {
        let k = (p / reach).floor();
        (k.x as i32, k.y as i32, k.z as i32)
    };
    let mut seeds: Vec<Vec3> = Vec::new();
    let mut sums: Vec<(Vec3, u32, f32)> = Vec::new();
    for l in &g.leaves {
        let (kx, ky, kz) = key(l.pos);
        let mut best: Option<(u32, f32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(kx + dx, ky + dy, kz + dz)) {
                        for &s in list {
                            let d = seeds[s as usize].distance_squared(l.pos);
                            if d <= reach * reach && best.is_none_or(|(_, b)| d < b) {
                                best = Some((s, d));
                            }
                        }
                    }
                }
            }
        }
        let s = match best {
            Some((s, _)) => s,
            None => {
                seeds.push(l.pos);
                sums.push((Vec3::ZERO, 0, 0.0));
                let s = (seeds.len() - 1) as u32;
                grid.entry((kx, ky, kz)).or_default().push(s);
                s
            }
        };
        let e = &mut sums[s as usize];
        e.0 += l.pos;
        e.1 += 1;
        e.2 = e.2.max(l.size);
    }
    let mean = g.leaves.len() as f32 / sums.len().max(1) as f32;
    sums.into_iter()
        .map(|(sum, n, size)| {
            let radius = cell * (0.45 + 0.35 * (n as f32 / mean).sqrt().min(1.6)) + size * 0.3;
            // Kept over a lifted crown's clear trunk: a lump round a low cluster hung below it.
            let mut centre = sum / n as f32;
            centre.y = centre.y.max(lift + radius * 0.7);
            (centre, radius)
        })
        .collect()
}

/// A crown lump's core, as a share of the lump's radius ([`grown_mesh`]): a fifth, drawn
/// darker, the shade between the cards (at 0.6 in the leaves' colour, then at half with 12 cards
/// at most a lump, the near holm oaks showed it as balls; at two fifths, once the metered
/// exposure lit the wood, as faceted green stones among the leaves; at three tenths, with the
/// leaves' detail (#346), as faceted olive balls through the gaps close up).
pub const CROWN_CORE: f32 = 0.2;

/// The sections of a plant's crown in [`grown_mesh`], its cores (1) and cards (2), which its cook
/// keeps whole (`CookOptions::keep_sections`): the shells and the impostor that take over far
/// off are made from its finest look, and the cluster LOD shrank the cores to nothing first (the
/// cards cannot simplify), so the far crowns thinned to specks before they filled out again.
pub const CROWN_SECTIONS: u32 = 1 << 1 | 1 << 2;

/// How far up (or down) a leaf card's normal may point, as its vertical component: past it the
/// card turns towards the side ([`leaf_cards`]), at least 37° off the horizontal.
const CARD_STEEP: f32 = 0.8;

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
        // No card lies flat: its normal at most [`CARD_STEEP`] up or down. A lump's top and
        // bottom cards faced the sky and the ground and, seen from the side, were edges: a
        // horizontal dash in every round crown, the same in all of them (the owner, 2026-10-24).
        let normal = if normal.y.abs() > CARD_STEEP {
            let level = Vec3::new(normal.x, 0.0, normal.z);
            let level = if level.length_squared() > 1e-6 {
                level.normalize()
            } else {
                let a = spin + golden * k as f32;
                Vec3::new(a.cos(), 0.0, a.sin())
            };
            level * (1.0 - CARD_STEEP * CARD_STEEP).sqrt() + Vec3::Y * CARD_STEEP.copysign(normal.y)
        } else {
            normal
        };
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

    /// A tree cooked with its crown kept whole ([`CROWN_SECTIONS`]) holds every core and card
    /// triangle in its coarsest cut, where the default cook shrinks the cores (the owner,
    /// 2026-10-25: far crowns thinned to specks); its wood still simplifies.
    #[test]
    fn a_kept_crown_reaches_the_coarsest_cut_whole() {
        use crate::meshlet::{CookOptions, MeshletMesh, triangle_section};
        let species = forge_flora::species::mediterranean();
        let (sp, s) = species
            .iter()
            .enumerate()
            .find(|(_, s)| s.habit != Habit::Shrub)
            .expect("a tree");
        let plant = grow_plant(s, &forge_flora::grow::individual(s, sp as u16, 0));
        // Each section's triangles at level 0 and in the roots (the cut drawn from afar).
        let counts = |options: CookOptions| {
            let built = MeshletMesh::build_with(&plant.mesh, options);
            let count = |root: bool, section: u32| -> usize {
                built
                    .meshlets
                    .iter()
                    .filter(|m| {
                        if root {
                            m.parent_error.is_infinite()
                        } else {
                            m.lod_level == 0
                        }
                    })
                    .map(|m| {
                        (0..m.triangle_count)
                            .filter(|&t| triangle_section(m.section, t) == section)
                            .count()
                    })
                    .sum()
            };
            [0, 1, 2].map(|section| (count(false, section), count(true, section)))
        };
        let kept = counts(CookOptions {
            keep_sections: CROWN_SECTIONS,
            ..CookOptions::default()
        });
        let plain = counts(CookOptions::default());
        assert_eq!(kept[1].0, kept[1].1, "the cores, kept");
        assert_eq!(kept[2].0, kept[2].1, "the cards, kept");
        assert!(kept[0].1 < kept[0].0, "the wood simplifies: {:?}", kept[0]);
        assert!(
            plain[1].1 < plain[1].0,
            "the cores shrink uncut: {:?}",
            plain[1]
        );
    }

    /// The ladder's middle (D-075, #302): the shells hold the crown's outline where the cards
    /// hand over (their crown's width 0.75 to 1.2 of the cards'), in fewer triangles than the
    /// drawn plant, the wood and the crown's sections only.
    #[test]
    fn the_crown_shells_keep_the_crown_s_outline() {
        let width = |mesh: &TriMesh, section: u8| {
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for (t, &s) in mesh.sections.iter().enumerate() {
                if s != section {
                    continue;
                }
                for &i in &mesh.indices[3 * t..3 * t + 3] {
                    let p = Vec3::from(mesh.positions[i as usize]);
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
            }
            (hi.x - lo.x).max(hi.z - lo.z)
        };
        for (sp, s) in forge_flora::species::mediterranean().iter().enumerate() {
            let (shrub, broad) = (s.habit == Habit::Shrub, s.foliage == Foliage::Broad);
            let conditions = forge_flora::grow::individual(s, sp as u16, 0);
            let graph = forge_flora::grow(s, &conditions);
            let lift = conditions.pruning.lift as f32;
            let drawn = grown_mesh(&graph, shrub, broad, lift, s.look.spray);
            let shells = grown_shells(&graph, shrub, broad, lift);
            assert!(shells.sections.iter().all(|&s| s == 0 || s == 4));
            assert!(
                shells.triangle_count() < drawn.triangle_count(),
                "{}: {} triangles against {}",
                s.id,
                shells.triangle_count(),
                drawn.triangle_count()
            );
            let (cards, crown) = (width(&drawn, 2), width(&shells, 4));
            assert!(
                (0.75..1.2).contains(&(crown / cards)),
                "{}: the shells {crown:.2} m across, the cards {cards:.2} m",
                s.id
            );
            // Their normals bend out from the crown's middle, as the cards' do: a lump's inner
            // side, facing the middle, lit as the crown's outside it belongs to.
            let shell: Vec<usize> = shells
                .indices
                .chunks(3)
                .zip(&shells.sections)
                .filter(|(_, section)| **section == 4)
                .flat_map(|(t, _)| t.iter().map(|&i| i as usize))
                .collect();
            let middle = shell
                .iter()
                .fold(Vec3::ZERO, |m, &i| m + Vec3::from(shells.positions[i]))
                / shell.len() as f32;
            let outward = shell
                .iter()
                .filter(|&&i| {
                    let n = Vec3::from(shells.normals[i]);
                    (n.length() - 1.0).abs() < 1e-3
                        && n.dot(Vec3::from(shells.positions[i]) - middle) > 0.0
                })
                .count();
            assert!(
                outward as f32 > 0.97 * shell.len() as f32,
                "{}: {outward} of {} shell normals face out of the crown",
                s.id,
                shell.len()
            );
        }
    }

    /// A young tree's trunk reaches its crown (the owner, 2026-10-24: "floating canopy, no
    /// trunk"), and a shrub's millimetre stems are not drawn (sticks under a pixel by the
    /// thousand).
    #[test]
    fn young_trunks_reach_their_crowns_and_shrub_stems_stay_undrawn() {
        let all = forge_flora::species::mediterranean();
        for s in &all {
            let young = forge_flora::grow(s, &forge_flora::Conditions::new(s.maturity * 0.25, 1));
            let mut mesh = TriMesh::default();
            wood(&mut mesh, &young, WOOD_RADIUS);
            if s.habit == Habit::Shrub {
                let thin = young.nodes.iter().all(|n| n.radius < TRUNK_RADIUS);
                assert!(
                    !thin || mesh.positions.is_empty(),
                    "{}: {} wood vertices from stems under a centimetre",
                    s.id,
                    mesh.positions.len()
                );
                continue;
            }
            let top = mesh.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
            let lowest = young
                .leaves
                .iter()
                .map(|l| l.pos.y)
                .fold(f32::MAX, f32::min);
            assert!(
                top >= lowest,
                "{}: the wood stops at {top:.2} m, the lowest leaves at {lowest:.2}",
                s.id
            );
        }
    }

    /// The trees' wood is closed (the owner: holes in the trunks, the most at the bends): its
    /// corners welded within 0.1 mm, each edge is two triangles' but the rings where an axis
    /// starts (a branch's, buried in its parent; a stem's, at the ground).
    #[test]
    fn the_trees_wood_has_no_holes() {
        for (sp, s) in forge_flora::species::mediterranean().iter().enumerate() {
            if s.habit == Habit::Shrub {
                continue;
            }
            for k in 0..forge_flora::grow::INDIVIDUALS {
                let graph = forge_flora::grow(s, &forge_flora::grow::individual(s, sp as u16, k));
                let mut mesh = TriMesh::default();
                wood(&mut mesh, &graph, WOOD_RADIUS);
                let key = |i: u32| {
                    (Vec3::from(mesh.positions[i as usize]) * 1e4)
                        .round()
                        .as_ivec3()
                };
                let mut edges = std::collections::HashMap::new();
                for t in mesh.indices.chunks(3) {
                    for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                        let (a, b) = (key(a).to_array(), key(b).to_array());
                        *edges
                            .entry(if a < b { (a, b) } else { (b, a) })
                            .or_insert(0) += 1;
                    }
                }
                let open = edges.values().filter(|&&c| c == 1).count();
                // The axes' starts: each drawn node but the way on of a drawn parent (its
                // thickest drawn child that is no branch) bending under 90° into it.
                let nodes = &graph.nodes;
                let drawn = |i: usize| {
                    nodes[i].parent != forge_flora::graph::NO_PARENT
                        && (nodes[i].radius >= WOOD_RADIUS
                            || (nodes[i].order == 0 && nodes[i].radius >= TRUNK_RADIUS))
                        && nodes[i].pos.distance(nodes[nodes[i].parent as usize].pos) > 1e-3
                };
                let along =
                    |i: usize| (nodes[i].pos - nodes[nodes[i].parent as usize].pos).normalize();
                let mut way_on = vec![usize::MAX; nodes.len()];
                for i in (0..nodes.len()).filter(|&i| drawn(i) && !nodes[i].starts_branch) {
                    let p = nodes[i].parent as usize;
                    if way_on[p] == usize::MAX || nodes[i].radius > nodes[way_on[p]].radius {
                        way_on[p] = i;
                    }
                }
                let starts = (0..nodes.len())
                    .filter(|&i| drawn(i))
                    .filter(|&i| {
                        let p = nodes[i].parent as usize;
                        !(drawn(p) && way_on[p] == i && along(p).dot(along(i)) > 0.0)
                    })
                    .count();
                assert!(
                    open <= starts * LIMB_SIDES as usize,
                    "{} {k}: {open} open edges, {starts} axes' starts",
                    s.id
                );
            }
        }
    }
}
