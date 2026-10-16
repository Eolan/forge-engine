//! CPU drawings for the owner: plants seen from the side and from above, wood as capsules by
//! the pipe model's radii and leaf clusters as discs shaded by depth, with rulers and labels in
//! the engine's font. A small rasteriser with coverage anti-aliasing; PNG through `image`.

use std::path::Path;

use glam::{Vec2, Vec3};
use image::{ImageBuffer, Rgb};

use crate::graph::TreeGraph;
use crate::species::Foliage;

/// An RGB canvas.
pub struct Canvas {
    /// Its width, pixels.
    pub width: u32,
    /// Its height, pixels.
    pub height: u32,
    pixels: Vec<[f32; 3]>,
}

/// The paper's colour.
pub const PAPER: [u8; 3] = [246, 243, 236];
/// The ink's colour, for rulers and text.
pub const INK: [u8; 3] = [60, 58, 54];

fn lin(c: [u8; 3]) -> [f32; 3] {
    c.map(f32::from)
}

impl Canvas {
    /// A canvas filled with `background`.
    pub fn new(width: u32, height: u32, background: [u8; 3]) -> Self {
        Self {
            width,
            height,
            pixels: vec![lin(background); (width * height) as usize],
        }
    }

    /// Blends `c` into a pixel by `alpha`.
    pub fn blend(&mut self, x: i64, y: i64, c: [u8; 3], alpha: f32) {
        if x < 0 || y < 0 || x >= i64::from(self.width) || y >= i64::from(self.height) {
            return;
        }
        let a = alpha.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let p = &mut self.pixels[y as usize * self.width as usize + x as usize];
        for k in 0..3 {
            p[k] += (f32::from(c[k]) - p[k]) * a;
        }
    }

    /// A filled disc, anti-aliased at its edge.
    pub fn disc(&mut self, centre: Vec2, radius: f32, c: [u8; 3], alpha: f32) {
        let r = radius.max(0.5);
        let x0 = (centre.x - r - 1.0).floor() as i64;
        let x1 = (centre.x + r + 1.0).ceil() as i64;
        let y0 = (centre.y - r - 1.0).floor() as i64;
        let y1 = (centre.y + r + 1.0).ceil() as i64;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(centre);
                let cover = (r + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, c, cover * alpha);
                }
            }
        }
    }

    /// A capsule from `a` to `b` of half-width `ra` at `a` and `rb` at `b`, anti-aliased.
    pub fn capsule(&mut self, a: Vec2, b: Vec2, ra: f32, rb: f32, c: [u8; 3], alpha: f32) {
        let ra = ra.max(0.45);
        let rb = rb.max(0.45);
        let r = ra.max(rb);
        let lo = a.min(b) - Vec2::splat(r + 1.0);
        let hi = a.max(b) + Vec2::splat(r + 1.0);
        let ab = b - a;
        let len2 = ab.length_squared().max(1e-9);
        for y in lo.y.floor() as i64..=hi.y.ceil() as i64 {
            for x in lo.x.floor() as i64..=hi.x.ceil() as i64 {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
                let d = p.distance(a + ab * t);
                let rr = ra + (rb - ra) * t;
                let cover = (rr + 0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, c, cover * alpha);
                }
            }
        }
    }

    /// A one-pixel line.
    pub fn line(&mut self, a: Vec2, b: Vec2, c: [u8; 3], alpha: f32) {
        self.capsule(a, b, 0.5, 0.5, c, alpha);
    }

    /// Text with its top-left corner at `at`, `px` pixels tall, in `font` (a TrueType file's
    /// bytes); nothing without a font.
    pub fn text(&mut self, font: Option<&[u8]>, at: Vec2, px: f32, s: &str, c: [u8; 3]) {
        use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
        let Some(font) = font.and_then(|f| FontRef::try_from_slice(f).ok()) else {
            return;
        };
        let units_per_em = font.units_per_em().unwrap_or(1000.0);
        let scaled = font.as_scaled(PxScale::from(px * font.height_unscaled() / units_per_em));
        let mut x = at.x;
        for ch in s.chars() {
            let mut glyph = scaled.scaled_glyph(ch);
            glyph.position = point(x, at.y + scaled.ascent());
            x += scaled.h_advance(glyph.id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                let mut cover = Vec::new();
                outlined.draw(|gx, gy, v| cover.push((gx, gy, v)));
                for (gx, gy, v) in cover {
                    self.blend(
                        bounds.min.x as i64 + i64::from(gx),
                        bounds.min.y as i64 + i64::from(gy),
                        c,
                        v,
                    );
                }
            }
        }
    }

    /// Writes the canvas as a PNG.
    pub fn save(&self, path: &Path) -> image::ImageResult<()> {
        let w = self.width;
        ImageBuffer::from_fn(self.width, self.height, |x, y| {
            let p = self.pixels[(y * w + x) as usize];
            Rgb(p.map(|v| v.round().clamp(0.0, 255.0) as u8))
        })
        .save(path)
    }
}

