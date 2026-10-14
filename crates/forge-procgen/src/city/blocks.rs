//! Blocks, parks and lots. A block is a bounded face of the road graph on land; its district
//! is the one at its centre. Parks come first from the green districts, then from the
//! 3-30-300 rule (`city-generation.md` §10): a built block without a park of at least
//! `min_area` within `reach` gets the best-placed block near it turned into one, until every
//! block has one or the parks reach their share of the land. Lots are cut by recursive splits
//! across each piece's long axis (Vanegas et al. 2012's oriented boxes); the pieces that
//! touch no street stay inside as courtyards and gardens.

use forge_core::SplitMix64;
use glam::DVec2;

use super::geom::{self, PointGrid};
use super::graph::RoadGraph;
use super::{DistrictKind, DistrictSpec, LAND, Site, district_index};
use crate::field::Field2;

/// What a block holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockUse {
    /// Buildings round a courtyard.
    Built,
    /// A park.
    Park,
    /// A square: a sliver too small to build on.
    Square,
    /// The land between the streets and the water: a quay or a promenade.
    Quay,
}

/// A block of the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Its outline along the streets' centrelines.
    pub polygon: Vec<DVec2>,
    /// Its area, m².
    pub area: f64,
    /// A point inside it.
    pub centre: DVec2,
    /// Its district's index.
    pub district: usize,
    /// What it holds.
    pub use_: BlockUse,
    /// Its courtyard: the outline moved in by the street and a building's depth, when the
    /// block is deep enough for one.
    pub courtyard: Option<Vec<DVec2>>,
    /// What can be built on: a built block's outline moved in to the building line of each
    /// street round it (half the street's width), where its lots are cut. The outline itself
    /// for the other blocks, and where the inset fails.
    pub buildable: Vec<DVec2>,
}

/// A lot of a built block.
#[derive(Clone, Debug, PartialEq)]
pub struct Lot {
    /// Its block's index.
    pub block: u32,
    /// Its outline.
    pub polygon: Vec<DVec2>,
    /// Its area, m².
    pub area: f64,
    /// Whether it touches a street (else a courtyard or garden).
    pub frontage: bool,
}

/// Blocks from the road graph's faces: those on land, with their district.
pub fn from_faces(
    site: &Site,
    faces: &[Vec<DVec2>],
    districts_field: &Field2<u8>,
    districts: &[DistrictSpec],
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for face in faces {
        let polygon = geom::simplify_closed(face, 0.5);
        if polygon.len() < 3 {
            continue;
        }
        let area = geom::signed_area(&polygon);
        // A face that holds water is the land between the streets and a river or the sea:
        // a quay. Its district is the one at its first land sample.
        let (water_share, land_point) = water_share(site, &polygon);
        let inside = inside_point(&polygon);
        let Some(centre) = (if site.water_at(inside) == LAND {
            Some(inside)
        } else {
            land_point
        }) else {
            continue;
        };
        let Some(district) = district_index(districts_field, centre) else {
            continue;
        };
        let kind = districts[district].kind;
        let use_ = if water_share > 0.2 {
            BlockUse::Quay
        } else if kind.is_green() {
            BlockUse::Park
        } else if area < 600.0 {
            BlockUse::Square
        } else {
            BlockUse::Built
        };
        // Courtyards come once the streets' profiles are known ([`courtyards`]).
        let courtyard = None;
        let _ = kind;
        blocks.push(Block {
            buildable: polygon.clone(),
            polygon,
            area,
            centre,
            district,
            use_,
            courtyard,
        });
    }
    blocks
}

/// Half a street's width plus its pavement, from a block's outline on the streets'
/// centrelines to its building line, metres, where the street's own profile is not known.
pub const STREET_HALF: f64 = 6.0;

/// The streets' centrelines sampled every 10 m, each sample tagged with its edge.
pub fn street_grid(roads: &RoadGraph, size: f64) -> PointGrid {
    let mut grid = PointGrid::new(size, 50.0);
    for (i, e) in roads.edges.iter().enumerate() {
        for p in geom::resample(&e.points, 10.0) {
            grid.insert(p, i as u32);
        }
    }
    grid
}

