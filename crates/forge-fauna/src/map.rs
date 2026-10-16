//! A day of the animals drawn on the CPU: the plan from above (water, pasture, gardens,
//! streets, buildings, walls, trees), each animal's path by species, a legend and the hours each
//! species spends moving. On `forge-flora`'s canvas; PNG through `image`.

use forge_flora::draw::{Canvas, INK};
use glam::{Vec2, Vec3};

use crate::plan::Plan;
use crate::species::{Behaviour, Species};

/// A species' colour on the map.
pub fn colour(b: Behaviour) -> [u8; 3] {
    match b {
        Behaviour::Sparrow => [150, 95, 40],
        Behaviour::Pigeon => [70, 90, 170],
        Behaviour::Gull => [235, 140, 10],
        Behaviour::Swift => [30, 30, 30],
        Behaviour::Cat => [205, 35, 130],
        Behaviour::Dog => [215, 25, 25],
        Behaviour::Goat => [120, 70, 170],
        Behaviour::Fish => [0, 150, 165],
    }
}

/// How strongly a step of a path at height `z` is drawn: flight above 15 m faintly (gulls
/// soaring, pigeons circling), the swifts only below 20 m (their screaming parties down the
/// streets: aloft they cover the map and beyond all day), the fish faintly.
pub fn alpha(b: Behaviour, z: f32) -> f32 {
    match b {
        Behaviour::Swift if z < 20.0 => 0.3,
        Behaviour::Swift => 0.0,
        Behaviour::Fish => 0.12,
        _ if z > 15.0 => 0.04,
        _ => 0.3,
    }
}

/// The map: a canvas with the plan drawn, in pixels from the plan's metres.
pub struct DayMap {
    /// The canvas.
    pub canvas: Canvas,
    /// Pixels a metre.
    pub scale: f32,
    /// The plan's origin, pixels.
    pub origin: Vec2,
    /// The plan's side, pixels.
    pub side: f32,
}

const PLAN_COLOURS: [[u8; 3]; 9] = [
    [242, 238, 224], // ground
    [222, 228, 196], // pasture
    [205, 195, 172], // fold
    [184, 212, 234], // water
    [212, 228, 196], // gardens
    [232, 226, 212], // squares
    [206, 204, 198], // streets
    [146, 140, 134], // buildings
    [104, 98, 92],   // walls
];

