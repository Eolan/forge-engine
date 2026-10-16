//! The first style set's kit (D-039, D-059): the modules a split grammar's terminals name
//! (`forge_procgen::grammar`), built as small part programs of boxes and quads. A module fills
//! its scope on the wall plane: `x` from 0 to its width, `y` from 0 to its height, the wall at
//! `z = 0` and `+z` out of it. Openings go into the wall (reveals, frames and glass behind it);
//! sills, surrounds, shutters, balconies and awnings stand out of it, past the scope's edges
//! where they must.
//!
//! Every module uses one section layout ([`PLASTER`] to [`PARTY`]), so one material group of
//! [`SECTIONS`] rows dresses any of them: a group per shutter colour and per lit or dark glass.
//! The walls and bands change size with every scope and are merged into the buildings' meshes;
//! the windows, balconies, doors and shops repeat at a few sizes and are instanced
//! ([`shared`]).

use glam::Vec3;

use crate::procedural::TriMesh;

/// The rendered wall.
pub const PLASTER: u8 = 0;
/// Stone: sills, surrounds, reveals, bands, cornices, balcony slabs.
pub const STONE: u8 = 1;
/// Window and shop frames.
pub const FRAME: u8 = 2;
/// Glass.
pub const GLASS: u8 = 3;
/// Painted wood: shutters and shop fascias.
pub const SHUTTER: u8 = 4;
/// Wrought iron: railings.
pub const METAL: u8 = 5;
/// Awnings.
pub const FABRIC: u8 = 6;
/// Doors.
pub const WOOD: u8 = 7;
/// A party wall's bare render.
pub const PARTY: u8 = 8;
/// A railing's bars, drawn on one card whose `u` counts them (a grating row,
/// `forge_core::material::RenderLayer::grating`, of [`BAR_SHARE`]).
pub const GRILLE: u8 = 9;
/// A shopfront's glass, a shop behind it.
pub const SHOP_GLASS: u8 = 10;
/// The number of sections.
pub const SECTIONS: u8 = 11;

/// A railing's square bars: their width, a share of their spacing (3 cm every 12 cm).
pub const BAR_SHARE: f32 = 0.25;

/// The modules the kit builds.
pub const NAMES: [&str; 16] = [
    "wall",
    "blank",
    "band",
    "cornice",
    "parapet",
    "window",
    "window_plain",
    "window_closed",
    "window_low",
    "window_court",
    "bay",
    "balcony",
    "door",
    "door_open",
    "shop",
    "shop_awning",
];

/// Whether a module repeats at a few sizes and is better instanced than merged.
pub fn shared(name: &str) -> bool {
    !matches!(name, "wall" | "blank" | "band" | "cornice" | "parapet")
}

/// A module `w` by `h` metres, or `None` for a name the kit does not have; its openings' reveals
/// 0.22 m deep (0.18 for a courtyard window).
pub fn module(name: &str, w: f32, h: f32) -> Option<TriMesh> {
    let reveal = if name == "window_court" { 0.18 } else { 0.22 };
    module_deep(name, w, h, reveal)
}

/// A module `w` by `h` metres, its opening's reveal `reveal` metres deep: the window set back
/// into a wall of its thickness (D-060: old stone's deep, modern walls' shallow).
pub fn module_deep(name: &str, w: f32, h: f32, reveal: f32) -> Option<TriMesh> {
    module_with(name, w, h, reveal, false)
}