/// A plant's colours: its bark and its leaves.
#[derive(Clone, Copy, Debug)]
pub struct Colours {
    /// The wood's.
    pub bark: [u8; 3],
    /// The leaves'.
    pub leaves: [u8; 3],
}

/// The colours a species is drawn in, by its id (a neutral pair for others).
pub fn colours(id: &str, foliage: Foliage) -> Colours {
    let (bark, leaves) = match id {
        "aleppo-pine" => ([128, 84, 62], [96, 128, 70]),
        "holm-oak" => ([92, 84, 76], [52, 84, 52]),
        "olive" => ([132, 126, 116], [128, 146, 112]),
        "italian-cypress" => ([104, 82, 64], [34, 66, 44]),
        "rosemary" => ([104, 90, 74], [88, 120, 98]),
        "cistus" => ([110, 96, 82], [154, 168, 142]),
        "kermes-oak" => ([96, 84, 70], [78, 108, 58]),
        _ => match foliage {
            Foliage::Needles => ([110, 86, 66], [80, 116, 72]),
            Foliage::Broad => ([100, 88, 76], [72, 110, 64]),
            Foliage::Scale => ([100, 82, 66], [40, 72, 48]),
        },
    };
    Colours { bark, leaves }
}

/// A view's projection: which axes go across and up the picture, and which is depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// From the side: X across, Y up, depth Z (towards the viewer at +Z).
    Side,
    /// From above: X across, Z down the page (north, −Z, at the top), depth Y.
    Top,
}

impl View {
    fn project(self, p: Vec3) -> (Vec2, f32) {
        match self {
            View::Side => (Vec2::new(p.x, -p.y), p.z),
            View::Top => (Vec2::new(p.x, p.z), p.y),
        }
    }
}

/// Draws a plant at `origin` (its base, in pixels) at `scale` pixels a metre: wood and leaf
/// clusters sorted back to front, the leaves darker where they lie deeper.
pub fn plant(
    canvas: &mut Canvas,
    g: &TreeGraph,
    view: View,
    origin: Vec2,
    scale: f32,
    colours: Colours,
) {
    enum Item {
        Wood(Vec2, Vec2, f32, f32),
        Leaf(Vec2, f32),
    }
    let mut items: Vec<(f32, Item)> = Vec::new();
    for s in g.segments() {
        let (a, da) = view.project(s.a);
        let (b, db) = view.project(s.b);
        let parent_radius = g.nodes[g.nodes[s.node as usize].parent as usize].radius;
        let ra = parent_radius.min(s.radius * 1.15) * scale;
        items.push((
            (da + db) * 0.5,
            Item::Wood(origin + a * scale, origin + b * scale, ra, s.radius * scale),
        ));
    }
    for l in &g.leaves {
        let (p, d) = view.project(l.pos);
        items.push((d, Item::Leaf(origin + p * scale, l.size * scale)));
    }
    // Back to front; ties keep the graph's order.
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (lo, hi) = items.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (d, _)| {
        (lo.min(*d), hi.max(*d))
    });
    let span = (hi - lo).max(1e-3);
    for (d, item) in &items {
        let k = (d - lo) / span;
        match *item {
            Item::Wood(a, b, ra, rb) => canvas.capsule(a, b, ra, rb, colours.bark, 1.0),
            Item::Leaf(p, r) => {
                let shade = 0.62 + 0.38 * k;
                let c = colours.leaves.map(|v| (f32::from(v) * shade) as u8);
                canvas.disc(p, r, c, 0.85);
            }
        }
    }
}

/// A vertical ruler at `x` pixels from `base` (the ground's pixel row) up `metres`, a tick
/// every `step` metres, labelled.
pub fn ruler(
    canvas: &mut Canvas,
    font: Option<&[u8]>,
    x: f32,
    base: f32,
    scale: f32,
    metres: f32,
    step: f32,
) {
    canvas.line(
        Vec2::new(x, base),
        Vec2::new(x, base - metres * scale),
        INK,
        0.8,
    );
    let mut m = 0.0;
    while m <= metres + 1e-3 {
        let y = base - m * scale;
        canvas.line(Vec2::new(x - 4.0, y), Vec2::new(x, y), INK, 0.8);
        let label = if step < 1.0 {
            format!("{m:.1}")
        } else {
            format!("{m:.0}")
        };
        canvas.text(font, Vec2::new(x - 34.0, y - 7.0), 12.0, &label, INK);
        m += step;
    }
}