/// Half the width of the street along a block's edge from `a` to `b` (its building line's
/// distance from the centreline), [`STREET_HALF`] when no street is found.
pub fn edge_half_width(grid: &PointGrid, roads: &RoadGraph, a: DVec2, b: DVec2) -> f64 {
    grid.nearest((a + b) * 0.5, 30.0)
        .map_or(STREET_HALF, |(_, e)| {
            roads.edges[e as usize].profile.width() * 0.5
        })
}

/// The smallest courtyard kept, square metres: a small room's floor and a little.
const MIN_COURT: f64 = 40.0;

/// Each built block's and square's buildable outline on the building lines, and each built
/// block's courtyard: its outline moved in by half the widest street round it and its
/// district's building depth, when the block is deep enough for one.
pub fn courtyards(
    blocks: &mut [Block],
    roads: &RoadGraph,
    ground: &super::ground::GroundMap,
    districts: &[DistrictSpec],
    size: f64,
) {
    let grid = street_grid(roads, size);
    for b in blocks.iter_mut() {
        if !matches!(b.use_, BlockUse::Built | BlockUse::Square) {
            continue;
        }
        let n = b.polygon.len();
        let halves: Vec<f64> = (0..n)
            .map(|i| edge_half_width(&grid, roads, b.polygon[i], b.polygon[(i + 1) % n]))
            .collect();
        let half = halves.iter().copied().fold(STREET_HALF, f64::max);
        // The building lines: the pavements' own back edge, what the street ground leaves of the
        // face (D-061), so a building's front stands on it. Moved in by a width read from the
        // nearest street, they missed it here and there, a strip of garden between the paving
        // and the wall (the owner: "buildings have no foundations"). The inset where the map
        // leaves nothing whole.
        let least = halves.iter().copied().fold(f64::MAX, f64::min);
        let lines = ground
            .off_streets(&b.polygon)
            .into_iter()
            .filter_map(|shape| shape.into_iter().next())
            .map(|outer| geom::clean(&geom::simplify_closed(&outer, 0.05), 0.01))
            .filter(|p| p.len() >= 3)
            .max_by(|a, b| {
                geom::signed_area(a)
                    .abs()
                    .total_cmp(&geom::signed_area(b).abs())
            })
            .filter(|p| geom::signed_area(p).abs() > 0.2 * b.area)
            .map(|mut p| {
                if geom::signed_area(&p) < 0.0 {
                    p.reverse();
                }
                p
            })
            .or_else(|| geom::inset_by(&b.polygon, &halves))
            .or_else(|| geom::inset_by_collapsing(&b.polygon, &halves));
        if let Some(inside) = lines.clone().or_else(|| geom::inset(&b.polygon, least)) {
            b.buildable = inside;
        }
        let Some(depth) = districts[b.district]
            .building_depth()
            .filter(|_| b.use_ == BlockUse::Built)
        else {
            continue;
        };
        // A building's depth behind the building lines, or behind the widest street's.
        // A court smaller than a room's floor is none: the block is built through (an 8 m²
        // court was left that no one could walk into).
        b.courtyard = match &lines {
            // The building lines' inset, else the face's by the widest street: the map's
            // detailed lines can fold where the plain inset holds.
            Some(inside) => {
                geom::inset(inside, depth).or_else(|| geom::inset(&b.polygon, half + depth))
            }
            None => geom::inset(&b.polygon, half + depth),
        }
        .filter(|c| geom::signed_area(c).abs() >= MIN_COURT);
    }
}

/// The share of a polygon's area over water, sampled every 10 m, and a land point of it.
fn water_share(site: &Site, polygon: &[DVec2]) -> (f64, Option<DVec2>) {
    let (lo, hi) = polygon.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let step = 10.0;
    let (mut inside, mut wet) = (0_u32, 0_u32);
    let mut land = None;
    let mut y = lo.y + step * 0.5;
    while y < hi.y {
        let mut x = lo.x + step * 0.5;
        while x < hi.x {
            let p = DVec2::new(x, y);
            if geom::contains(polygon, p) {
                inside += 1;
                if site.water_at(p) == LAND {
                    land.get_or_insert(p);
                } else {
                    wet += 1;
                }
            }
            x += step;
        }
        y += step;
    }
    (f64::from(wet) / f64::from(inside.max(1)), land)
}

