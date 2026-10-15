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
use super::structure::{OpeningKind, Structure, Wall, WallRole};
use super::{CityPlan, mass};
use crate::grammar::{DeriveStats, Gap, Grammar, Scope, Terminal};

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
    faces_of(masses, &edge_kinds(plan, masses))
}

/// Each lot's masses (to tell walls inside a building from walls between lots), and each
/// block's lots.
type LotMaps = (std::collections::HashMap<u32, Vec<u32>>, Vec<Vec<u32>>);

fn lot_maps(plan: &CityPlan, masses: &[Mass]) -> LotMaps {
    let mut by_lot: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for (i, m) in masses.iter().enumerate() {
        by_lot.entry(m.lot).or_default().push(i as u32);
    }
    let mut lots_of = vec![Vec::new(); plan.blocks.len()];
    for (i, lot) in plan.lots.iter().enumerate() {
        lots_of[lot.block as usize].push(i as u32);
    }
    (by_lot, lots_of)
}

/// How tall the buildings across each edge of each mass's footprint stand (edge `i` from
/// vertex `i` to `i + 1`): the lowest, along the wall, of the tallest mass a pace outside it
/// at five points, of the lot's other masses or of the block's other lots; 0 where nothing
/// stands at one of the points. A gable wants a neighbour as tall as its eaves all along.
pub fn neighbour_heights(plan: &CityPlan, masses: &[Mass]) -> Vec<Vec<f64>> {
    let (by_lot, lots_of) = lot_maps(plan, masses);
    masses
        .iter()
        .enumerate()
        .map(|(index, m)| {
            let block = plan.lots[m.lot as usize].block as usize;
            let n = m.footprint.len();
            (0..n)
                .map(|i| {
                    let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
                    let t = (b - a).normalize_or_zero();
                    let out_dir = DVec2::new(t.y, -t.x);
                    [0.1, 0.3, 0.5, 0.7, 0.9]
                        .into_iter()
                        .map(|s| {
                            let probe = a + (b - a) * s + out_dir * 0.75;
                            lots_of[block]
                                .iter()
                                .flat_map(|l| by_lot.get(l).into_iter().flatten())
                                .filter(|&&j| {
                                    j as usize != index
                                        && geom::contains(&masses[j as usize].footprint, probe)
                                })
                                .map(|&j| masses[j as usize].height)
                                .fold(0.0, f64::max)
                        })
                        .fold(f64::INFINITY, f64::min)
                })
                .collect()
        })
        .collect()
}