/// [`module_deep`], its railings' bars as geometry or, with `cards`, one card each (a grating,
/// [`GRILLE`]; the mesh then has UVs). The cards cannot flicker but draw through the cut-out
/// raster, which costs more than they save for now (`research/trim-sheets.md` §2.9).
pub fn module_with(name: &str, w: f32, h: f32, reveal: f32, cards: bool) -> Option<TriMesh> {
    let mut m = TriMesh::default();
    let railing = |m: &mut TriMesh, a: Vec3, b: Vec3, height: f32| {
        if cards {
            railing_card(m, a, b, height);
        } else {
            railing_bars(m, a, b, height);
        }
    };
    match name {
        "wall" => quad(&mut m, wall_quad(w, h, 0.0), PLASTER),
        "blank" => quad(&mut m, wall_quad(w, h, 0.0), PARTY),
        "band" => cuboid(&mut m, Vec3::ZERO, Vec3::new(w, h, 0.08), STONE, NO_BACK),
        "cornice" => {
            // Three steps out, the top one furthest.
            let steps = [(0.0, 0.3, 0.06), (0.3, 0.65, 0.18), (0.65, 1.0, 0.3)];
            for (y0, y1, out) in steps {
                cuboid(
                    &mut m,
                    Vec3::new(0.0, h * y0, 0.0),
                    Vec3::new(w, h * y1, out),
                    STONE,
                    ALL,
                );
            }
        }
        "parapet" => {
            quad(&mut m, wall_quad(w, h, 0.0), PLASTER);
            // Its back, seen from the roof, and the coping on top.
            let back = [
                Vec3::new(w, 0.0, -0.25),
                Vec3::new(0.0, 0.0, -0.25),
                Vec3::new(0.0, h, -0.25),
                Vec3::new(w, h, -0.25),
            ];
            quad(&mut m, back, PLASTER);
            cuboid(
                &mut m,
                Vec3::new(0.0, h - 0.06, -0.3),
                Vec3::new(w, h + 0.03, 0.05),
                STONE,
                ALL,
            );
        }
        "window" | "window_plain" => {
            window(&mut m, w, h, reveal, true);
            surround(&mut m, w, h, 0.15, 0.22);
            sill(&mut m, w, reveal, 0.07);
            railing(
                &mut m,
                Vec3::new(0.0, 0.0, GUARD_Z),
                Vec3::new(w, 0.0, GUARD_Z),
                1.0,
            );
            if name == "window" {
                shutters(&mut m, w, h);
            }
        }
        "window_closed" => {
            window(&mut m, w, h, reveal, true);
            surround(&mut m, w, h, 0.15, 0.22);
            sill(&mut m, w, reveal, 0.07);
            closed_shutters(&mut m, w, h);
            railing(
                &mut m,
                Vec3::new(0.0, 0.0, GUARD_Z),
                Vec3::new(w, 0.0, GUARD_Z),
                1.0,
            );
        }
        "window_low" => {
            window(&mut m, w, h, reveal, true);
            surround(&mut m, w, h, 0.15, 0.22);
            sill(&mut m, w, reveal, 0.08);
        }
        "window_court" => {
            window(&mut m, w, h, reveal, false);
            sill(&mut m, w, reveal, 0.05);
        }
        // A glazed sliding bay to the floor, onto a terrace: two leaves, no sill, its threshold
        // flush with the floor.
        "bay" => window(&mut m, w, h, reveal, true),
        "balcony" => {
            window(&mut m, w, h, reveal, true);
            surround(&mut m, w, h, 0.15, 0.22);
            shutters(&mut m, w, h);
            balcony(&mut m, w, railing);
        }
        "door" | "door_open" => door(&mut m, w, h, name == "door_open"),
        "shop" | "shop_awning" => shop(&mut m, w, h, name == "shop_awning"),
        _ => return None,
    }
    // The parts after a railing's card get UVs too.
    if !m.uvs.is_empty() {
        m.uvs.resize(m.positions.len(), [0.0, 0.0]);
    }
    Some(m)
}

/// Faces of a cuboid, as bits.
pub(crate) const POS_X: u8 = 1;
pub(crate) const NEG_X: u8 = 2;
pub(crate) const POS_Y: u8 = 4;
pub(crate) const NEG_Y: u8 = 8;
pub(crate) const POS_Z: u8 = 16;
pub(crate) const NEG_Z: u8 = 32;
pub(crate) const ALL: u8 = 63;
pub(crate) const NO_BACK: u8 = ALL & !NEG_Z;

/// A quad, its corners counter-clockwise seen from its front.
pub(crate) fn quad(m: &mut TriMesh, p: [Vec3; 4], section: u8) {
    let n = (p[2] - p[0]).cross(p[3] - p[1]).normalize_or_zero();
    let base = m.positions.len() as u32;
    for q in p {
        m.positions.push(q.to_array());
        m.normals.push(n.to_array());
    }
    m.indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    m.sections.extend([section, section]);
}

/// How far behind a thin sheet's front its back face stands, metres. Two faces on one plane fold
/// into each other in the coarser levels of detail: a market stall's canvas lost half of itself
/// from the air, the shops' awnings all of themselves while their shadows stayed, and a moving
/// tram's roof lights flickered (the owner, 2026-10-15).
pub(crate) const SHEET: f32 = 0.02;

/// A thin sheet seen from both sides: [`quad`] `p`, and its back [`SHEET`] behind it.
pub(crate) fn sheet(m: &mut TriMesh, p: [Vec3; 4], section: u8) {
    quad(m, p, section);
    let n = (p[2] - p[0]).cross(p[3] - p[1]).normalize_or_zero();
    let b = p.map(|q| q - n * SHEET);
    quad(m, [b[1], b[0], b[3], b[2]], section);
}

/// A thin triangle seen from both sides: [`triangle`] `p`, and its back [`SHEET`] behind it.
pub(crate) fn sheet_triangle(m: &mut TriMesh, p: [Vec3; 3], section: u8) {
    triangle(m, p, section);
    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
    let b = p.map(|q| q - n * SHEET);
    triangle(m, [b[0], b[2], b[1]], section);
}

