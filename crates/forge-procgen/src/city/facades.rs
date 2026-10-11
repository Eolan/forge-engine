//! The component split of D-039's grammar: each mass's walls as faces with their context, the
//! start shapes of a style set's rules ([`crate::grammar`]). A wall is on the street when the
//! ground outside it is street up to the building line, on the courtyard when it looks into its
//! own lot, and a party wall when its neighbour's lot lies against it; walls between two pieces
//! of one lot are inside the building and have no face. A street face knows the corners it ends
//! on, and each building's longest street face holds its entrance.

use glam::{DVec2, DVec3};

use super::blocks::{edge_half_width, street_grid};
use super::geom;
use super::mass::Mass;
use super::{CityPlan, mass};
use crate::grammar::{Attrs, DeriveStats, Grammar, Scope, Terminal};

/// What a face looks onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FaceKind {
    /// The street.
    Street,
    /// The building's own courtyard or garden.
    Court,
    /// The neighbouring lot: a blank wall where the neighbour is lower.
    Party,
}

/// A wall of a mass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    /// Its mass's index.
    pub mass: u32,
    /// What it looks onto.
    pub kind: FaceKind,
    /// Its left end seen from outside, metres on the plan.
    pub left: DVec2,
    /// Its right end seen from outside.
    pub right: DVec2,
    /// Its height to the roof, metres.
    pub height: f64,
    /// Whether it turns a street corner at its left and right ends.
    pub corners: [bool; 2],
    /// Whether it holds its building's entrance.
    pub entrance: bool,
}

impl Face {
    /// Its width, metres.
    pub fn width(&self) -> f64 {
        self.left.distance(self.right)
    }

    /// Its scope in the plan's space (x east, y up, z south, as the plan's `y`), from the ground
    /// to `height + top`: `top` is what the style puts above the roof line (a parapet).
    pub fn scope(&self, top: f64) -> Scope {
        let along = (self.right - self.left).normalize_or_zero();
        Scope {
            origin: DVec3::new(self.left.x, 0.0, self.left.y),
            x: DVec3::new(along.x, 0.0, along.y),
            y: DVec3::Y,
            size: glam::DVec2::new(self.width(), self.height + top),
        }
    }
}

/// The faces of every mass, in mass order.
pub fn faces(plan: &CityPlan, masses: &[Mass]) -> Vec<Face> {
    let streets = street_grid(&plan.roads, plan.size);
    // Each lot's masses, to tell walls inside a building from walls between lots.
    let mut by_lot: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for (i, m) in masses.iter().enumerate() {
        by_lot.entry(m.lot).or_default().push(i as u32);
    }
    let mut lots_of = vec![Vec::new(); plan.blocks.len()];
    for (i, lot) in plan.lots.iter().enumerate() {
        lots_of[lot.block as usize].push(i as u32);
    }
    let mut out = Vec::new();
    for (index, m) in masses.iter().enumerate() {
        let lot = &plan.lots[m.lot as usize];
        let block = &plan.blocks[lot.block as usize];
        let n = m.footprint.len();
        let first = out.len();
        let mut kinds = Vec::with_capacity(n);
        for i in 0..n {
            let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
            let t = (b - a).normalize_or_zero();
            if t == DVec2::ZERO || a.distance(b) < 0.5 {
                kinds.push(None);
                continue;
            }
            // Outward: right of the edge for a positive outline.
            let out_dir = DVec2::new(t.y, -t.x);
            let mid = (a + b) * 0.5;
            let probe = mid + out_dir * 0.75;
            let sibling = by_lot[&m.lot].iter().any(|&j| {
                j as usize != index && geom::contains(&masses[j as usize].footprint, probe)
            });
            // The courtyard first, then a neighbour's lot, then the street: a wall within a
            // pace of its street's building line. What is left looks onto the lot's own garden.
            let courtyard = || {
                block
                    .courtyard
                    .as_ref()
                    .is_some_and(|c| geom::contains(c, probe))
            };
            let neighbour = || {
                lots_of[lot.block as usize]
                    .iter()
                    .any(|&l| l != m.lot && geom::contains(&plan.lots[l as usize].polygon, probe))
            };
            let on_street = || {
                let k = block.polygon.len();
                (0..k).any(|e| {
                    let (p, q) = (block.polygon[e], block.polygon[(e + 1) % k]);
                    let half = edge_half_width(&streets, &plan.roads, p, q);
                    geom::distance_to_segment(mid, p, q) < half + 1.5
                })
            };
            let kind = if sibling {
                None
            } else if courtyard() {
                Some(FaceKind::Court)
            } else if neighbour() {
                Some(FaceKind::Party)
            } else if on_street() {
                Some(FaceKind::Street)
            } else {
                Some(FaceKind::Court)
            };
            kinds.push(kind);
        }
        for i in 0..n {
            let Some(kind) = kinds[i] else { continue };
            let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
            // Seen from outside the right end is `a`, the left `b`. A corner where the next
            // street face turns outwards.
            let turn = |p: DVec2, q: DVec2, r: DVec2| geom::cross(q - p, r - q) > 1e-6;
            let prev = (i + n - 1) % n;
            let next = (i + 1) % n;
            let street = |j: usize| kinds[j] == Some(FaceKind::Street);
            let corners = if kind == FaceKind::Street {
                [
                    street(next) && turn(a, b, m.footprint[(i + 2) % n]),
                    street(prev) && turn(m.footprint[prev], a, b),
                ]
            } else {
                [false; 2]
            };
            out.push(Face {
                mass: index as u32,
                kind,
                left: b,
                right: a,
                height: m.height,
                corners,
                entrance: false,
            });
        }
        // The entrance on the longest street face.
        if let Some(face) = out[first..]
            .iter_mut()
            .filter(|f| f.kind == FaceKind::Street)
            .max_by(|x, y| x.width().total_cmp(&y.width()))
        {
            face.entrance = true;
        }
    }
    out
}

