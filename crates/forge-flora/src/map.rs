//! The placed hillside drawn on the CPU: the ground in relief with its contours, the stream and
//! the track, each plant's crown from above by species, a legend, and a cross-section from
//! north to south with the plants near it drawn from the side from grown individuals.

use glam::{DVec2, Vec2};

use crate::draw::{self, Canvas, INK, PAPER, View};
use crate::graph::TreeGraph;
use crate::place::{Fields, Ground, Placement};
use crate::species::Species;

/// The map's inputs.
pub struct HillsideMap<'a> {
    /// The ground.
    pub ground: &'a Ground,
    /// Its fields.
    pub fields: &'a Fields,
    /// The species placed.
    pub species: &'a [Species],
    /// The placement.
    pub placement: &'a Placement,
    /// Lines drawn as water (streams), metres on the plan.
    pub water: &'a [Vec<DVec2>],
    /// Lines drawn as tracks.
    pub tracks: &'a [Vec<DVec2>],
    /// Grown individuals per species (a few each), for the cross-section.
    pub grown: &'a [Vec<TreeGraph>],
    /// The cross-section's `x`, metres: it runs north to south.
    pub section_x: f64,
    /// Labels on the plan: text and place.
    pub labels: &'a [(String, DVec2)],
    /// The engine's font.
    pub font: Option<&'a [u8]>,
}

/// Metres per pixel of the plan.
const PLAN_SCALE: f64 = 0.7;