/// A triangle, its corners counter-clockwise seen from its front.
pub(crate) fn triangle(m: &mut TriMesh, p: [Vec3; 3], section: u8) {
    let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
    let base = m.positions.len() as u32;
    for q in p {
        m.positions.push(q.to_array());
        m.normals.push(n.to_array());
    }
    m.indices.extend([base, base + 1, base + 2]);
    m.sections.push(section);
}

/// The wall's quad at depth `z`.
fn wall_quad(w: f32, h: f32, z: f32) -> [Vec3; 4] {
    [
        Vec3::new(0.0, 0.0, z),
        Vec3::new(w, 0.0, z),
        Vec3::new(w, h, z),
        Vec3::new(0.0, h, z),
    ]
}

/// The faces `faces` of the box from `lo` to `hi`.
pub(crate) fn cuboid(m: &mut TriMesh, lo: Vec3, hi: Vec3, section: u8, faces: u8) {
    let (a, b) = (lo, hi);
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    let all = [
        (
            POS_X,
            [
                v(b.x, a.y, b.z),
                v(b.x, a.y, a.z),
                v(b.x, b.y, a.z),
                v(b.x, b.y, b.z),
            ],
        ),
        (
            NEG_X,
            [
                v(a.x, a.y, a.z),
                v(a.x, a.y, b.z),
                v(a.x, b.y, b.z),
                v(a.x, b.y, a.z),
            ],
        ),
        (
            POS_Y,
            [
                v(a.x, b.y, b.z),
                v(b.x, b.y, b.z),
                v(b.x, b.y, a.z),
                v(a.x, b.y, a.z),
            ],
        ),
        (
            NEG_Y,
            [
                v(a.x, a.y, a.z),
                v(b.x, a.y, a.z),
                v(b.x, a.y, b.z),
                v(a.x, a.y, b.z),
            ],
        ),
        (
            POS_Z,
            [
                v(a.x, a.y, b.z),
                v(b.x, a.y, b.z),
                v(b.x, b.y, b.z),
                v(a.x, b.y, b.z),
            ],
        ),
        (
            NEG_Z,
            [
                v(b.x, a.y, a.z),
                v(a.x, a.y, a.z),
                v(a.x, b.y, a.z),
                v(b.x, b.y, a.z),
            ],
        ),
    ];
    for (bit, p) in all {
        if faces & bit != 0 {
            quad(m, p, section);
        }
    }
}

/// An opening `depth` deep: its reveals, the glass behind and a frame round the glass with a
/// mullion (two leaves) and a transom light over tall openings.
fn window(m: &mut TriMesh, w: f32, h: f32, depth: f32, mullion: bool) {
    let d = -depth;
    // The reveals, looking into the opening.
    quad(
        m,
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, d),
            Vec3::new(0.0, h, d),
            Vec3::new(0.0, h, 0.0),
        ],
        STONE,
    );
    quad(
        m,
        [
            Vec3::new(w, 0.0, d),
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(w, h, 0.0),
            Vec3::new(w, h, d),
        ],
        STONE,
    );
    quad(
        m,
        [
            Vec3::new(0.0, h, d),
            Vec3::new(w, h, d),
            Vec3::new(w, h, 0.0),
            Vec3::new(0.0, h, 0.0),
        ],
        STONE,
    );
    quad(
        m,
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(w, 0.0, d),
            Vec3::new(0.0, 0.0, d),
        ],
        STONE,
    );
    let glass = d + 0.05;
    quad(m, wall_quad(w, h, glass), GLASS);
    // The frame, standing a little in front of the glass.
    let (z0, z1) = (glass - 0.01, glass + 0.06);
    let f = 0.065;
    let bar = |m: &mut TriMesh, x0: f32, y0: f32, x1: f32, y1: f32| {
        cuboid(
            m,
            Vec3::new(x0, y0, z0),
            Vec3::new(x1, y1, z1),
            FRAME,
            NO_BACK,
        );
    };
    bar(m, 0.0, 0.0, f, h);
    bar(m, w - f, 0.0, w, h);
    bar(m, f, h - f, w - f, h);
    bar(m, f, 0.0, w - f, f);
    let transom = (h > 2.0).then_some(h - 0.5);
    if let Some(t) = transom {
        bar(m, f, t - f * 0.5, w - f, t + f * 0.5);
    }
    if mullion {
        let top = transom.map_or(h - f, |t| t - f * 0.5);
        bar(m, w * 0.5 - f * 0.5, f, w * 0.5 + f * 0.5, top);
    }
}

