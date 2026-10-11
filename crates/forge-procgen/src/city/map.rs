//! The plan as a map image and a table of numbers: how a plan is judged before any building
//! exists (`city-generation.md`, stage 5). Water from the site's mask, blocks by district,
//! courtyards, parks, streets by profile, bridges, tram lines with their stops, boat crossings
//! and landmarks, drawn by a small rasteriser at a given scale.

use std::path::Path;

use glam::DVec2;
use image::{ImageBuffer, Rgb};

use super::blocks::BlockUse;
use super::geom;
use super::{CityParams, CityPlan, DistrictKind, LAGOON, LAND, PlanStats, Profile, RIVER, Site};

/// A district kind's colour on the map.
fn district_colour(kind: DistrictKind) -> [u8; 3] {
    match kind {
        DistrictKind::OldTown => [222, 178, 150],
        DistrictKind::Centre => [226, 190, 160],
        DistrictKind::Perimeter => [226, 196, 164],
        DistrictKind::Courtyard => [214, 190, 170],
        DistrictKind::Waterfront => [190, 198, 214],
        DistrictKind::Industrial => [200, 186, 178],
        DistrictKind::Eco => [196, 206, 170],
        DistrictKind::Village => [230, 220, 200],
        DistrictKind::Park => [168, 202, 140],
        DistrictKind::Reserve => [150, 186, 132],
    }
}

const SEA: [u8; 3] = [118, 162, 204];
const RIVER_C: [u8; 3] = [96, 150, 200];
const LAGOON_C: [u8; 3] = [132, 178, 200];
const LAND_C: [u8; 3] = [238, 234, 224];
const BEACH: [u8; 3] = [236, 220, 176];
const PARK: [u8; 3] = [132, 186, 112];
const COURTYARD: [u8; 3] = [222, 228, 204];
const SQUARE: [u8; 3] = [226, 222, 212];
const QUAY: [u8; 3] = [222, 214, 198];

/// An RGB canvas over the plan's frame.
struct Canvas {
    side: u32,
    metres_per_pixel: f64,
    origin: DVec2,
    pixels: Vec<[u8; 3]>,
}

impl Canvas {
    fn new(size: f64, metres_per_pixel: f64) -> Self {
        Self::window(DVec2::ZERO, size, metres_per_pixel)
    }

    /// A canvas over the square of `size` metres whose top-left corner is `origin`.
    fn window(origin: DVec2, size: f64, metres_per_pixel: f64) -> Self {
        let side = (size / metres_per_pixel).ceil() as u32;
        Self {
            side,
            metres_per_pixel,
            origin,
            pixels: vec![LAND_C; (side as usize) * (side as usize)],
        }
    }

    fn put(&mut self, x: i64, y: i64, c: [u8; 3]) {
        if x >= 0 && y >= 0 && (x as u32) < self.side && (y as u32) < self.side {
            self.pixels[y as usize * self.side as usize + x as usize] = c;
        }
    }