/// Draws the map.
pub fn hillside(m: &HillsideMap) -> Canvas {
    let size = m.ground.size();
    let side = (size / PLAN_SCALE).round() as u32;
    let legend_w = 300u32;
    let section_h = 330u32;
    let top = 70u32;
    let width = side + legend_w + 60;
    let height = top + side + 40 + 2 * (section_h + 40);
    let mut c = Canvas::new(width, height, PAPER);
    let origin = Vec2::new(30.0, top as f32);
    // The ground: hillshade from the north-west over a tint by soil depth.
    let light = glam::DVec3::new(-0.5, 0.75, -0.45).normalize();
    for py in 0..side {
        for px in 0..side {
            let p = DVec2::new(
                (f64::from(px) + 0.5) * PLAN_SCALE,
                (f64::from(py) + 0.5) * PLAN_SCALE,
            );
            let e = 1.0;
            let gx =
                (m.ground.height(p + DVec2::X * e) - m.ground.height(p - DVec2::X * e)) / (2.0 * e);
            let gy =
                (m.ground.height(p + DVec2::Y * e) - m.ground.height(p - DVec2::Y * e)) / (2.0 * e);
            // World normal: x east, y up, z south (the plan's y).
            let n = glam::DVec3::new(-gx, 1.0, -gy).normalize();
            // In eight steps: flat tones compress, the map stays under a megabyte.
            let shade = ((0.55 + 0.45 * n.dot(light)).clamp(0.0, 1.0) * 8.0).round() as f32 / 8.0;
            let soil = ((m.fields.site(p).soil * 3.0).round() / 3.0) as f32;
            let rock = [214.0f32, 208.0, 190.0];
            let earth = [188.0f32, 168.0, 128.0];
            let base: Vec<f32> = (0..3)
                .map(|k| rock[k] + (earth[k] - rock[k]) * soil)
                .collect();
            let col = [0, 1, 2].map(|k| (base[k] * (0.62 + 0.45 * shade)).min(255.0) as u8);
            c.blend(
                i64::from(origin.x as u32 + px),
                i64::from(top + py),
                col,
                1.0,
            );
        }
    }
    // Contours every 5 m, every 25 m darker.
    for py in 0..side - 1 {
        for px in 0..side - 1 {
            let at = |x: u32, y: u32| {
                m.ground.height(DVec2::new(
                    (f64::from(x) + 0.5) * PLAN_SCALE,
                    (f64::from(y) + 0.5) * PLAN_SCALE,
                ))
            };
            let h = at(px, py);
            for (qx, qy) in [(px + 1, py), (px, py + 1)] {
                let o = at(qx, qy);
                let (a, b) = ((h / 5.0).floor(), (o / 5.0).floor());
                if a != b {
                    let major = (a.max(b) as i64) % 5 == 0;
                    c.blend(
                        i64::from(origin.x as u32 + px),
                        i64::from(top + py),
                        [110, 96, 78],
                        if major { 0.55 } else { 0.25 },
                    );
                }
            }
        }
    }
    let to_px = |p: DVec2| origin + (p / PLAN_SCALE).as_vec2();
    for line in m.tracks {
        for w in line.windows(2) {
            c.capsule(to_px(w[0]), to_px(w[1]), 2.4, 2.4, [226, 210, 172], 1.0);
        }
    }
    for line in m.water {
        for w in line.windows(2) {
            c.capsule(to_px(w[0]), to_px(w[1]), 2.2, 2.2, [96, 146, 196], 1.0);
        }
    }
    // The plants, the lowest first so the canopy covers the shrubs.
    let mut order: Vec<usize> = (0..m.placement.plants.len()).collect();
    order.sort_by(|&a, &b| {
        let (pa, pb) = (&m.placement.plants[a], &m.placement.plants[b]);
        pa.height.total_cmp(&pb.height).then(a.cmp(&b))
    });
    for k in order {
        let p = &m.placement.plants[k];
        let sp = &m.species[p.species as usize];
        let col = draw::colours(&sp.id, sp.foliage).leaves;
        let r = (f64::from(p.crown) / PLAN_SCALE) as f32;
        let at = to_px(p.pos);
        if r > 3.0 {
            let rim = col.map(|v| (f32::from(v) * 0.7) as u8);
            c.disc(at, r, rim, 0.95);
            c.disc(at, (r - 1.2).max(0.5), col, 0.95);
        } else {
            c.disc(at, r.max(0.7), col, 0.9);
        }
    }
    // The section's line.
    let (a, b) = (
        to_px(DVec2::new(m.section_x, 0.0)),
        to_px(DVec2::new(m.section_x, size)),
    );
    for k in 0..((b.y - a.y) / 8.0) as i32 {
        let y = a.y + k as f32 * 8.0;
        c.line(
            Vec2::new(a.x, y),
            Vec2::new(a.x, y + 4.0),
            [180, 40, 40],
            0.9,
        );
    }
    c.text(m.font, a + Vec2::new(4.0, 2.0), 12.0, "N", [180, 40, 40]);
    c.text(m.font, b + Vec2::new(4.0, -16.0), 12.0, "S", [180, 40, 40]);
    for (text, at) in m.labels {
        let p = to_px(*at);
        // On a pale band, readable over the crowns.
        let w = text.chars().count() as f32 * 7.9 + 8.0;
        for y in (p.y - 2.0) as i64..(p.y + 18.0) as i64 {
            for x in (p.x - 4.0) as i64..(p.x + w) as i64 {
                c.blend(x, y, PAPER, 0.8);
            }
        }
        c.text(m.font, p, 13.0, text, INK);
    }
    // Title.
    c.text(
        m.font,
        Vec2::new(30.0, 16.0),
        22.0,
        "A placed hillside (#269): garrigue on the sunny slope, holm oaks in the valley",
        INK,
    );
    c.text(
        m.font,
        Vec2::new(30.0, 44.0),
        13.0,
        &format!(
            "{:.0} m square, north up, contours every 5 m, 0.7 m a pixel; plants by their habitats, the light under the canopy and spacing; {} plants",
            size,
            m.placement.plants.len()
        ),
        INK,
    );
    // Legend: each species' colour and count.
    let lx = origin.x + side as f32 + 24.0;
    let mut ly = top as f32 + 4.0;
    c.text(m.font, Vec2::new(lx, ly), 15.0, "Species", INK);
    ly += 26.0;
    for (k, sp) in m.species.iter().enumerate() {
        let count = m
            .placement
            .plants
            .iter()
            .filter(|p| usize::from(p.species) == k)
            .count();
        let planted = m
            .placement
            .plants
            .iter()
            .filter(|p| usize::from(p.species) == k && p.planted)
            .count();
        let col = draw::colours(&sp.id, sp.foliage).leaves;
        c.disc(Vec2::new(lx + 8.0, ly + 8.0), 7.0, col, 1.0);
        c.text(m.font, Vec2::new(lx + 24.0, ly), 13.0, &sp.name, INK);
        let note = if planted > 0 {
            format!("{count} planted")
        } else {
            format!("{count}")
        };
        c.text(m.font, Vec2::new(lx + 24.0, ly + 16.0), 11.0, &note, INK);
        ly += 40.0;
    }
    ly += 10.0;
    c.capsule(
        Vec2::new(lx, ly),
        Vec2::new(lx + 30.0, ly),
        2.2,
        2.2,
        [96, 146, 196],
        1.0,
    );
    c.text(m.font, Vec2::new(lx + 40.0, ly - 8.0), 12.0, "stream", INK);
    ly += 22.0;
    c.capsule(
        Vec2::new(lx, ly),
        Vec2::new(lx + 30.0, ly),
        2.4,
        2.4,
        [226, 210, 172],
        1.0,
    );
    c.text(m.font, Vec2::new(lx + 40.0, ly - 8.0), 12.0, "track", INK);
    ly += 30.0;
    let bar = (100.0 / PLAN_SCALE) as f32;
    c.capsule(
        Vec2::new(lx, ly),
        Vec2::new(lx + bar, ly),
        1.5,
        1.5,
        INK,
        1.0,
    );
    c.text(m.font, Vec2::new(lx, ly + 6.0), 12.0, "100 m", INK);
    // The cross-sections, north on the left: the whole slope, then the valley closer.
    let left = origin.x;
    let sy = (top + side + 40) as f32;
    section(
        &mut c,
        m,
        (0.0, size),
        Vec2::new(left, sy),
        Vec2::new(side as f32, section_h as f32),
        "The section from north to south along the dashed line, plants within 10 m of it, no exaggeration",
    );
    let v = crate::place::example_valley(m.section_x, size);
    let (y0, y1) = ((v - 130.0).max(0.0), (v + 130.0).min(size));
    section(
        &mut c,
        m,
        (y0, y1),
        Vec2::new(left, sy + section_h as f32 + 40.0),
        Vec2::new(side as f32, section_h as f32),
        &format!(
            "The same section closer: {:.0} m across the valley, from the olives and the garrigue to the holm oaks",
            y1 - y0
        ),
    );
    // Channels in steps of 8: fewer colours, the map under a megabyte.
    c.posterize(8.0);
    c
}