/// A point inside a polygon: its centroid if inside, else the middle of the widest horizontal
/// chord through its centroid's height.
fn inside_point(polygon: &[DVec2]) -> DVec2 {
    let c = geom::centroid(polygon);
    if geom::contains(polygon, c) {
        return c;
    }
    let n = polygon.len();
    let mut xs = Vec::new();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > c.y) != (b.y > c.y) {
            xs.push(a.x + (c.y - a.y) / (b.y - a.y) * (b.x - a.x));
        }
    }
    xs.sort_by(|x, y| x.partial_cmp(y).expect("finite"));
    let mut best = (0.0, c);
    for pair in xs.chunks(2) {
        if let [x0, x1] = pair
            && x1 - x0 > best.0
        {
            best = (x1 - x0, DVec2::new((x0 + x1) * 0.5, c.y));
        }
    }
    best.1
}

/// The distance from `p` to a block (0 inside).
fn distance_to_block(p: DVec2, block: &Block) -> f64 {
    if geom::contains(&block.polygon, p) {
        return 0.0;
    }
    let n = block.polygon.len();
    (0..n)
        .map(|i| geom::distance_to_segment(p, block.polygon[i], block.polygon[(i + 1) % n]))
        .fold(f64::MAX, f64::min)
}

/// Squares (#245): the built block nearest each anchor (a district's centre, a landmark), within
/// 150 m and within the anchor's range of areas (m²), becomes a square unless one already opens
/// within 120 m; and in old
/// towns, blocks of 600–3 000 m² at least `spacing` metres from any other square, the small
/// squares of a Mediterranean town. Returns how many blocks became squares.
pub fn add_squares(
    blocks: &mut [Block],
    anchors: &[(DVec2, (f64, f64))],
    districts: &[DistrictSpec],
    spacing: f64,
) -> usize {
    let mut made = 0;
    for &(anchor, (smallest, largest)) in anchors {
        // One square to a place: none where one already opens within 120 m.
        let near_one = blocks.iter().any(|b| {
            b.use_ == BlockUse::Square && b.area >= 600.0 && b.centre.distance(anchor) < 120.0
        });
        if near_one {
            continue;
        }
        let nearest = blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.use_ == BlockUse::Built && (smallest..=largest).contains(&b.area))
            .map(|(i, b)| {
                let d = if geom::contains(&b.polygon, anchor) {
                    0.0
                } else {
                    b.centre.distance(anchor)
                };
                (d, i)
            })
            .filter(|&(d, _)| d <= 150.0)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if let Some((_, i)) = nearest {
            blocks[i].use_ = BlockUse::Square;
            blocks[i].courtyard = None;
            made += 1;
        }
    }
    let mut squares: Vec<DVec2> = blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Square && b.area >= 600.0)
        .map(|b| b.centre)
        .collect();
    for b in blocks.iter_mut() {
        if b.use_ != BlockUse::Built
            || districts[b.district].kind != DistrictKind::OldTown
            || !(600.0..=3_000.0).contains(&b.area)
            || squares.iter().any(|s| s.distance(b.centre) < spacing)
        {
            continue;
        }
        b.use_ = BlockUse::Square;
        b.courtyard = None;
        squares.push(b.centre);
        made += 1;
    }
    made
}

