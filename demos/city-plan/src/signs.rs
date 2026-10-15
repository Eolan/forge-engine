//! Fingerposts at the avenues' junctions (the owner got lost on their second walk, 2026-10-11):
//! a pole on the corner's pavement with up to three blades, each pointing the way to a landmark
//! or a district and carrying its name, white on blue, from one atlas of names.

use forge_geom::procedural::TriMesh;
use forge_procgen::city::CityPlan;
use forge_render::textures::TextureData;
use glam::{DVec2, Vec3};

use forge_procgen::city::ground::RAMP;

/// The names' atlas: this many rows, a name each.
pub const ROWS: usize = 17;
/// The row that says no tram is due this way: a terminus's far platform, the night's pause.
pub const NO_SERVICE: usize = ROWS - 1;
/// The atlas's side, pixels.
const SIZE: usize = 1024;
/// A blade's length and height, metres.
const BLADE: (f32, f32) = (1.6, 0.26);
/// The font the names are written in (run from the engine's folder, as the shaders need).
const FONT: &str = "assets/fonts/jetbrains-mono/JetBrainsMono-Variable.ttf";

/// The blades' faces (the atlas), the pole, the blades' edges.
pub const FACE: u8 = 0;
/// The pole's section.
pub const POLE: u8 = 1;
/// The blades' edges' section.
pub const EDGE: u8 = 2;

/// A fingerpost: where it stands on the plan, and its blades (the unit way each points, on the
/// plan, and its name's row).
#[derive(Clone, Debug)]
pub struct Sign {
    /// Where its pole stands, metres on the plan.
    pub pos: DVec2,
    /// Its blades, top first.
    pub blades: Vec<(DVec2, usize)>,
}