/// A filled polygon (even–odd), sampled four times a pixel in height.
fn polygon(c: &mut Canvas, pts: &[Vec2], colour: [u8; 3], alpha: f32) {
    if pts.len() < 3 {
        return;
    }
    let (lo, hi) = pts.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let y0 = lo.y.floor().max(0.0) as i64;
    let y1 = hi.y.ceil().min(c.height as f32) as i64;
    let n = pts.len();
    for y in y0..y1 {
        let mut cover = vec![0.0f32; (hi.x.ceil() - lo.x.floor()) as usize + 2];
        let x_base = lo.x.floor() as i64;
        for sub in 0..4 {
            let sy = y as f32 + (sub as f32 + 0.5) / 4.0;
            let mut xs = Vec::new();
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.y > sy) != (b.y > sy) {
                    xs.push(a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            xs.sort_by(f32::total_cmp);
            for pair in xs.chunks(2) {
                if let [xa, xb] = pair {
                    let (xa, xb) = (*xa - x_base as f32, *xb - x_base as f32);
                    let (ia, ib) = (xa.floor() as i64, xb.floor() as i64);
                    for x in ia.max(0)..=ib.min(cover.len() as i64 - 1) {
                        let l = (x as f32).max(xa);
                        let r = ((x + 1) as f32).min(xb);
                        if r > l {
                            cover[x as usize] += (r - l) * 0.25;
                        }
                    }
                }
            }
        }
        for (k, &a) in cover.iter().enumerate() {
            if a > 0.0 {
                c.blend(x_base + k as i64, y, colour, a.min(1.0) * alpha);
            }
        }
    }
}

impl DayMap {
    /// The plan drawn at `scale` pixels a metre, with `panel` pixels on the right for the legend
    /// and `strip` below for the hours.
    pub fn new(plan: &Plan, scale: f32, panel: u32, strip: u32) -> Self {
        let side = plan.parts.size * scale;
        let margin = 20.0;
        let w = (side + margin * 2.0) as u32 + panel;
        let h = (side + margin * 2.0) as u32 + strip;
        let mut m = Self {
            canvas: Canvas::new(w, h, [250, 250, 246]),
            scale,
            origin: Vec2::splat(margin),
            side,
        };
        let p = &plan.parts;
        let sq = |m: &Self| {
            let o = m.origin;
            vec![
                o,
                o + Vec2::new(m.side, 0.0),
                o + Vec2::splat(m.side),
                o + Vec2::new(0.0, m.side),
            ]
        };
        let ground = sq(&m);
        polygon(&mut m.canvas, &ground, PLAN_COLOURS[0], 1.0);
        let px = |m: &Self, v: &[Vec2]| v.iter().map(|&q| m.px(q)).collect::<Vec<_>>();
        let pasture = px(&m, &p.pasture);
        polygon(&mut m.canvas, &pasture, PLAN_COLOURS[1], 1.0);
        let fold = px(&m, &p.fold);
        polygon(&mut m.canvas, &fold, PLAN_COLOURS[2], 1.0);
        for w in &p.water {
            let w = px(&m, w);
            polygon(&mut m.canvas, &w, PLAN_COLOURS[3], 1.0);
        }
        for g in &p.gardens {
            let g = px(&m, &g.polygon);
            polygon(&mut m.canvas, &g, PLAN_COLOURS[4], 1.0);
        }
        for s in &p.squares {
            let s = px(&m, s);
            polygon(&mut m.canvas, &s, PLAN_COLOURS[5], 1.0);
        }
        for s in &p.streets {
            let (a, b) = (m.px(p.nodes[s.a as usize]), m.px(p.nodes[s.b as usize]));
            let r = if s.limit > 0.0 { 3.5 } else { 1.2 } * scale;
            m.canvas.capsule(a, b, r, r, PLAN_COLOURS[6], 1.0);
        }
        for t in &p.trees {
            let c = m.px(t.pos);
            m.canvas.disc(c, t.crown * scale, [110, 150, 100], 0.35);
        }
        for &s in &p.shrubs {
            let c = m.px(s);
            m.canvas.disc(c, 0.5 * scale, [150, 165, 120], 0.5);
        }
        for b in &p.buildings {
            let f = px(&m, &b.footprint);
            polygon(&mut m.canvas, &f, PLAN_COLOURS[7], 1.0);
        }
        for w in &p.walls {
            let (a, b) = (m.px(w.a), m.px(w.b));
            m.canvas.line(a, b, PLAN_COLOURS[8], 1.0);
        }
        if p.quay.len() >= 2 {
            for k in 0..p.quay.len() - 1 {
                let (a, b) = (m.px(p.quay[k]), m.px(p.quay[k + 1]));
                m.canvas.capsule(a, b, 1.2, 1.2, [90, 90, 96], 1.0);
            }
        }
        m
    }

    /// A plan point in pixels.
    pub fn px(&self, p: Vec2) -> Vec2 {
        self.origin + p * self.scale
    }

    /// A step of a path, `a` to `b`, in `colour` at `alpha`.
    pub fn path(&mut self, a: Vec3, b: Vec3, colour: [u8; 3], alpha: f32) {
        let size = self.side / self.scale;
        let on = |p: Vec3| (0.0..=size).contains(&p.x) && (0.0..=size).contains(&p.y);
        if !on(a) || !on(b) {
            return;
        }
        let (pa, pb) = (self.px(a.truncate()), self.px(b.truncate()));
        self.canvas.capsule(pa, pb, 0.45, 0.45, colour, alpha);
    }

    /// The legend in the panel: each species' colour, name and count, and lines of notes.
    pub fn legend(
        &mut self,
        font: Option<&[u8]>,
        title: &str,
        rows: &[(Behaviour, String)],
        notes: &[String],
    ) {
        let x = self.origin.x * 2.0 + self.side + 10.0;
        let mut y = self.origin.y;
        self.canvas.text(font, Vec2::new(x, y), 17.0, title, INK);
        y += 34.0;
        for (b, label) in rows {
            self.canvas.capsule(
                Vec2::new(x, y + 7.0),
                Vec2::new(x + 26.0, y + 7.0),
                2.5,
                2.5,
                colour(*b),
                0.9,
            );
            self.canvas
                .text(font, Vec2::new(x + 36.0, y), 13.0, label, INK);
            y += 22.0;
        }
        y += 12.0;
        for n in notes {
            self.canvas.text(font, Vec2::new(x, y), 12.0, n, INK);
            y += 18.0;
        }
        // A 50 m bar.
        let (a, b) = (
            Vec2::new(x, y + 20.0),
            Vec2::new(x + 50.0 * self.scale, y + 20.0),
        );
        self.canvas.capsule(a, b, 1.5, 1.5, INK, 1.0);
        self.canvas
            .text(font, Vec2::new(x, y + 26.0), 12.0, "50 m", INK);
    }

    /// The hours strip: for each species a row of 24 cells, darker the larger the share of its
    /// animals moving (faster than 0.1 m/s) that hour.
    pub fn hours(&mut self, font: Option<&[u8]>, species: &[Species], moving: &[[f32; 24]]) {
        let x0 = self.origin.x + 140.0;
        let y0 = self.origin.y * 2.0 + self.side + 8.0;
        let cell = ((self.side - 140.0) / 24.0).floor();
        self.canvas.text(
            font,
            Vec2::new(self.origin.x, y0 - 2.0),
            12.0,
            "share moving by hour (faster than 0.1 m/s)",
            INK,
        );
        for h in 0..24 {
            if h % 3 == 0 {
                self.canvas.text(
                    font,
                    Vec2::new(x0 + h as f32 * cell, y0 + 14.0),
                    11.0,
                    &format!("{h:02}"),
                    INK,
                );
            }
        }
        for (s, sp) in species.iter().enumerate() {
            let y = y0 + 32.0 + s as f32 * 18.0;
            self.canvas
                .text(font, Vec2::new(self.origin.x, y), 12.0, &sp.name, INK);
            for (h, share) in moving[s].iter().enumerate() {
                let v = share.clamp(0.0, 1.0);
                let lo = Vec2::new(x0 + h as f32 * cell, y);
                let pts = [
                    lo,
                    lo + Vec2::new(cell - 2.0, 0.0),
                    lo + Vec2::new(cell - 2.0, 15.0),
                    lo + Vec2::new(0.0, 15.0),
                ];
                polygon(&mut self.canvas, &pts, [236, 234, 228], 1.0);
                polygon(&mut self.canvas, &pts, colour(sp.behaviour), v);
            }
        }
    }
}