    /// Fills a polygon (even-odd), pixel centres inside.
    fn fill(&mut self, polygon: &[DVec2], c: [u8; 3]) {
        let s = self.metres_per_pixel;
        let pts: Vec<DVec2> = polygon.iter().map(|&p| (p - self.origin) / s).collect();
        let (lo, hi) = pts.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let y0 = lo.y.floor().max(0.0) as i64;
        let y1 = (hi.y.ceil() as i64).min(self.side as i64 - 1);
        let n = pts.len();
        let mut xs = Vec::new();
        for y in y0..=y1 {
            let yc = y as f64 + 0.5;
            xs.clear();
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a.y > yc) != (b.y > yc) {
                    xs.push(a.x + (yc - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
            for pair in xs.chunks(2) {
                if let [xa, xb] = pair {
                    let x0 = ((xa - 0.5).ceil() as i64).max(0);
                    let x1 = ((xb - 0.5).floor() as i64).min(self.side as i64 - 1);
                    for x in x0..=x1 {
                        self.put(x, y, c);
                    }
                }
            }
        }
    }

    /// Strokes a polyline `width` metres wide with round joins.
    fn stroke(&mut self, points: &[DVec2], width: f64, c: [u8; 3]) {
        let s = self.metres_per_pixel;
        let half = (width / s * 0.5).max(0.5);
        for w in points.windows(2) {
            let (a, b) = ((w[0] - self.origin) / s, (w[1] - self.origin) / s);
            let lo = a.min(b) - half;
            let hi = a.max(b) + half;
            for y in lo.y.floor() as i64..=hi.y.ceil() as i64 {
                for x in lo.x.floor() as i64..=hi.x.ceil() as i64 {
                    let p = DVec2::new(x as f64 + 0.5, y as f64 + 0.5);
                    if geom::distance_to_segment(p, a, b) <= half {
                        self.put(x, y, c);
                    }
                }
            }
        }
    }

    /// A disc of `radius` metres.
    fn disc(&mut self, centre: DVec2, radius: f64, c: [u8; 3]) {
        self.stroke(&[centre, centre], radius * 2.0, c);
    }

    fn save(&self, path: &Path) -> image::ImageResult<()> {
        let side = self.side;
        ImageBuffer::from_fn(side, side, |x, y| {
            Rgb(self.pixels[y as usize * side as usize + x as usize])
        })
        .save(path)
    }
}

/// A polyline moved `d` metres to its left.
fn offset(points: &[DVec2], d: f64) -> Vec<DVec2> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let prev = if i > 0 {
                points[i] - points[i - 1]
            } else {
                points[1] - points[0]
            };
            let next = if i + 1 < n {
                points[i + 1] - points[i]
            } else {
                prev
            };
            let t = (prev.normalize_or_zero() + next.normalize_or_zero()).normalize_or_zero();
            points[i] + DVec2::new(-t.y, t.x) * d
        })
        .collect()
}

/// What a built block lacks, for the checks map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lack {
    /// Nothing: a park within reach and a tram stop within 400 m.
    None,
    /// No tram stop within 400 m.
    Stop,
    /// No park of the minimum area within reach.
    Park,
    /// Neither.
    Both,
}

/// Each block's lack (squares, parks and quays lack nothing).
pub fn lacks(plan: &CityPlan, params: &CityParams) -> Vec<Lack> {
    let stops: Vec<DVec2> = plan
        .tram_lines
        .iter()
        .flat_map(|l| l.stops.iter().copied())
        .collect();
    let parks: Vec<&super::Block> = plan
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Park && b.area >= params.park_min_area)
        .collect();
    plan.blocks
        .iter()
        .map(|b| {
            if b.use_ != BlockUse::Built {
                return Lack::None;
            }
            let stop = stops.iter().any(|s| s.distance(b.centre) <= 400.0);
            let park = parks
                .iter()
                .any(|p| block_within(b.centre, p, params.park_reach));
            match (stop, park) {
                (true, true) => Lack::None,
                (false, true) => Lack::Stop,
                (true, false) => Lack::Park,
                (false, false) => Lack::Both,
            }
        })
        .collect()
}

/// Whether `p` lies within `reach` of a block.
fn block_within(p: DVec2, block: &super::Block, reach: f64) -> bool {
    if geom::contains(&block.polygon, p) {
        return true;
    }
    let n = block.polygon.len();
    (0..n).any(|i| {
        geom::distance_to_segment(p, block.polygon[i], block.polygon[(i + 1) % n]) <= reach
    })
}