/// A stone surround `band` wide round an opening, a little proud of the wall, with a deeper
/// lintel `lintel` tall.
fn surround(m: &mut TriMesh, w: f32, h: f32, band: f32, lintel: f32) {
    let proud = 0.025;
    cuboid(
        m,
        Vec3::new(-band, 0.0, 0.0),
        Vec3::new(0.0, h, proud),
        STONE,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(w, 0.0, 0.0),
        Vec3::new(w + band, h, proud),
        STONE,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(-band, h, 0.0),
        Vec3::new(w + band, h + lintel, proud * 1.6),
        STONE,
        NO_BACK,
    );
}

/// A stone sill under an opening `depth` deep, standing `out` metres out of the wall.
fn sill(m: &mut TriMesh, w: f32, depth: f32, out: f32) {
    cuboid(
        m,
        Vec3::new(-0.08, -0.07, -depth),
        Vec3::new(w + 0.08, 0.0, out),
        STONE,
        NO_BACK,
    );
}

/// A French window's guard rail, in the reveal between the closed shutters and the frame: on
/// the wall's face it stood where a leaf swings shut (the owner, 2026-10-16).
const GUARD_Z: f32 = -0.115;

/// Where an open shutter's back stands, metres out of the wall: 2 cm off the surround's face, so
/// no two faces share a plane.
const OPEN_Z: f32 = 0.045;

/// Two louvred shutters open: each leaf half the opening's width, hung on the opening's outer
/// edge and folded back flat against the facade over the surround, so it swings shut over the
/// opening from outside (the owner, 2026-10-16). Each hangs by two iron straps across its face
/// on pins sealed in the masonry at the opening's edge (`pentures` on `gonds`), a sixth of its
/// height from its top and its bottom.
fn shutters(m: &mut TriMesh, w: f32, h: f32) {
    let leaf = w * 0.5;
    // Each leaf's span and its hinge edge.
    for (x0, x1, hinge) in [(-leaf, 0.0, 0.0), (w, w + leaf, w)] {
        shutter_leaf(m, x0, x1, h, OPEN_Z);
        let front = OPEN_Z + 0.04;
        let reach = (x1 - x0) * 0.7;
        let (s0, s1) = if hinge == x1 {
            (x1 - reach, x1)
        } else {
            (x0, x0 + reach)
        };
        for y in [h / 6.0, h * 5.0 / 6.0] {
            // The strap on the leaf's face.
            cuboid(
                m,
                Vec3::new(s0, y - 0.02, front),
                Vec3::new(s1, y + 0.02, front + 0.006),
                METAL,
                NO_BACK,
            );
            // The pin it turns on, from the surround to the leaf, just beside the opening.
            let (p0, p1) = if hinge == x1 {
                (hinge - 0.04, hinge - 0.01)
            } else {
                (hinge + 0.01, hinge + 0.04)
            };
            cuboid(
                m,
                Vec3::new(p0, y - 0.025, 0.025),
                Vec3::new(p1, y + 0.025, OPEN_Z),
                METAL,
                NO_BACK,
            );
        }
    }
}

/// Two louvred shutters closed over an opening, in its reveal just behind the wall's face.
fn closed_shutters(m: &mut TriMesh, w: f32, h: f32) {
    let leaf = w * 0.5;
    for x0 in [0.0, leaf] {
        shutter_leaf(m, x0, x0 + leaf, h, -0.08);
    }
}

/// One louvred shutter from `x0` to `x1`, `h` tall, its back at depth `z0`: a frame of stiles
/// and rails round sloping slats.
fn shutter_leaf(m: &mut TriMesh, x0: f32, x1: f32, h: f32, z0: f32) {
    let (zb, zf) = (z0 + 0.01, z0 + 0.035);
    // The leaf's frame: stiles and rails.
    let (stile, rail) = (0.05, 0.08);
    cuboid(
        m,
        Vec3::new(x0, 0.0, z0),
        Vec3::new(x0 + stile, h, zf + 0.005),
        SHUTTER,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(x1 - stile, 0.0, z0),
        Vec3::new(x1, h, zf + 0.005),
        SHUTTER,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(x0 + stile, 0.0, z0),
        Vec3::new(x1 - stile, rail, zf + 0.005),
        SHUTTER,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(x0 + stile, h - rail, z0),
        Vec3::new(x1 - stile, h, zf + 0.005),
        SHUTTER,
        NO_BACK,
    );
    // The slats: each slopes back from its lower edge, its underside shows below it.
    let (xa, xb) = (x0 + stile, x1 - stile);
    let pitch = 0.07;
    let slats = ((h - 2.0 * rail) / pitch).floor() as u32;
    let start = rail + ((h - 2.0 * rail) - slats as f32 * pitch) * 0.5;
    for k in 0..slats {
        let y = start + k as f32 * pitch;
        quad(
            m,
            [
                Vec3::new(xa, y, zf),
                Vec3::new(xb, y, zf),
                Vec3::new(xb, y + pitch, zb),
                Vec3::new(xa, y + pitch, zb),
            ],
            SHUTTER,
        );
        quad(
            m,
            [
                Vec3::new(xa, y, zb),
                Vec3::new(xb, y, zb),
                Vec3::new(xb, y, zf),
                Vec3::new(xa, y, zf),
            ],
            SHUTTER,
        );
    }
}