/// Draws the section along `m.section_x` from `span.0` to `span.1` (metres of `y`, north on the
/// left) into the box at `at` of `extent` pixels, ground and plants at one scale.
fn section(c: &mut Canvas, m: &HillsideMap, span: (f64, f64), at: Vec2, extent: Vec2, title: &str) {
    c.text(m.font, at, 15.0, title, INK);
    let metres_per_px = (span.1 - span.0) / f64::from(extent.x);
    let scale = (1.0 / metres_per_px) as f32;
    let columns = extent.x as usize;
    let heights: Vec<f64> = (0..=columns)
        .map(|k| {
            m.ground
                .height(DVec2::new(m.section_x, span.0 + k as f64 * metres_per_px))
        })
        .collect();
    let hmin = heights.iter().copied().fold(f64::MAX, f64::min);
    // The ground's lowest point near the box's foot.
    let floor = at.y + extent.y - 30.0;
    let ground_y = |h: f64| floor - ((h - hmin) as f32) * scale;
    // Plants near the section, the farthest (east) first, drawn from the side at their size.
    let mut near: Vec<usize> = (0..m.placement.plants.len())
        .filter(|&k| {
            let p = &m.placement.plants[k];
            (p.pos.x - m.section_x).abs() < 10.0 && p.pos.y >= span.0 && p.pos.y <= span.1
        })
        .collect();
    near.sort_by(|&a, &b| {
        let (pa, pb) = (&m.placement.plants[a], &m.placement.plants[b]);
        pb.pos.x.total_cmp(&pa.pos.x).then(a.cmp(&b))
    });
    for k in near {
        let p = &m.placement.plants[k];
        let sp = &m.species[p.species as usize];
        let variants = &m.grown[p.species as usize];
        if variants.is_empty() {
            continue;
        }
        let g = &variants[(p.seed % variants.len() as u64) as usize];
        let s = p.height / g.height().max(0.01);
        let base = Vec2::new(
            at.x + ((p.pos.y - span.0) / metres_per_px) as f32,
            ground_y(m.ground.height(p.pos)),
        );
        draw::plant(
            c,
            g,
            View::Side,
            base,
            scale * s,
            draw::colours(&sp.id, sp.foliage),
        );
    }
    // The ground's profile over the plants' feet.
    for (k, &h) in heights.iter().enumerate().take(columns) {
        let x = at.x + k as f32;
        let y = ground_y(h);
        for yy in y as i64..(at.y + extent.y) as i64 - 18 {
            c.blend(x as i64, yy, [196, 180, 150], 1.0);
        }
        c.blend(x as i64, y as i64, [120, 100, 76], 1.0);
    }
    let foot = at.y + extent.y - 16.0;
    c.text(m.font, Vec2::new(at.x, foot), 13.0, "N", INK);
    c.text(
        m.font,
        Vec2::new(at.x + extent.x - 10.0, foot),
        13.0,
        "S",
        INK,
    );
}