/// The checks as a map: the plan's water, streets and transit over its blocks coloured by what
/// they lack (green: nothing; orange: a tram stop within 400 m; red: a park within reach;
/// purple: both).
pub fn write_checks(
    site: &Site,
    plan: &CityPlan,
    params: &CityParams,
    path: &Path,
    metres_per_pixel: f64,
) -> image::ImageResult<()> {
    let mut canvas = Canvas::new(plan.size, metres_per_pixel);
    for (block, lack) in plan.blocks.iter().zip(lacks(plan, params)) {
        let c = match (block.use_, lack) {
            (BlockUse::Park, _) => PARK,
            (BlockUse::Built, Lack::None) => [176, 214, 160],
            (BlockUse::Built, Lack::Stop) => [240, 170, 80],
            (BlockUse::Built, Lack::Park) => [220, 80, 70],
            (BlockUse::Built, Lack::Both) => [150, 70, 160],
            _ => SQUARE,
        };
        canvas.fill(&block.polygon, c);
    }
    paint_water(&mut canvas, site);
    for e in &plan.roads.edges {
        canvas.stroke(&e.points, e.profile.width(), [250, 248, 244]);
    }
    for line in &plan.tram_lines {
        canvas.stroke(&line.points, 6.0, line.colour);
        for &stop in &line.stops {
            canvas.disc(stop, 14.0, [40, 40, 48]);
            canvas.disc(stop, 9.0, [255, 255, 255]);
        }
    }
    canvas.save(path)
}

/// Draws the plan over its site at `metres_per_pixel` and writes it as a PNG.
pub fn write(
    site: &Site,
    plan: &CityPlan,
    path: &Path,
    metres_per_pixel: f64,
) -> image::ImageResult<()> {
    write_labelled(site, plan, path, metres_per_pixel, &[], None)
}

/// [`write`] with names drawn over the map in `font` (a TTF's bytes; none without one).
pub fn write_labelled(
    site: &Site,
    plan: &CityPlan,
    path: &Path,
    metres_per_pixel: f64,
    labels: &[Label],
    font: Option<&[u8]>,
) -> image::ImageResult<()> {
    let mut canvas = Canvas::new(plan.size, metres_per_pixel);
    let s = metres_per_pixel;
    paint_blocks(&mut canvas, plan);
    paint_water(&mut canvas, site);
    // Streets: outlines first, then fills, the narrow profiles under the wide ones.
    let mut edges: Vec<&super::RoadEdge> = plan.roads.edges.iter().collect();
    edges.sort_by_key(|e| std::cmp::Reverse(e.profile));
    for e in &edges {
        let outline = if e.bridge {
            [70, 70, 78]
        } else {
            [168, 160, 150]
        };
        canvas.stroke(&e.points, e.profile.width() + 2.0 * s, outline);
    }
    for e in &edges {
        let fill = match e.profile {
            Profile::TramAvenue => [252, 252, 250],
            Profile::Avenue => [250, 246, 238],
            Profile::Shared => [244, 238, 228],
            Profile::Pedestrian => [240, 224, 196],
            Profile::Path => [214, 204, 170],
        };
        canvas.stroke(&e.points, e.profile.width(), fill);
    }
    // Tram lines side by side, their stops, the boats, the landmarks.
    for (k, line) in plan.tram_lines.iter().enumerate() {
        let slot = (k % 3) as f64 - 1.0;
        canvas.stroke(&offset(&line.points, slot * 6.0), 5.0, line.colour);
    }
    for line in &plan.tram_lines {
        for &stop in &line.stops {
            canvas.disc(stop, 13.0, [40, 40, 48]);
            canvas.disc(stop, 9.0, [255, 255, 255]);
        }
    }
    for crossing in &plan.crossings {
        let length = crossing.a.distance(crossing.b);
        let dashes = (length / 12.0).ceil() as usize;
        for i in (0..dashes).step_by(2) {
            let t0 = i as f64 / dashes as f64;
            let t1 = ((i + 1) as f64 / dashes as f64).min(1.0);
            let (p, q) = (
                crossing.a.lerp(crossing.b, t0),
                crossing.a.lerp(crossing.b, t1),
            );
            canvas.stroke(&[p, q], 5.0, [24, 48, 120]);
        }
        canvas.disc(crossing.a, 9.0, [24, 48, 120]);
        canvas.disc(crossing.b, 9.0, [24, 48, 120]);
    }
    for landmark in &plan.landmarks {
        let r = 45.0;
        let p = landmark.pos;
        let diamond = [
            p + DVec2::new(0.0, -r),
            p + DVec2::new(r, 0.0),
            p + DVec2::new(0.0, r),
            p + DVec2::new(-r, 0.0),
        ];
        canvas.fill(&diamond, [196, 30, 40]);
    }
    if let Some(font) = font.and_then(|bytes| ab_glyph::FontRef::try_from_slice(bytes).ok()) {
        let mut placed: Vec<[f32; 4]> = Vec::new();
        for label in labels {
            canvas.text(&font, label, &mut placed);
        }
    }
    canvas.save(path)
}