/// A face's attributes for a grammar: `street`, `court` and `party` (1 for its kind),
/// `corner_left`, `corner_right`, `entrance`, `tiled` (a pitched roof over it), `floors`,
/// `street_width`, and the storey heights `ground` and `floor`.
pub fn attrs(grammar: &Grammar, face: &Face, m: &Mass) -> Attrs {
    let flag = |b: bool| f64::from(u8::from(b));
    grammar.attrs(&[
        ("street", flag(face.kind == FaceKind::Street)),
        ("court", flag(face.kind == FaceKind::Court)),
        ("party", flag(face.kind == FaceKind::Party)),
        ("corner_left", flag(face.corners[0])),
        ("corner_right", flag(face.corners[1])),
        ("entrance", flag(face.entrance)),
        ("tiled", flag(m.roof == mass::Roof::Tiled)),
        ("floors", f64::from(m.floors)),
        ("street_width", m.street_width),
        ("ground", mass::GROUND_FLOOR),
        ("floor", mass::FLOOR),
    ])
}

/// The terminals of each face derived from `start`, the faces of one lot sharing their seed so
/// a building keeps one design; `top` metres stand over the roof line. Each terminal comes with
/// its face's index.
pub fn derive(
    grammar: &Grammar,
    start: &str,
    faces: &[Face],
    masses: &[Mass],
    top: f64,
    seed: u64,
) -> (Vec<(u32, Terminal)>, DeriveStats) {
    let mut out = Vec::new();
    let mut stats = DeriveStats::default();
    let mut terminals = Vec::new();
    for (i, face) in faces.iter().enumerate() {
        let m = &masses[face.mass as usize];
        let building = seed ^ u64::from(m.lot).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        terminals.clear();
        stats += grammar.derive(
            start,
            face.scope(top),
            &attrs(grammar, face, m),
            building,
            &mut terminals,
        );
        out.extend(terminals.iter().map(|&t| (i as u32, t)));
    }
    (out, stats)
}

/// The example style set of the engine's demos: a Mediterranean-European perimeter block
/// (D-059's first style set), in the grammar's text form.
pub const MEDITERRANEAN: &str = include_str!("mediterranean.rules");

#[cfg(test)]
mod tests {
    use super::super::bible::StyleBible;
    use super::super::{example, plan};
    use super::*;

    #[test]
    fn faces_look_onto_streets_courtyards_and_neighbours() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = mass::masses(&city, &StyleBible::default(), 9);
        let faces = faces(&city, &masses);
        let width = |k: FaceKind| -> f64 {
            faces
                .iter()
                .filter(|f| f.kind == k)
                .map(Face::width)
                .sum::<f64>()
        };
        let (street, court, party) = (
            width(FaceKind::Street),
            width(FaceKind::Court),
            width(FaceKind::Party),
        );
        let total = street + court + party;
        // Perimeter blocks: streets and courtyards on either side, party walls between lots.
        assert!(street / total > 0.25, "street {street} of {total}");
        assert!(court / total > 0.15, "court {court} of {total}");
        assert!(party / total > 0.1, "party {party} of {total}");
        let built: std::collections::HashSet<u32> = faces
            .iter()
            .filter(|f| f.kind == FaceKind::Street)
            .map(|f| f.mass)
            .collect();
        let entrances = faces.iter().filter(|f| f.entrance).count();
        assert_eq!(entrances, built.len(), "one entrance per mass on a street");
        assert!(faces.iter().any(|f| f.corners[0] || f.corners[1]));
    }

    #[test]
    fn the_example_style_derives_every_face() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = mass::masses(&city, &StyleBible::default(), 9);
        let faces = faces(&city, &masses);
        let grammar = Grammar::parse(MEDITERRANEAN).unwrap();
        let some: Vec<Face> = faces.iter().take(400).copied().collect();
        let (terminals, stats) = derive(&grammar, "Facade", &some, &masses, 0.9, 1);
        assert_eq!(stats.unmatched, 0, "{stats:?}");
        assert_eq!(stats.too_deep, 0);
        // The terminals cover each face exactly.
        let mut area = vec![0.0; some.len()];
        for (f, t) in &terminals {
            area[*f as usize] += t.scope.size.x * t.scope.size.y;
        }
        for (face, a) in some.iter().zip(area) {
            // A tiled roof's face stops at its cornice (the eaves are nil); party walls are whole.
            let tiled = masses[face.mass as usize].roof == mass::Roof::Tiled
                && face.kind != FaceKind::Party;
            let full = face.width() * (face.height + if tiled { 0.4 } else { 0.9 });
            assert!((a - full).abs() < 1e-6 * full.max(1.0), "{a} of {full}");
        }
        let windows = grammar.symbol("window").unwrap();
        assert!(terminals.iter().any(|(_, t)| t.module == windows));
    }
}