/// A railing's run from `lo` to `hi` along x or along z, its bars `step` apart, the first one
/// `step` past `lo` and none at the ends.
struct Run {
    along_x: bool,
    lo: Vec3,
    length: f32,
    bars: f32,
    step: f32,
}

impl Run {
    /// The point `t` metres along the run.
    fn at(&self, t: f32) -> Vec3 {
        if self.along_x {
            Vec3::new(self.lo.x + t, 0.0, self.lo.z)
        } else {
            Vec3::new(self.lo.x, 0.0, self.lo.z + t)
        }
    }
}

/// A wrought-iron railing's rails `height` tall from `a` to `b`, a run along x or along z, top
/// and bottom; its bars 3 cm square every 12 cm or a little more.
fn rails(m: &mut TriMesh, a: Vec3, b: Vec3, height: f32) -> Run {
    let along_x = (b.x - a.x).abs() >= (b.z - a.z).abs();
    let (lo, hi) = (a.min(b), a.max(b));
    let r = 0.02;
    let rail = |m: &mut TriMesh, y0: f32, y1: f32| {
        let (l, h) = if along_x {
            (Vec3::new(lo.x, y0, lo.z - r), Vec3::new(hi.x, y1, hi.z + r))
        } else {
            (Vec3::new(lo.x - r, y0, lo.z), Vec3::new(hi.x + r, y1, hi.z))
        };
        cuboid(m, l, h, METAL, ALL);
    };
    rail(m, a.y + height - 0.04, a.y + height);
    rail(m, a.y + 0.08, a.y + 0.11);
    let length = if along_x { hi.x - lo.x } else { hi.z - lo.z };
    let bars = (length / 0.12).floor().max(2.0);
    Run {
        along_x,
        lo,
        length,
        bars,
        step: length / bars,
    }
}

/// A wrought-iron railing `height` tall from `a` to `b`: [`rails`], and square bars between.
fn railing_bars(m: &mut TriMesh, a: Vec3, b: Vec3, height: f32) {
    let run = rails(m, a, b, height);
    let s = 0.015;
    let faces = POS_X | NEG_X | POS_Z | NEG_Z;
    for k in 1..run.bars as u32 {
        let c = run.at(k as f32 * run.step);
        cuboid(
            m,
            Vec3::new(c.x - s, a.y + 0.11, c.z - s),
            Vec3::new(c.x + s, a.y + height - 0.04, c.z + s),
            METAL,
            faces,
        );
    }
}

/// A wrought-iron railing `height` tall from `a` to `b`: [`rails`], and its bars on one card (a
/// grating, [`GRILLE`]), from half a spacing past the first end to half a spacing short of the
/// other. As geometry the bars are under a pixel past about 18 m and flicker as the view moves
/// (D-013, trim sheets §2.9).
fn railing_card(m: &mut TriMesh, a: Vec3, b: Vec3, height: f32) {
    let run = rails(m, a, b, height);
    let (along_x, bars) = (run.along_x, run.bars);
    let (y0, y1) = (a.y + 0.11, a.y + height - 0.04);
    let (p0, p1) = (run.at(run.step * 0.5), run.at(run.length - run.step * 0.5));
    let normal = if along_x { Vec3::Z } else { Vec3::X };
    // Every vertex before the card gets UVs too: a mesh has them for all its vertices or none.
    m.uvs.resize(m.positions.len(), [0.0, 0.0]);
    let base = m.positions.len() as u32;
    for (p, u, y) in [
        (p0, 0.5, y0),
        (p1, bars - 0.5, y0),
        (p1, bars - 0.5, y1),
        (p0, 0.5, y1),
    ] {
        m.positions.push([p.x, y, p.z]);
        m.normals.push(normal.to_array());
        m.uvs.push([u, y]);
    }
    // Counter-clockwise seen from +z along x; from +x along z the order turns.
    if along_x {
        m.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    } else {
        m.indices
            .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
    }
    m.sections.extend([GRILLE, GRILLE]);
}