/// What each edge of each mass's footprint looks onto (edge `i` from vertex `i` to `i + 1`):
/// `None` for a wall inside its building (onto another mass of its lot) or a sliver under half
/// a metre.
pub fn edge_kinds(plan: &CityPlan, masses: &[Mass]) -> Vec<Vec<Option<FaceKind>>> {
    let streets = street_grid(&plan.roads, plan.size);
    let (by_lot, lots_of) = lot_maps(plan, masses);
    let mut out = Vec::with_capacity(masses.len());
    for (index, m) in masses.iter().enumerate() {
        let lot = &plan.lots[m.lot as usize];
        let block = &plan.blocks[lot.block as usize];
        let n = m.footprint.len();
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
            // A wall inside the building is one the lot's other masses cover all along: probed
            // at five points, not only its middle. A wall they covered in part lost its whole
            // face, and the street saw into the building (the owner's walk, 2026-10-11).
            let covered = |s: f64| {
                let q = a + (b - a) * s + out_dir * 0.75;
                by_lot[&m.lot].iter().any(|&j| {
                    j as usize != index && geom::contains(&masses[j as usize].footprint, q)
                })
            };
            let bare: Vec<f64> = [0.5, 0.3, 0.7, 0.1, 0.9]
                .into_iter()
                .filter(|&s| !covered(s))
                .collect();
            let sibling = bare.is_empty();
            // What the wall looks onto, from its open stretch nearest its middle.
            let along = bare.first().copied().unwrap_or(0.5);
            let mid = a + (b - a) * along;
            let probe = mid + out_dir * 0.75;
            // The courtyard first, then a neighbour's lot, then the street: a wall within a
            // pace of its street's building line. What is left looks onto the lot's own garden.
            let courtyard = || {
                block
                    .courtyard
                    .as_ref()
                    .is_some_and(|c| geom::contains(c, probe))
            };
            // A neighbour's lot with a building on it: an empty lot is a garden.
            let neighbour = || {
                lots_of[lot.block as usize].iter().any(|&l| {
                    l != m.lot
                        && by_lot.contains_key(&l)
                        && geom::contains(&plan.lots[l as usize].polygon, probe)
                })
            };
            // A neighbour's building within 2.5 m, whatever lies between: a wall a metre from
            // another is blank, not windows onto a slit (the owner's second walk).
            let close = mid + out_dir * 2.5;
            let crowded = || {
                lots_of[lot.block as usize].iter().any(|&l| {
                    l != m.lot
                        && by_lot.get(&l).is_some_and(|ms| {
                            ms.iter()
                                .any(|&j| geom::contains(&masses[j as usize].footprint, close))
                        })
                })
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
            } else if crowded() {
                Some(FaceKind::Party)
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
        out.push(kinds);
    }
    out
}

/// The faces of every mass from its edges' kinds ([`edge_kinds`]), in mass order.
pub fn faces_of(masses: &[Mass], kinds: &[Vec<Option<FaceKind>>]) -> Vec<Face> {
    let mut out = Vec::new();
    for (index, (m, kinds)) in masses.iter().zip(kinds).enumerate() {
        let n = m.footprint.len();
        let first = out.len();
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

/// The kinds of opening a style reads in `opening.kind`.
pub mod opening {
    /// A window.
    pub const WINDOW: f64 = 1.0;
    /// A shopfront.
    pub const SHOPFRONT: f64 = 2.0;
    /// The building's door.
    pub const DOOR: f64 = 3.0;
    /// A passage into the court.
    pub const PASSAGE: f64 = 4.0;
}

/// A terminal of a building's dressing: its mass, its wall (an index into its structure's
/// walls), its storey (`None` over the roof line or on a party wall).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dressed {
    /// Its mass.
    pub mass: u32,
    /// Its wall.
    pub wall: u32,
    /// Its storey.
    pub storey: Option<u8>,
    /// What goes where.
    pub terminal: Terminal,
}

/// The scope of wall `w` of a mass seen from outside, from `bottom` up `height` metres: its
/// width from the wall's end `b` to its start `a` (as [`Face::scope`]).
fn wall_scope(w: &Wall, bottom: f64, height: f64) -> Scope {
    let along = (w.a - w.b).normalize_or_zero();
    Scope {
        origin: DVec3::new(w.b.x, bottom, w.b.y),
        x: DVec3::new(along.x, 0.0, along.y),
        y: DVec3::Y,
        size: glam::DVec2::new(w.length(), height),
    }
}

/// The masses `wanted` dressed by `grammar` from their structures (D-060): each storey of each
/// street and courtyard wall derived from `Storey` with its openings as gaps, the roofline from
/// `Roofline` (`top` metres over the roof), each party wall from `PartyWall`. One seed for a
/// lot's walls, so a building keeps one design.
pub fn dress(
    grammar: &Grammar,
    structures: &[Structure],
    masses: &[Mass],
    wanted: &[usize],
    top: f64,
    seed: u64,
) -> (Vec<Dressed>, DeriveStats) {
    let mut out = Vec::new();
    let mut stats = DeriveStats::default();
    let mut terminals = Vec::new();
    for &i in wanted {
        let s = &structures[i];
        let m = &masses[i];
        let building = seed ^ u64::from(m.lot).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let flag = |b: bool| f64::from(u8::from(b));
        let n = s.storeys.len();
        for (w, wall) in s.walls.iter().enumerate() {
            if wall.length() < 0.5 || wall.role == WallRole::Inner {
                continue;
            }
            let base = [
                ("street", flag(wall.role == WallRole::Street)),
                ("court", flag(wall.role == WallRole::Court)),
                ("party", flag(wall.role == WallRole::Party)),
                ("tiled", flag(m.roof == mass::Roof::Tiled)),
                ("floors", f64::from(m.floors)),
                ("street_width", m.street_width),
                ("open", flag(m.open)),
            ];
            let mut push = |storey: Option<u8>, terminals: &[Terminal]| {
                out.extend(terminals.iter().map(|&terminal| Dressed {
                    mass: i as u32,
                    wall: w as u32,
                    storey,
                    terminal,
                }));
            };
            if wall.role == WallRole::Party {
                terminals.clear();
                stats += grammar.derive(
                    "PartyWall",
                    wall_scope(wall, 0.0, m.height + top),
                    &grammar.attrs(&base),
                    building,
                    &mut terminals,
                );
                push(None, &terminals);
                continue;
            }
            let length = wall.length();
            for (k, storey) in s.storeys.iter().enumerate() {
                // The openings of this storey, along the scope's width from the wall's end `b`.
                let gaps: Vec<Gap> = s
                    .openings
                    .iter()
                    .filter(|o| o.wall == w && o.storey == k)
                    .map(|o| Gap {
                        from: length - o.span().1,
                        to: length - o.span().0,
                        kind: match o.kind {
                            OpeningKind::Window => opening::WINDOW,
                            OpeningKind::Shopfront => opening::SHOPFRONT,
                            OpeningKind::Door => opening::DOOR,
                            OpeningKind::Passage => opening::PASSAGE,
                        },
                        sill: o.sill,
                        head: o.head,
                    })
                    .collect();
                let mut attrs = base.to_vec();
                attrs.extend([
                    ("level.index", k as f64),
                    ("level.count", n as f64),
                    ("level.first", flag(k == 0)),
                    ("level.last", flag(k + 1 == n)),
                ]);
                terminals.clear();
                stats += grammar.derive_with_gaps(
                    "Storey",
                    wall_scope(wall, storey.level, storey.height),
                    &grammar.attrs(&attrs),
                    building,
                    &gaps,
                    &mut terminals,
                );
                push(Some(k as u8), &terminals);
            }
            terminals.clear();
            stats += grammar.derive(
                "Roofline",
                wall_scope(wall, m.height, top),
                &grammar.attrs(&base),
                building,
                &mut terminals,
            );
            push(None, &terminals);
        }
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
    fn a_gable_wants_a_neighbour_as_tall() {
        // Across a street wall nothing stands; a wall inside a building has its sibling all
        // along; and among the party walls some neighbours stand as tall and some lower, so
        // the roofs' rule (a gable only over a neighbour as tall) bites both ways.
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = mass::masses(&city, &StyleBible::default(), 9);
        let kinds = edge_kinds(&city, &masses);
        let heights = neighbour_heights(&city, &masses);
        let (mut streets, mut built_across, mut inside, mut open) = (0, 0, 0, 0);
        let (mut as_tall, mut lower) = (0, 0);
        for ((m, kinds), heights) in masses.iter().zip(&kinds).zip(&heights) {
            let n = m.footprint.len();
            for (i, (kind, &h)) in kinds.iter().zip(heights).enumerate() {
                if m.footprint[i].distance(m.footprint[(i + 1) % n]) < 0.5 {
                    continue;
                }
                match kind {
                    Some(FaceKind::Street) => {
                        streets += 1;
                        built_across += usize::from(h > 0.0);
                    }
                    None => {
                        inside += 1;
                        open += usize::from(h == 0.0);
                    }
                    Some(FaceKind::Party) if h >= m.height - 0.5 => as_tall += 1,
                    Some(FaceKind::Party) => lower += 1,
                    _ => {}
                }
            }
        }
        assert!(
            built_across * 100 <= streets,
            "{built_across} of {streets} street walls have a building across them"
        );
        assert!(
            open * 100 <= inside,
            "{open} of {inside} walls inside a building have nothing across them"
        );
        assert!(
            as_tall > 0 && lower > 0,
            "party walls: {as_tall} with a neighbour as tall, {lower} with a lower one"
        );
    }

    #[test]
    fn no_wall_is_left_open() {
        // A mass's edge without a face must lie inside its lot's other masses all along: probed
        // every tenth of it, at most one edge in a hundred is covered only in part.
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = mass::masses(&city, &StyleBible::default(), 9);
        let kinds = edge_kinds(&city, &masses);
        let (mut hidden, mut open) = (0, 0);
        for (index, (m, kinds)) in masses.iter().zip(&kinds).enumerate() {
            let n = m.footprint.len();
            for (i, kind) in kinds.iter().enumerate() {
                let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
                if kind.is_some() || a.distance(b) < 0.5 {
                    continue;
                }
                hidden += 1;
                let t = (b - a).normalize();
                let out_dir = DVec2::new(t.y, -t.x);
                let inside = (1..10).all(|k| {
                    let q = a + (b - a) * (f64::from(k) / 10.0) + out_dir * 0.75;
                    masses.iter().enumerate().any(|(j, o)| {
                        j != index && o.lot == m.lot && geom::contains(&o.footprint, q)
                    })
                });
                if !inside {
                    open += 1;
                }
            }
        }
        assert!(hidden > 0);
        assert!(
            open * 100 <= hidden,
            "{open} of {hidden} hidden walls open in part"
        );
    }

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
    fn the_example_style_dresses_every_wall_round_its_openings() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let masses = mass::masses(&city, &StyleBible::default(), 9);
        let structures = super::super::structure::structures(&city, &masses);
        let grammar = Grammar::parse(MEDITERRANEAN).unwrap();
        let some: Vec<usize> = (0..masses.len()).step_by(40).collect();
        let (dressed, stats) = dress(&grammar, &structures, &masses, &some, 0.9, 1);
        assert_eq!(stats.unmatched, 0, "{stats:?}");
        assert_eq!(stats.too_deep, 0);
        // The terminals cover each wall exactly, but for its passages; a tiled roof's wall
        // stops at its cornice (the eaves are nil).
        let mut area: std::collections::HashMap<(u32, u32), f64> = Default::default();
        for d in &dressed {
            *area.entry((d.mass, d.wall)).or_default() +=
                d.terminal.scope.size.x * d.terminal.scope.size.y;
        }
        for &i in &some {
            let (s, m) = (&structures[i], &masses[i]);
            for (w, wall) in s.walls.iter().enumerate() {
                if wall.length() < 0.5 || wall.role == WallRole::Inner {
                    continue;
                }
                let tiled = m.roof == mass::Roof::Tiled && wall.role != WallRole::Party;
                let holes: f64 = s
                    .openings
                    .iter()
                    .filter(|o| o.wall == w && o.kind == OpeningKind::Passage)
                    .map(|o| o.width * o.head)
                    .sum();
                let full = wall.length() * (m.height + if tiled { 0.4 } else { 0.9 }) - holes;
                let a = area.get(&(i as u32, w as u32)).copied().unwrap_or(0.0);
                assert!((a - full).abs() < 1e-6 * full.max(1.0), "{a} of {full}");
            }
        }
        // Every opening the structure holds is dressed: a window, a shopfront, a door or a hole.
        let windows = grammar.symbol("window").unwrap();
        assert!(dressed.iter().any(|d| d.terminal.module == windows));
        let doors = grammar.symbol("door").unwrap();
        // Every door the structures hold is dressed as one: the street's and the court's.
        let held: usize = some
            .iter()
            .map(|&i| {
                structures[i]
                    .openings
                    .iter()
                    .filter(|o| o.kind == OpeningKind::Door)
                    .count()
            })
            .sum();
        let dressed_doors = dressed
            .iter()
            .filter(|d| d.terminal.module == doors)
            .count();
        assert!(held >= some.len() / 2, "{held} doors");
        assert_eq!(dressed_doors, held, "every door dressed");
    }
}