/// A name drawn on a map, centred on its place.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// Its place, metres on the plan.
    pub pos: DVec2,
    /// What it says.
    pub text: String,
    /// Its height, pixels.
    pub px: f32,
    /// Its colour.
    pub colour: [u8; 3],
    /// The halo's colour round it.
    pub halo: [u8; 3],
}

/// The plan's own names: its districts at their anchors (those on land in the frame), its
/// landmarks beside their marks, and each tram line at its first stop.
pub fn labels(plan: &CityPlan) -> Vec<Label> {
    let mut out = Vec::new();
    for d in &plan.districts {
        let inside = d.anchor.min_element() >= 0.0 && d.anchor.max_element() <= plan.size;
        if inside
            && plan
                .district_at(d.anchor)
                .is_some_and(|at| at.name == d.name)
        {
            out.push(Label {
                pos: d.anchor,
                text: d.name.clone(),
                px: 30.0,
                colour: [60, 50, 44],
                halo: [255; 3],
            });
        }
    }
    for l in &plan.landmarks {
        out.push(Label {
            pos: l.pos + DVec2::new(0.0, 90.0),
            text: l.name.clone(),
            px: 24.0,
            colour: [150, 20, 30],
            halo: [255; 3],
        });
    }
    for line in &plan.tram_lines {
        if let Some(&first) = line.stops.first() {
            out.push(Label {
                pos: first + DVec2::new(0.0, -60.0),
                text: line.name.clone(),
                px: 20.0,
                colour: line.colour,
                halo: [255; 3],
            });
        }
    }
    out
}

impl Canvas {
    /// Blends `c` into a pixel by `coverage`.
    fn blend(&mut self, x: i64, y: i64, c: [u8; 3], coverage: f32) {
        if x < 0 || y < 0 || x as u32 >= self.side || y as u32 >= self.side {
            return;
        }
        let p = &mut self.pixels[y as usize * self.side as usize + x as usize];
        for k in 0..3 {
            let a = f32::from(p[k]);
            p[k] = (a + (f32::from(c[k]) - a) * coverage.clamp(0.0, 1.0)).round() as u8;
        }
    }

    /// A label centred on its place, over a white halo. One that would overlap a label already
    /// drawn (`placed`, pixel boxes) moves down a line, up to three times, or is left out.
    fn text(&mut self, font: &ab_glyph::FontRef, label: &Label, placed: &mut Vec<[f32; 4]>) {
        use ab_glyph::{Font, PxScale, ScaleFont, point};
        let units_per_em = font.units_per_em().unwrap_or(1000.0);
        let scale = PxScale::from(label.px * font.height_unscaled() / units_per_em);
        let scaled = font.as_scaled(scale);
        let width: f32 = label
            .text
            .chars()
            .map(|c| scaled.h_advance(scaled.glyph_id(c)))
            .sum();
        let centre = (label.pos - self.origin) / self.metres_per_pixel;
        let left = centre.x as f32 - width * 0.5;
        let height = scaled.height();
        let mut top = centre.y as f32 - height * 0.5;
        let overlaps = |top: f32, placed: &[[f32; 4]]| {
            placed
                .iter()
                .any(|b| left < b[2] && left + width > b[0] && top < b[3] && top + height > b[1])
        };
        let mut tries = 0;
        while overlaps(top, placed) {
            tries += 1;
            if tries > 3 {
                return;
            }
            top += height;
        }
        placed.push([left, top, left + width, top + height]);
        let mut coverage = Vec::new();
        let mut x = left;
        for c in label.text.chars() {
            let mut glyph = scaled.scaled_glyph(c);
            glyph.position = point(x, top + scaled.ascent());
            x += scaled.h_advance(glyph.id);
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, v| {
                    coverage.push((
                        bounds.min.x as i64 + i64::from(gx),
                        bounds.min.y as i64 + i64::from(gy),
                        v,
                    ));
                });
            }
        }
        for &(px, py, v) in &coverage {
            for (dx, dy) in [
                (-2, 0),
                (2, 0),
                (0, -2),
                (0, 2),
                (-1, -1),
                (1, 1),
                (-1, 1),
                (1, -1),
            ] {
                self.blend(px + dx, py + dy, label.halo, v * 0.85);
            }
        }
        for &(px, py, v) in &coverage {
            self.blend(px, py, label.colour, v);
        }
    }
}