/// A stone balcony on two consoles before an opening `w` wide, railed on three sides by
/// `railing`. It spans the opening and both its open shutters with 0.1 m to spare, and stands
/// out further than a leaf is wide, so a leaf swings shut inside its railing (the owner,
/// 2026-10-16: a balcony narrower than its shutters kept them from closing).
fn balcony(m: &mut TriMesh, w: f32, railing: impl Fn(&mut TriMesh, Vec3, Vec3, f32)) {
    let leaf = w * 0.5;
    let (x0, x1, deep) = (-leaf - 0.1, w + leaf + 0.1, (leaf + 0.2).max(0.8));
    cuboid(
        m,
        Vec3::new(x0, -0.14, 0.0),
        Vec3::new(x1, 0.0, deep),
        STONE,
        NO_BACK,
    );
    for x in [x0 + 0.2, x1 - 0.2] {
        cuboid(
            m,
            Vec3::new(x - 0.07, -0.45, 0.0),
            Vec3::new(x + 0.07, -0.14, deep * 0.55),
            STONE,
            NO_BACK,
        );
    }
    let (rx0, rx1, rz) = (x0 + 0.03, x1 - 0.03, deep - 0.04);
    railing(m, Vec3::new(rx0, 0.0, rz), Vec3::new(rx1, 0.0, rz), 1.0);
    railing(m, Vec3::new(rx0, 0.0, 0.02), Vec3::new(rx0, 0.0, rz), 1.0);
    railing(m, Vec3::new(rx1, 0.0, 0.02), Vec3::new(rx1, 0.0, rz), 1.0);
}

/// A building's entrance: a deep stone reveal, two panelled doors under a glazed transom, a
/// surround with a hood over it and a threshold. `open`: its leaves swung in against the reveal
/// (a house whose insides are drawn, entered from the street).
fn door(m: &mut TriMesh, w: f32, h: f32, open: bool) {
    let d = -0.35;
    let transom = h - 0.7;
    // Reveals, as an opening's, without its glass and frame.
    quad(
        m,
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, d),
            Vec3::new(0.0, h, d),
            Vec3::new(0.0, h, 0.0),
        ],
        STONE,
    );
    quad(
        m,
        [
            Vec3::new(w, 0.0, d),
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(w, h, 0.0),
            Vec3::new(w, h, d),
        ],
        STONE,
    );
    quad(
        m,
        [
            Vec3::new(0.0, h, d),
            Vec3::new(w, h, d),
            Vec3::new(w, h, 0.0),
            Vec3::new(0.0, h, 0.0),
        ],
        STONE,
    );
    // The leaves, their raised panels, and the transom light.
    let z = d + 0.05;
    let leaf = w * 0.5;
    if open {
        // Swung in against the reveal's inner ends, square to the wall, their panels facing
        // the way through.
        for (x0, x1, face) in [(0.0, 0.05, 0.05), (w - 0.05, w, -0.025)] {
            cuboid(
                m,
                Vec3::new(x0, 0.0, d - leaf),
                Vec3::new(x1, transom, d),
                WOOD,
                ALL,
            );
            let px = if face > 0.0 { x1 } else { x0 + face };
            for (y0, y1) in [(0.25, transom * 0.45), (transom * 0.5, transom - 0.2)] {
                cuboid(
                    m,
                    Vec3::new(px, y0, d - leaf + 0.12),
                    Vec3::new(px + 0.025, y1, d - 0.12),
                    WOOD,
                    ALL,
                );
            }
        }
    } else {
        cuboid(
            m,
            Vec3::new(0.0, 0.0, z - 0.05),
            Vec3::new(w, transom, z),
            WOOD,
            POS_Z,
        );
        for k in 0..2 {
            let x = k as f32 * leaf;
            for (y0, y1) in [(0.25, transom * 0.45), (transom * 0.5, transom - 0.2)] {
                cuboid(
                    m,
                    Vec3::new(x + 0.12, y0, z),
                    Vec3::new(x + leaf - 0.12, y1, z + 0.025),
                    WOOD,
                    NO_BACK,
                );
            }
        }
    }
    cuboid(
        m,
        Vec3::new(0.0, transom, z - 0.02),
        Vec3::new(w, transom + 0.08, z + 0.03),
        WOOD,
        NO_BACK,
    );
    let glass = [
        Vec3::new(0.0, transom + 0.08, z - 0.03),
        Vec3::new(w, transom + 0.08, z - 0.03),
        Vec3::new(w, h, z - 0.03),
        Vec3::new(0.0, h, z - 0.03),
    ];
    quad(m, glass, GLASS);
    for k in 1..4 {
        let x = w * k as f32 / 4.0;
        cuboid(
            m,
            Vec3::new(x - 0.02, transom + 0.08, z - 0.03),
            Vec3::new(x + 0.02, h, z),
            FRAME,
            NO_BACK,
        );
    }
    // Surround, hood and threshold: a stone 2 cm proud, level with the floor inside (the owner:
    // no step where people walk; it was a 12 cm step).
    surround(m, w, h, 0.25, 0.3);
    cuboid(
        m,
        Vec3::new(-0.4, h + 0.3, 0.0),
        Vec3::new(w + 0.4, h + 0.42, 0.22),
        STONE,
        NO_BACK,
    );
    cuboid(
        m,
        Vec3::new(-0.1, 0.0, d),
        Vec3::new(w + 0.1, 0.02, 0.3),
        STONE,
        NO_BACK,
    );
}