/// Turns blocks into parks until every built block has a park of `min_area` within `reach`
/// (or the parks hold `share_cap` of the blocks' area).
pub fn add_parks(blocks: &mut [Block], reach: f64, min_area: f64, share_cap: f64) {
    let n = blocks.len();
    let total: f64 = blocks.iter().map(|b| b.area).sum();
    let mut park_area: f64 = blocks
        .iter()
        .filter(|b| b.use_ == BlockUse::Park)
        .map(|b| b.area)
        .sum();
    // Each block's centre in a grid, and its radius (centre to farthest corner).
    let extent = blocks
        .iter()
        .flat_map(|b| b.polygon.iter())
        .fold(0.0_f64, |m, p| m.max(p.x).max(p.y));
    let mut grid = PointGrid::new(extent + 1.0, reach.max(50.0));
    for (i, b) in blocks.iter().enumerate() {
        grid.insert(b.centre, i as u32);
    }
    let radius: Vec<f64> = blocks
        .iter()
        .map(|b| {
            b.polygon
                .iter()
                .map(|p| p.distance(b.centre))
                .fold(0.0, f64::max)
        })
        .collect();
    let max_radius = radius.iter().copied().fold(0.0, f64::max);
    // The blocks whose centre lies within reach of block `j`.
    let served_by = |blocks: &[Block], j: usize| -> Vec<usize> {
        let mut out = Vec::new();
        grid.for_each_within(blocks[j].centre, reach + radius[j], |_, k| {
            if distance_to_block(blocks[k as usize].centre, &blocks[j]) <= reach {
                out.push(k as usize);
            }
        });
        out.sort_unstable();
        out
    };
    let mut covered = vec![false; n];
    for j in 0..n {
        if blocks[j].use_ == BlockUse::Park && blocks[j].area >= min_area {
            for k in served_by(blocks, j) {
                covered[k] = true;
            }
        }
    }
    for i in 0..n {
        if blocks[i].use_ != BlockUse::Built || covered[i] {
            continue;
        }
        if park_area > share_cap * total {
            break;
        }
        // The built block of a park's size within reach that would serve the most uncovered
        // blocks; ties to the lower index.
        let mut candidates = Vec::new();
        grid.for_each_within(blocks[i].centre, reach + max_radius, |_, j| {
            candidates.push(j as usize);
        });
        candidates.sort_unstable();
        let mut best: Option<(usize, usize)> = None;
        for j in candidates {
            let candidate = &blocks[j];
            if candidate.use_ != BlockUse::Built
                || candidate.area < min_area
                || candidate.area > 6.0 * min_area
                || distance_to_block(blocks[i].centre, candidate) > reach
            {
                continue;
            }
            let serves = served_by(blocks, j)
                .into_iter()
                .filter(|&k| blocks[k].use_ == BlockUse::Built && !covered[k])
                .count();
            if best.is_none_or(|(_, s)| serves > s) {
                best = Some((j, serves));
            }
        }
        if let Some((j, _)) = best {
            blocks[j].use_ = BlockUse::Park;
            blocks[j].courtyard = None;
            park_area += blocks[j].area;
            for k in served_by(blocks, j) {
                covered[k] = true;
            }
        }
    }
}

/// Cuts the built blocks into lots.
pub fn subdivide(blocks: &[Block], districts: &[DistrictSpec], seed: u64) -> Vec<Lot> {
    let mut lots = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if block.use_ != BlockUse::Built {
            continue;
        }
        let district = &districts[block.district];
        let target = district.lot_area;
        let mut rng = SplitMix64::new(seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut stack = vec![block.buildable.clone()];
        let mut pieces = Vec::new();
        while let Some(piece) = stack.pop() {
            let area = geom::signed_area(&piece).abs();
            let (axis, extent, centre) = geom::oriented_extent(&piece);
            if area < target * 1.6 || extent.y < 14.0 && extent.x < 28.0 {
                pieces.push(piece);
                continue;
            }
            // Split across the longer side, near its middle.
            let (along, normal) = if extent.x >= extent.y {
                (axis, DVec2::new(-axis.y, axis.x))
            } else {
                (DVec2::new(-axis.y, axis.x), axis)
            };
            let span = extent.x.max(extent.y);
            let offset = rng.range_f64(-0.1, 0.1) * span;
            let origin = centre + along * offset;
            let a = geom::clip_half_plane(&piece, origin, normal);
            let b = geom::clip_half_plane(&piece, origin, -normal);
            if a.len() < 3 || b.len() < 3 {
                pieces.push(piece);
                continue;
            }
            stack.push(a);
            stack.push(b);
        }
        // A block with a courtyard keeps only the lots on its streets: the inner pieces are the
        // courtyard. Without one (halls, thin blocks) the inner pieces stay as lots reached
        // through the others.
        for piece in pieces {
            let frontage = touches_boundary(&piece, &block.buildable);
            if !frontage && block.courtyard.is_some() {
                continue;
            }
            let area = geom::signed_area(&piece).abs();
            lots.push(Lot {
                block: i as u32,
                polygon: piece,
                area,
                frontage,
            });
        }
    }
    lots
}