/// Water and beaches from the site's mask, over the blocks (a quay's face holds its river).
fn paint_water(canvas: &mut Canvas, site: &Site) {
    let s = canvas.metres_per_pixel;
    let water = &site.water;
    let beach_cells = (40.0 / water.spacing).ceil() as i64;
    for y in 0..canvas.side {
        for x in 0..canvas.side {
            let p = canvas.origin + DVec2::new((x as f64 + 0.5) * s, (y as f64 + 0.5) * s);
            let c = match site.water_at(p) {
                LAND => continue,
                RIVER => RIVER_C,
                LAGOON => LAGOON_C,
                _ => SEA,
            };
            canvas.pixels[y as usize * canvas.side as usize + x as usize] = c;
        }
    }
    // Sand along the sea: land cells within 40 m of it.
    let n = water.size as i64;
    for cy in 0..n {
        for cx in 0..n {
            if water.get(cx as u32, cy as u32) != LAND {
                continue;
            }
            let near_sea = (-beach_cells..=beach_cells).any(|dy| {
                (-beach_cells..=beach_cells).any(|dx| {
                    let (x, y) = (cx + dx, cy + dy);
                    x >= 0
                        && y >= 0
                        && x < n
                        && y < n
                        && water.get(x as u32, y as u32) == super::SEA
                })
            });
            if near_sea {
                let p = DVec2::new(cx as f64, cy as f64) * water.spacing;
                let half = water.spacing * 0.5;
                canvas.fill(
                    &[
                        p + DVec2::new(-half, -half),
                        p + DVec2::new(half, -half),
                        p + DVec2::new(half, half),
                        p + DVec2::new(-half, half),
                    ],
                    BEACH,
                );
            }
        }
    }
}

/// Blocks by use and district, the lots' outlines, then the courtyards over them.
fn paint_blocks(canvas: &mut Canvas, plan: &CityPlan) {
    let s = canvas.metres_per_pixel;
    for block in &plan.blocks {
        let c = match block.use_ {
            BlockUse::Built => district_colour(plan.districts[block.district].kind),
            BlockUse::Park => PARK,
            BlockUse::Square => SQUARE,
            BlockUse::Quay => QUAY,
        };
        canvas.fill(&block.polygon, c);
    }
    for lot in &plan.lots {
        let block = &plan.blocks[lot.block as usize];
        let base = district_colour(plan.districts[block.district].kind);
        let edge = base.map(|v| (f64::from(v) * 0.82) as u8);
        let mut ring = lot.polygon.clone();
        ring.push(lot.polygon[0]);
        canvas.stroke(&ring, 0.6 * s, edge);
    }
    for block in &plan.blocks {
        if let Some(courtyard) = &block.courtyard {
            canvas.fill(courtyard, COURTYARD);
        }
    }
}