/// A shopfront: a deep opening the scope's size, glazed over a stone stall riser, mullions; a
/// painted fascia over it, on the wall above the scope (the wall's own module shows round it);
/// and an awning pulled out from the fascia when `awning`.
fn shop(m: &mut TriMesh, w: f32, h: f32, awning: bool) {
    let top = h;
    let d = -0.3;
    quad(
        m,
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, d),
            Vec3::new(0.0, top, d),
            Vec3::new(0.0, top, 0.0),
        ],
        FRAME,
    );
    quad(
        m,
        [
            Vec3::new(w, 0.0, d),
            Vec3::new(w, 0.0, 0.0),
            Vec3::new(w, top, 0.0),
            Vec3::new(w, top, d),
        ],
        FRAME,
    );
    quad(
        m,
        [
            Vec3::new(0.0, top, d),
            Vec3::new(w, top, d),
            Vec3::new(w, top, 0.0),
            Vec3::new(0.0, top, 0.0),
        ],
        FRAME,
    );
    let riser = 0.45;
    let glass = d + 0.06;
    // The shop's door at its right end, glazed to the floor and step-free (0.90 m clear: #263);
    // the whole front where it is narrower than a door and a pane (the owner missed the shops'
    // doors).
    let door = if w >= 1.9 { 0.95 } else { w };
    let shown = w - door;
    if shown > 0.0 {
        cuboid(
            m,
            Vec3::new(0.0, 0.0, d),
            Vec3::new(shown, riser, glass + 0.04),
            STONE,
            POS_Z | POS_Y,
        );
        quad(
            m,
            [
                Vec3::new(0.0, riser, glass),
                Vec3::new(shown, riser, glass),
                Vec3::new(shown, top, glass),
                Vec3::new(0.0, top, glass),
            ],
            SHOP_GLASS,
        );
    }
    quad(
        m,
        [
            Vec3::new(shown, 0.0, glass),
            Vec3::new(w, 0.0, glass),
            Vec3::new(w, top, glass),
            Vec3::new(shown, top, glass),
        ],
        SHOP_GLASS,
    );
    let f = 0.05;
    let frame = |m: &mut TriMesh, lo: Vec3, hi: Vec3| cuboid(m, lo, hi, FRAME, NO_BACK);
    let panes = (shown / 1.4).ceil().max(1.0) as u32;
    for k in 0..=panes {
        let x = (shown * k as f32 / panes as f32).clamp(f * 0.5, w - f * 0.5);
        frame(
            m,
            Vec3::new(x - f * 0.5, riser, glass - 0.01),
            Vec3::new(x + f * 0.5, top, glass + 0.05),
        );
    }
    // The door's frame to the floor, its transom and its push bar.
    for x in [shown, w] {
        let x = x.clamp(f * 0.5, w - f * 0.5);
        frame(
            m,
            Vec3::new(x - f * 0.5, 0.0, glass - 0.01),
            Vec3::new(x + f * 0.5, top, glass + 0.05),
        );
    }
    let transom = top.min(2.25);
    frame(
        m,
        Vec3::new(shown, transom - f, glass - 0.01),
        Vec3::new(w, transom, glass + 0.05),
    );
    frame(
        m,
        Vec3::new(shown + 0.1, 1.0, glass + 0.05),
        Vec3::new(w - 0.1, 1.04, glass + 0.09),
    );
    frame(
        m,
        Vec3::new(0.0, top - f, glass - 0.01),
        Vec3::new(w, top, glass + 0.05),
    );
    // The fascia, painted as the shutters are.
    cuboid(
        m,
        Vec3::new(-0.1, top + 0.1, 0.0),
        Vec3::new(w + 0.1, top + 0.62, 0.07),
        SHUTTER,
        NO_BACK,
    );
    if awning {
        // Its valance 2.3 m up over a 2.6 m shopfront: walkers pass under it.
        let (y0, y1, out) = (top + 0.55, top - 0.05, 1.5);
        // Its cheeks, seen from both sides (sheets: on one plane, the faces folded into each
        // other far off, and only a cheek's blade was left with the awning's shadow).
        for x in [0.0, w] {
            let cheek = [
                Vec3::new(x, y0, 0.08),
                Vec3::new(x, y1, out),
                Vec3::new(x, y1 - 0.22, out),
            ];
            sheet_triangle(m, cheek, FABRIC);
        }
        // Its slope, its underside seen from the pavement, and the valance along its front.
        let slope = [
            Vec3::new(0.0, y1, out),
            Vec3::new(w, y1, out),
            Vec3::new(w, y0, 0.08),
            Vec3::new(0.0, y0, 0.08),
        ];
        sheet(m, slope, FABRIC);
        quad(
            m,
            [
                Vec3::new(0.0, y1 - 0.22, out),
                Vec3::new(w, y1 - 0.22, out),
                Vec3::new(w, y1, out),
                Vec3::new(0.0, y1, out),
            ],
            FABRIC,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_builds_within_its_sections() {
        for (name, cards) in NAMES.iter().flat_map(|&n| [(n, false), (n, true)]) {
            let m = module_with(name, 1.2, 2.5, 0.22, cards).unwrap();
            assert_eq!(
                m.sections.contains(&GRILLE),
                cards && m.sections.contains(&METAL),
                "{name}: the bars on cards exactly when asked"
            );
            assert!(m.triangle_count() > 0, "{name}");
            assert_eq!(m.sections.len(), m.triangle_count(), "{name}");
            assert!(m.sections.iter().all(|&s| s < SECTIONS), "{name}");
            assert!(
                m.uvs.is_empty() || m.uvs.len() == m.positions.len(),
                "{name}: UVs for some vertices only"
            );
            assert!(
                m.normals
                    .iter()
                    .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-4),
                "{name}: a degenerate face"
            );
            // Nothing far outside its scope: shutters and balconies a little beyond it.
            for p in &m.positions {
                let p = Vec3::from(*p);
                assert!(
                    p.x > -1.0 && p.x < 2.2 && p.y > -0.5 && p.y < 3.5,
                    "{name} {p}"
                );
                // An open door's leaves reach in by half its width behind its reveal.
                let inside = if name == "door_open" { 1.2 } else { 0.5 };
                assert!(p.z > -inside && p.z < 1.6, "{name} {p}");
            }
        }
        assert!(module("spire", 1.0, 1.0).is_none());
    }

    #[test]
    fn faces_look_out_of_the_wall() {
        // A wall's quad faces +z; a window's glass faces +z behind it.
        let wall = module("wall", 2.0, 3.0).unwrap();
        assert!(wall.normals.iter().all(|n| n[2] > 0.99));
        let window = module("window_court", 1.0, 1.6).unwrap();
        let glass: Vec<usize> = (0..window.triangle_count())
            .filter(|&t| window.sections[t] == GLASS)
            .collect();
        assert!(!glass.is_empty());
        for t in glass {
            let v = window.indices[t * 3] as usize;
            assert!(window.normals[v][2] > 0.99);
            assert!(window.positions[v][2] < -0.1);
        }
        // Triangles wind counter-clockwise round their normals.
        for (name, cards) in NAMES.iter().flat_map(|&n| [(n, false), (n, true)]) {
            let m = module_with(name, 1.2, 2.5, 0.22, cards).unwrap();
            for tri in m.indices.as_chunks::<3>().0 {
                let p = tri.map(|i| Vec3::from(m.positions[i as usize]));
                let n = Vec3::from(m.normals[tri[0] as usize]);
                let face = (p[1] - p[0]).cross(p[2] - p[0]);
                assert!(face.dot(n) >= -1e-6, "{name}");
            }
        }
    }

    #[test]
    fn railings_bars_are_one_card_that_counts_them() {
        // A balcony railed on three sides: three cards, upright between the rails, each one's
        // u running from half a bar past its first end to half a bar short of the other, its
        // bars 12 cm apart or a little more.
        let m = module_with("balcony", 1.2, 2.5, 0.22, true).unwrap();
        let cards: Vec<usize> = (0..m.triangle_count())
            .filter(|&t| m.sections[t] == GRILLE)
            .collect();
        assert_eq!(cards.len(), 6);
        for pair in cards.chunks(2) {
            let corners: Vec<usize> = pair
                .iter()
                .flat_map(|&t| m.indices[t * 3..t * 3 + 3].iter().map(|&i| i as usize))
                .collect();
            let p: Vec<Vec3> = corners
                .iter()
                .map(|&i| Vec3::from(m.positions[i]))
                .collect();
            let u: Vec<f32> = corners.iter().map(|&i| m.uvs[i][0]).collect();
            let (u0, u1) = u
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
            assert_eq!(u0, 0.5);
            let bars = u1 + 0.5;
            assert_eq!(bars, bars.round());
            let (lo, hi) = p
                .iter()
                .fold((Vec3::MAX, Vec3::MIN), |(a, b), &q| (a.min(q), b.max(q)));
            let run = (hi - lo).x.max((hi - lo).z);
            let step = run / (bars - 1.0);
            assert!((0.12..0.15).contains(&step), "{step}");
            assert!((lo.y - 0.11).abs() < 1e-5 && (hi.y - 0.96).abs() < 1e-5);
        }
    }
}