/// "old centre" as "Old Centre", "harbour and seafront" as "Harbour and Seafront".
fn title(name: &str) -> String {
    name.split(' ')
        .enumerate()
        .map(|(i, w)| {
            if i > 0 && matches!(w, "and" | "of" | "the" | "de" | "la" | "du") {
                return w.to_owned();
            }
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The plan's fingerposts within `reach` of `centre` where `clear` allows a pole, and the names
/// their rows hold: at each junction of three ways or more where two avenues (streets with
/// raised pavements) meet, on the corner between them. Each points to the nearest of the
/// landmarks and the built districts' centres between 200 m and 4 km away, up to three, at
/// least 35° apart.
pub fn place(
    plan: &CityPlan,
    centre: DVec2,
    reach: f64,
    clear: impl Fn(DVec2) -> bool,
) -> (Vec<Sign>, Vec<String>) {
    let mut places: Vec<(DVec2, String)> = plan
        .landmarks
        .iter()
        .map(|l| (l.pos, title(&l.name)))
        .collect();
    places.extend(
        plan.districts
            .iter()
            .filter(|d| d.building_depth().is_some())
            .map(|d| (d.anchor, title(&d.name))),
    );
    let mut names: Vec<String> = Vec::new();
    for (_, name) in &places {
        if !names.contains(name) && names.len() < ROWS {
            names.push(name.clone());
        }
    }
    let row = |name: &str| names.iter().position(|n| n == name);
    // Each node's ways out: the unit direction, its half width and its pavement.
    let mut ways: Vec<Vec<(DVec2, f64, f64)>> = vec![Vec::new(); plan.roads.nodes.len()];
    for e in &plan.roads.edges {
        let n = e.points.len();
        if n < 2 {
            continue;
        }
        let (half, pave) = (e.profile.width() * 0.5, e.profile.pavement());
        let out_a = (e.points[1] - e.points[0]).normalize_or_zero();
        let out_b = (e.points[n - 2] - e.points[n - 1]).normalize_or_zero();
        ways[e.a as usize].push((out_a, half, pave));
        ways[e.b as usize].push((out_b, half, pave));
    }
    let mut signs = Vec::new();
    for (node, out) in plan.roads.nodes.iter().zip(&mut ways) {
        if out.len() < 3 || node.pos.distance(centre) > reach {
            continue;
        }
        out.sort_by(|a, b| a.0.y.atan2(a.0.x).total_cmp(&b.0.y.atan2(b.0.x)));
        // The first corner between two avenues: on the first's raised pavement, by the kerb
        // on the second's side, just past its dropped kerb (the pavements stop
        // that far from the junction).
        let widest = out.iter().map(|w| w.1).fold(0.0, f64::max);
        let corner = (0..out.len()).find_map(|i| {
            let (a, b) = (out[i], out[(i + 1) % out.len()]);
            let angle = a.0.angle_to(b.0).abs().to_degrees();
            (a.2 > 0.0 && b.2 > 0.0 && (45.0..160.0).contains(&angle)).then(|| {
                let mut across = DVec2::new(-a.0.y, a.0.x);
                if across.dot(b.0) < 0.0 {
                    across = -across;
                }
                // By the kerb, off the walkers' line; slid along clear of the trunks.
                let at = |s: f64| node.pos + a.0 * s + across * (a.1 - a.2 + 0.5);
                let start = widest + 1.0 + RAMP + 1.5;
                (0..8).map(|k| at(start + f64::from(k))).find(|&p| clear(p))
            })
        });
        let Some(pos) = corner.flatten() else {
            continue;
        };
        let mut near: Vec<(f64, DVec2, usize)> = places
            .iter()
            .filter_map(|(p, name)| {
                let d = p.distance(pos);
                ((200.0..4000.0).contains(&d))
                    .then(|| row(name).map(|r| (d, (*p - pos) / d, r)))
                    .flatten()
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut blades: Vec<(DVec2, usize)> = Vec::new();
        for (_, way, r) in near {
            if blades.len() == 3 {
                break;
            }
            if blades
                .iter()
                .all(|(w, b)| *b != r && w.angle_to(way).abs().to_degrees() >= 35.0)
            {
                blades.push((way, r));
            }
        }
        if !blades.is_empty() {
            signs.push(Sign { pos, blades });
        }
    }
    (signs, names)
}

/// A fingerpost's mesh, its pole's foot at the origin, the plan's x and y as the scene's x and
/// z: a dark pole 3.4 m tall and the blades from 2.95 m down, each 1.6 m long.
pub fn mesh(sign: &Sign) -> TriMesh {
    let mut m = TriMesh::default();
    let quad = |m: &mut TriMesh, p: [Vec3; 4], uv: [[f32; 2]; 4], normal: Vec3, s: u8| {
        let base = m.positions.len() as u32;
        let (a, b) = (p[1] - p[0], p[2] - p[0]);
        let flip = a.cross(b).dot(normal) < 0.0;
        for (q, t) in p.into_iter().zip(uv) {
            m.positions.push(q.to_array());
            m.normals.push(normal.to_array());
            m.uvs.push(t);
        }
        let tris = if flip {
            [0, 2, 1, 0, 3, 2]
        } else {
            [0, 1, 2, 0, 2, 3]
        };
        m.indices.extend(tris.map(|k| base + k));
        m.sections.extend([s, s]);
    };
    // The pole: six sides.
    let (r, top) = (0.05_f32, 3.4_f32);
    for k in 0..6 {
        let (a0, a1) = (
            k as f32 / 6.0 * std::f32::consts::TAU,
            (k + 1) as f32 / 6.0 * std::f32::consts::TAU,
        );
        let (p0, p1) = (
            Vec3::new(a0.cos() * r, 0.0, a0.sin() * r),
            Vec3::new(a1.cos() * r, 0.0, a1.sin() * r),
        );
        let out = ((p0 + p1) * 0.5).normalize();
        quad(
            &mut m,
            [p0, p1, p1 + Vec3::Y * top, p0 + Vec3::Y * top],
            [[0.0; 2]; 4],
            out,
            POLE,
        );
    }
    // The blades: the name's row across each face, read from its left end.
    let (length, high) = BLADE;
    let span = length / high * (SIZE / ROWS) as f32 / SIZE as f32;
    for (k, &(way, row)) in sign.blades.iter().enumerate() {
        let d = Vec3::new(way.x as f32, 0.0, way.y as f32);
        let side = Vec3::new(-d.z, 0.0, d.x) * 0.02;
        let y1 = 2.95 - 0.32 * k as f32;
        let y0 = y1 - high;
        let (inner, outer) = (d * 0.06, d * (0.06 + length));
        let (v0, v1) = row_span(row);
        for s in [1.0_f32, -1.0] {
            let normal = (side * s).normalize();
            // Seen from this face, its right is the view's right: (−n) × y.
            let right = (-normal).cross(Vec3::Y);
            let (left_end, right_end) = if inner.dot(right) < outer.dot(right) {
                (inner, outer)
            } else {
                (outer, inner)
            };
            let o = side * s;
            quad(
                &mut m,
                [
                    left_end + o + Vec3::Y * y1,
                    right_end + o + Vec3::Y * y1,
                    right_end + o + Vec3::Y * y0,
                    left_end + o + Vec3::Y * y0,
                ],
                [[0.0, v0], [span, v0], [span, v1], [0.0, v1]],
                normal,
                FACE,
            );
        }
        // Its edges: top, bottom and the outer end.
        let (a, b) = (side, -side);
        for (y, n) in [(y1, Vec3::Y), (y0, Vec3::NEG_Y)] {
            quad(
                &mut m,
                [
                    inner + a + Vec3::Y * y,
                    outer + a + Vec3::Y * y,
                    outer + b + Vec3::Y * y,
                    inner + b + Vec3::Y * y,
                ],
                [[0.0; 2]; 4],
                n,
                EDGE,
            );
        }
        quad(
            &mut m,
            [
                outer + a + Vec3::Y * y0,
                outer + b + Vec3::Y * y0,
                outer + b + Vec3::Y * y1,
                outer + a + Vec3::Y * y1,
            ],
            [[0.0; 2]; 4],
            d,
            EDGE,
        );
    }
    m
}

/// The names' atlas: a row each, white on a deep blue, left-aligned, with its mipmaps. Without
/// the font (not run from the engine's folder), plain blue.
pub fn atlas(names: &[String]) -> TextureData {
    text_atlas(names, [0.03, 0.12, 0.33], [0.92, 0.92, 0.90], "sign names")
}

/// The stops' screens' atlas: a row per [`screen_rows`], amber on black.
pub fn screen_atlas() -> TextureData {
    text_atlas(
        &screen_rows(),
        [0.004, 0.004, 0.006],
        [1.0, 0.55, 0.12],
        "stop screens",
    )
}

/// The atlas's texture coordinates of `row`, top and bottom: by the row's pixels, as the atlas
/// draws them (`SIZE / ROWS` each), not the row's share of the image, which drifts by a few
/// pixels when the rows do not divide it.
fn row_span(row: usize) -> (f32, f32) {
    let px = SIZE / ROWS;
    (
        (row * px) as f32 / SIZE as f32,
        ((row + 1) * px) as f32 / SIZE as f32,
    )
}

/// A stop's screen showing row `row` of [`screen_atlas`]: a plate 0.4 by 0.18 m round the
/// origin, facing −z, its text from its left end.
pub fn screen_plate(row: usize) -> TriMesh {
    let (w, h) = (0.4_f32, 0.18_f32);
    let span = w / h * (SIZE / ROWS) as f32 / SIZE as f32;
    let (v0, v1) = row_span(row);
    let mut m = TriMesh::default();
    // Seen from −z, its left is +x.
    let p = [
        Vec3::new(w * 0.5, h * 0.5, 0.0),
        Vec3::new(-w * 0.5, h * 0.5, 0.0),
        Vec3::new(-w * 0.5, -h * 0.5, 0.0),
        Vec3::new(w * 0.5, -h * 0.5, 0.0),
    ];
    let uv = [[0.0, v0], [span, v0], [span, v1], [0.0, v1]];
    for (q, t) in p.into_iter().zip(uv) {
        m.positions.push(q.to_array());
        m.normals.push([0.0, 0.0, -1.0]);
        m.uvs.push(t);
    }
    // Counter-clockwise seen from −z.
    m.indices.extend([0, 2, 1, 0, 3, 2]);
    m.sections.extend([0, 0]);
    m
}

/// What a stop's screen shows, by row: the next tram due, then in 1 to 14 minutes, then in 15
/// or more.
pub fn screen_rows() -> Vec<String> {
    let mut rows = vec!["Due".to_owned()];
    rows.extend((1..15).map(|m| format!("{m} min")));
    rows.push("15+ min".to_owned());
    // Rather than a dark screen where no tram is due this way (the owner: "the tram's minutes
    // not showing, no tram coming, a lot of people waiting").
    rows.push("No service".to_owned());
    rows
}

/// `names`, a row each, `fg` on `bg` (linear colours), with its mipmaps; plain `bg` without the
/// font.
fn text_atlas(names: &[String], bg: [f32; 3], fg: [f32; 3], name: &'static str) -> TextureData {
    use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
    let (blue, white) = (bg, fg);
    let mut cover = vec![0.0_f32; SIZE * SIZE];
    let row_px = SIZE / ROWS;
    let bytes = std::fs::read(FONT)
        .inspect_err(|e| tracing::warn!("the signs' font {FONT}: {e}"))
        .ok();
    if let Some(font) = bytes
        .as_deref()
        .and_then(|b| FontRef::try_from_slice(b).ok())
    {
        let scaled = font.as_scaled(PxScale::from(row_px as f32 * 0.5));
        for (r, name) in names.iter().enumerate().take(ROWS) {
            let top = (r * row_px) as f32 + (row_px as f32 - scaled.height()) * 0.5;
            let mut x = 10.0;
            for c in name.chars() {
                let mut glyph = scaled.scaled_glyph(c);
                glyph.position = point(x, top + scaled.ascent());
                x += scaled.h_advance(glyph.id);
                if let Some(outlined) = font.outline_glyph(glyph) {
                    let b = outlined.px_bounds();
                    outlined.draw(|gx, gy, v| {
                        let (px, py) = (
                            b.min.x as i64 + i64::from(gx),
                            b.min.y as i64 + i64::from(gy),
                        );
                        if (0..SIZE as i64).contains(&px) && (0..SIZE as i64).contains(&py) {
                            let c = &mut cover[py as usize * SIZE + px as usize];
                            *c = c.max(v);
                        }
                    });
                }
            }
        }
    }
    let srgb = |c: f32| {
        let s = if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    };
    let mut level: Vec<[f32; 3]> = cover
        .iter()
        .map(|&v| [0, 1, 2].map(|k| blue[k] + (white[k] - blue[k]) * v))
        .collect();
    let encode = |level: &[[f32; 3]]| {
        level
            .iter()
            .flat_map(|c| [srgb(c[0]), srgb(c[1]), srgb(c[2]), 255])
            .collect::<Vec<u8>>()
    };
    let mut levels = vec![encode(&level)];
    let mut side = SIZE;
    while side > 1 {
        let half = side / 2;
        let mut next = Vec::with_capacity(half * half);
        for y in 0..half {
            for x in 0..half {
                let i = 2 * y * side + 2 * x;
                let q = [level[i], level[i + 1], level[i + side], level[i + side + 1]];
                next.push([0, 1, 2].map(|k| q.iter().map(|c| c[k]).sum::<f32>() * 0.25));
            }
        }
        level = next;
        side = half;
        levels.push(encode(&level));
    }
    TextureData {
        name: name.into(),
        size: SIZE as u32,
        height: SIZE as u32,
        srgb: true,
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_titled() {
        assert_eq!(title("old centre"), "Old Centre");
        assert_eq!(title("Market Hall"), "Market Hall");
        assert_eq!(title("harbour and seafront"), "Harbour and Seafront");
    }

    #[test]
    fn a_sign_reads_from_its_left_on_both_faces() {
        let sign = Sign {
            pos: DVec2::ZERO,
            blades: vec![(DVec2::X, 2), (DVec2::Y, 5)],
        };
        let m = mesh(&sign);
        assert_eq!(m.uvs.len(), m.positions.len());
        assert_eq!(m.sections.len() * 3, m.indices.len());
        // The faces' rows: 2/16 to 3/16 and 5/16 to 6/16 in v.
        let faces: Vec<usize> = (0..m.sections.len())
            .filter(|&t| m.sections[t] == FACE)
            .collect();
        assert_eq!(faces.len(), 8);
        for t in faces {
            let n = Vec3::from(m.normals[m.indices[3 * t] as usize]);
            let right = (-n).cross(Vec3::Y);
            // The vertex with u = 0 is left of the one with the largest u.
            let vs: Vec<usize> = (0..3).map(|k| m.indices[3 * t + k] as usize).collect();
            let at = |v: usize| Vec3::from(m.positions[v]).dot(right);
            let lo = vs
                .iter()
                .min_by(|&&a, &&b| m.uvs[a][0].total_cmp(&m.uvs[b][0]))
                .unwrap();
            let hi = vs
                .iter()
                .max_by(|&&a, &&b| m.uvs[a][0].total_cmp(&m.uvs[b][0]))
                .unwrap();
            assert!(at(*lo) < at(*hi));
        }
    }
}