/// The plan's numbers.
pub fn stats(site: &Site, plan: &CityPlan, params: &CityParams) -> PlanStats {
    let land = site.land_area();
    let mut st = PlanStats {
        land_km2: land / 1e6,
        nodes: plan.roads.nodes.len(),
        ..PlanStats::default()
    };
    let mut street_area = 0.0;
    for e in &plan.roads.edges {
        let length = e.length();
        match e.class {
            super::RoadClass::Main => st.main_km += length / 1000.0,
            super::RoadClass::Street => st.street_km += length / 1000.0,
        }
        st.bridges += usize::from(e.bridge);
        street_area += length * e.profile.width();
    }
    st.street_share = street_area / land;
    let built: Vec<&super::Block> = plan
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Built)
        .collect();
    let parks: Vec<&super::Block> = plan
        .blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Park)
        .collect();
    st.built_blocks = built.len();
    st.park_blocks = parks.len();
    st.mean_block_m2 = built.iter().map(|b| b.area).sum::<f64>() / built.len().max(1) as f64;
    st.park_share = parks.iter().map(|b| b.area).sum::<f64>() / land;
    st.lots = plan.lots.len();
    st.courtyards = plan.blocks.iter().filter(|b| b.courtyard.is_some()).count();
    let big_parks: Vec<&&super::Block> = parks
        .iter()
        .filter(|p| p.area >= params.park_min_area)
        .collect();
    let near = |p: DVec2, block: &super::Block, reach: f64| {
        if geom::contains(&block.polygon, p) {
            return true;
        }
        let n = block.polygon.len();
        (0..n).any(|i| {
            geom::distance_to_segment(p, block.polygon[i], block.polygon[(i + 1) % n]) <= reach
        })
    };
    let covered = built
        .iter()
        .filter(|b| {
            big_parks
                .iter()
                .any(|p| near(b.centre, p, params.park_reach))
        })
        .count();
    st.park_coverage = covered as f64 / built.len().max(1) as f64;
    let stops: Vec<DVec2> = plan
        .tram_lines
        .iter()
        .flat_map(|l| l.stops.iter().copied())
        .collect();
    let served = built
        .iter()
        .filter(|b| stops.iter().any(|s| s.distance(b.centre) <= 400.0))
        .count();
    st.stop_coverage = served as f64 / built.len().max(1) as f64;
    st.tram_km = plan
        .tram_lines
        .iter()
        .map(|l| geom::polyline_length(&l.points))
        .sum::<f64>()
        / 1000.0;
    st.stops = stops.len();
    st.crossings = plan.crossings.len();
    st.digest = digest(plan);
    st
}

/// A 64-bit FNV-1a digest of the plan's records (D-016).
pub fn digest(plan: &CityPlan) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |v: u64| {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for n in &plan.roads.nodes {
        eat(n.pos.x.to_bits());
        eat(n.pos.y.to_bits());
    }
    for e in &plan.roads.edges {
        eat(u64::from(e.a) << 32 | u64::from(e.b));
        eat(e.profile as u64);
    }
    for b in &plan.blocks {
        eat(b.area.to_bits());
        eat(b.use_ as u64);
    }
    for l in &plan.lots {
        eat(l.area.to_bits());
    }
    for t in &plan.tram_lines {
        for s in &t.stops {
            eat(s.x.to_bits());
            eat(s.y.to_bits());
        }
    }
    h
}