/// Whether two consecutive vertices of `piece` lie on the block's outline.
fn touches_boundary(piece: &[DVec2], outline: &[DVec2]) -> bool {
    let on = |p: DVec2| {
        let n = outline.len();
        (0..n).any(|i| geom::distance_to_segment(p, outline[i], outline[(i + 1) % n]) < 0.5)
    };
    let n = piece.len();
    (0..n).any(|i| on(piece[i]) && on(piece[(i + 1) % n]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Vec<DVec2> {
        vec![
            DVec2::new(x, y),
            DVec2::new(x + side, y),
            DVec2::new(x + side, y + side),
            DVec2::new(x, y + side),
        ]
    }

    #[test]
    fn lots_cover_their_block_and_inner_ones_are_courtyards() {
        let polygon = square(0.0, 0.0, 120.0);
        let mut block = Block {
            courtyard: geom::inset(&polygon, STREET_HALF + 14.0),
            buildable: polygon.clone(),
            polygon,
            area: 14_400.0,
            centre: DVec2::splat(60.0),
            district: 0,
            use_: BlockUse::Built,
        };
        let courtyard = block
            .courtyard
            .clone()
            .expect("a 120 m block has a courtyard");
        assert!((geom::signed_area(&courtyard) - 80.0 * 80.0).abs() < 1e-6);
        let district = DistrictSpec::test(DVec2::ZERO, 1000.0, super::super::Pattern::Grid, 0.0);
        let lots = subdivide(
            std::slice::from_ref(&block),
            std::slice::from_ref(&district),
            3,
        );
        assert!(
            lots.iter().all(|l| l.frontage),
            "the middle is the courtyard"
        );
        assert!(lots.iter().all(|l| l.area < 400.0 * 1.6));
        // Without a courtyard every piece is a lot and they tile the block.
        block.courtyard = None;
        let lots = subdivide(&[block], &[district], 3);
        let total: f64 = lots.iter().map(|l| l.area).sum();
        assert!((total - 14_400.0).abs() < 1e-6, "{total}");
        assert!(lots.iter().any(|l| !l.frontage));
    }

    #[test]
    fn squares_open_at_the_anchors_and_space_out_in_old_towns() {
        let mut blocks: Vec<Block> = (0..10)
            .map(|i| Block {
                polygon: square(i as f64 * 50.0, 0.0, 45.0),
                area: 2025.0,
                centre: DVec2::new(i as f64 * 50.0 + 22.5, 22.5),
                district: 0,
                use_: BlockUse::Built,
                courtyard: None,
                buildable: square(i as f64 * 50.0, 0.0, 45.0),
            })
            .collect();
        let mut district =
            DistrictSpec::test(DVec2::ZERO, 1000.0, super::super::Pattern::Grid, 0.0);
        district.kind = DistrictKind::OldTown;
        let made = add_squares(
            &mut blocks,
            &[(DVec2::new(122.0, 20.0), (0.0, 10_000.0))],
            std::slice::from_ref(&district),
            180.0,
        );
        // The anchor's block, then one in four blocks of the old town (180 m apart, 50 m a block).
        assert_eq!(blocks[2].use_, BlockUse::Square);
        let squares: Vec<usize> = (0..10)
            .filter(|&i| blocks[i].use_ == BlockUse::Square)
            .collect();
        assert_eq!(made, squares.len());
        for w in squares.windows(2) {
            assert!(blocks[w[1]].centre.distance(blocks[w[0]].centre) >= 180.0 - 1e-9);
        }
        assert!(squares.len() >= 2, "{squares:?}");
    }

    #[test]
    fn a_park_is_made_where_none_is_near() {
        let mut blocks: Vec<Block> = (0..6)
            .map(|i| Block {
                polygon: square(i as f64 * 120.0, 0.0, 110.0),
                area: 12_100.0,
                centre: DVec2::new(i as f64 * 120.0 + 55.0, 55.0),
                district: 0,
                use_: BlockUse::Built,
                courtyard: None,
                buildable: square(i as f64 * 120.0, 0.0, 110.0),
            })
            .collect();
        add_parks(&mut blocks, 300.0, 10_000.0, 0.5);
        let parks = blocks.iter().filter(|b| b.use_ == BlockUse::Park).count();
        assert!((1..=2).contains(&parks), "{parks} parks");
    }
}