/// The masses in an oblique view from the south, a tilt of `tilt` screen metres up per metre of
/// height, over the plan's ground: walls facing the viewer shaded by their direction, roofs
/// coloured by what they carry (green, solar, terrace).
pub fn write_massing(
    site: &Site,
    plan: &CityPlan,
    masses: &[super::mass::Mass],
    path: &Path,
    window: (DVec2, f64),
    metres_per_pixel: f64,
    tilt: f64,
) -> image::ImageResult<()> {
    use super::mass::Roof;
    let mut canvas = Canvas::window(window.0, window.1, metres_per_pixel);
    // The ground: courtyards and gardens, parks, squares, water, then the streets.
    for block in &plan.blocks {
        let c = match block.use_ {
            BlockUse::Park => PARK,
            BlockUse::Quay => QUAY,
            BlockUse::Square => SQUARE,
            BlockUse::Built => COURTYARD,
        };
        canvas.fill(&block.polygon, c);
    }
    paint_water(&mut canvas, site);
    for e in &plan.roads.edges {
        canvas.stroke(&e.points, e.profile.width(), [236, 232, 224]);
    }
    for line in &plan.tram_lines {
        canvas.stroke(&line.points, 2.0, line.colour);
    }
    // Far masses first: the viewer stands to the south (+y).
    let mut order: Vec<&super::mass::Mass> = masses.iter().collect();
    order.sort_by(|a, b| {
        let ya = a.footprint.iter().fold(f64::MIN, |m, p| m.max(p.y));
        let yb = b.footprint.iter().fold(f64::MIN, |m, p| m.max(p.y));
        ya.partial_cmp(&yb).expect("finite")
    });
    let light = DVec2::new(-0.6, 0.8);
    for m in order {
        let lot = &plan.lots[m.lot as usize];
        let kind = plan.districts[plan.blocks[lot.block as usize].district].kind;
        let base = district_colour(kind).map(|v| f64::from(v).mul_add(0.5, 127.0));
        let lift = DVec2::new(0.0, -m.height * tilt);
        let n = m.footprint.len();
        for i in 0..n {
            let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
            let t = (b - a).normalize_or_zero();
            let outward = DVec2::new(t.y, -t.x);
            if outward.y <= 0.0 {
                continue;
            }
            let shade = 0.62 + 0.38 * outward.dot(light).max(0.0);
            let c = base.map(|v| (v * shade).min(255.0) as u8);
            canvas.fill(&[a, b, b + lift, a + lift], c);
        }
        let roof: Vec<DVec2> = m.footprint.iter().map(|&p| p + lift).collect();
        let c = match m.roof {
            Roof::Green => [126, 172, 98],
            Roof::Solar => [70, 82, 104],
            Roof::Terrace => [214, 206, 192],
        };
        canvas.fill(&roof, c);
        let mut ring = roof.clone();
        ring.push(roof[0]);
        canvas.stroke(&ring, 0.5 * metres_per_pixel, [96, 90, 84]);
    }
    canvas.save(path)
}

/// People (or anything) as points over a dark version of the plan: each point brightens its pixel
/// by its colour, so crowds glow; a caption in `font` at the top left.
pub fn write_points(
    site: &Site,
    plan: &CityPlan,
    points: &[([f32; 2], [u8; 3])],
    path: &Path,
    metres_per_pixel: f64,
    caption: Option<(&str, &[u8])>,
) -> image::ImageResult<()> {
    let mut canvas = Canvas::new(plan.size, metres_per_pixel);
    canvas.pixels.fill([16, 18, 24]);
    for block in &plan.blocks {
        let c = match block.use_ {
            BlockUse::Built => [32, 34, 40],
            BlockUse::Park => [20, 36, 24],
            _ => [26, 27, 32],
        };
        canvas.fill(&block.polygon, c);
    }
    let s = metres_per_pixel;
    for y in 0..canvas.side {
        for x in 0..canvas.side {
            let p = DVec2::new((x as f64 + 0.5) * s, (y as f64 + 0.5) * s);
            if site.water_at(p) != LAND {
                canvas.pixels[y as usize * canvas.side as usize + x as usize] = [10, 18, 30];
            }
        }
    }
    for e in &plan.roads.edges {
        canvas.stroke(&e.points, e.profile.width(), [40, 40, 46]);
    }
    for &(pos, c) in points {
        let x = (f64::from(pos[0]) / s) as i64;
        let y = (f64::from(pos[1]) / s) as i64;
        if x < 0 || y < 0 || x as u32 >= canvas.side || y as u32 >= canvas.side {
            continue;
        }
        let px = &mut canvas.pixels[y as usize * canvas.side as usize + x as usize];
        for k in 0..3 {
            px[k] = px[k].saturating_add(c[k] / 3);
        }
    }
    if let Some((text, bytes)) = caption
        && let Ok(font) = ab_glyph::FontRef::try_from_slice(bytes)
    {
        let label = Label {
            pos: DVec2::new(plan.size * 0.5, 40.0 * s),
            text: text.to_string(),
            px: 40.0,
            colour: [235, 235, 235],
            halo: [16, 18, 24],
        };
        canvas.text(&font, &label, &mut Vec::new());
    }
    canvas.save(path)
}
