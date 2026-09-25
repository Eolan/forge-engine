//! `u64` ids for the cells of a partition ([`crate::partition`]): the streaming and persistence
//! keys (S2-style, `docs/research/large-worlds.md` §5). One id names a square cell of a flat
//! grid or of a cube-sphere face at a level of a quadtree; a parent's id and its children's are
//! arithmetic on the bits, so a cell's ancestors and descendants need no table.
//!
//! Layout, high bits first: `kind` (2 bits: 0 the flat grid, 1 the cube sphere), `level` (5 bits,
//! 0–31), `face` (3 bits, 0–5; 0 on the flat grid), `x` (27 bits), `y` (27 bits). On the cube
//! sphere `x` and `y` count cells across the face at `level`, in `0..2^level`; on the flat grid
//! they are signed cell coordinates offset by 2²⁶ (±67 million cells: at 1 km cells, ±67 billion
//! km).

use std::fmt;

/// The bits of a level.
const LEVEL_BITS: u32 = 5;
/// The bits of a coordinate.
const COORD_BITS: u32 = 27;
/// The offset a flat grid's signed coordinates carry.
const FLAT_OFFSET: i64 = 1 << (COORD_BITS - 1);
/// The deepest level.
pub const MAX_LEVEL: u8 = 27;

/// Which partition a cell belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellKind {
    /// A flat grid of squares in the x–z plane ([`crate::partition::FlatGrid`]).
    Flat,
    /// A face of the cube sphere ([`crate::partition::CubeSphere`]).
    Cube,
}

/// A square cell of a partition at a level of its quadtree.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(pub u64);

impl CellId {
    /// A cell of the flat grid at `level`, cell coordinates `x`, `y` (signed).
    pub fn flat(level: u8, x: i32, y: i32) -> Self {
        assert!(level <= MAX_LEVEL, "level {level} beyond {MAX_LEVEL}");
        let ox = i64::from(x) + FLAT_OFFSET;
        let oy = i64::from(y) + FLAT_OFFSET;
        assert!(
            (0..1 << COORD_BITS).contains(&ox) && (0..1 << COORD_BITS).contains(&oy),
            "flat cell ({x}, {y}) beyond the id's range"
        );
        Self::pack(CellKind::Flat, level, 0, ox as u64, oy as u64)
    }

    /// A cell of face `face` (0–5) of the cube sphere at `level`, `x` and `y` in `0..2^level`.
    pub fn cube(face: u8, level: u8, x: u32, y: u32) -> Self {
        assert!(face < 6, "face {face}");
        assert!(level <= MAX_LEVEL, "level {level} beyond {MAX_LEVEL}");
        let n = 1_u64 << level;
        assert!(
            u64::from(x) < n && u64::from(y) < n,
            "cube cell ({x}, {y}) beyond level {level}"
        );
        Self::pack(CellKind::Cube, level, face, u64::from(x), u64::from(y))
    }

    fn pack(kind: CellKind, level: u8, face: u8, x: u64, y: u64) -> Self {
        let kind = match kind {
            CellKind::Flat => 0,
            CellKind::Cube => 1,
        };
        Self(
            kind << 62
                | u64::from(level) << (62 - LEVEL_BITS)
                | u64::from(face) << (2 * COORD_BITS)
                | x << COORD_BITS
                | y,
        )
    }

    /// Which partition.
    pub fn kind(self) -> CellKind {
        if self.0 >> 62 == 0 {
            CellKind::Flat
        } else {
            CellKind::Cube
        }
    }

    /// The quadtree level.
    pub fn level(self) -> u8 {
        ((self.0 >> (62 - LEVEL_BITS)) & ((1 << LEVEL_BITS) - 1)) as u8
    }

    /// The cube face (0 on the flat grid).
    pub fn face(self) -> u8 {
        ((self.0 >> (2 * COORD_BITS)) & 7) as u8
    }

    fn raw_x(self) -> u64 {
        (self.0 >> COORD_BITS) & ((1 << COORD_BITS) - 1)
    }

    fn raw_y(self) -> u64 {
        self.0 & ((1 << COORD_BITS) - 1)
    }

    /// The cell coordinates: signed on the flat grid, `0..2^level` on a cube face.
    pub fn xy(self) -> (i64, i64) {
        match self.kind() {
            CellKind::Flat => (
                self.raw_x() as i64 - FLAT_OFFSET,
                self.raw_y() as i64 - FLAT_OFFSET,
            ),
            CellKind::Cube => (self.raw_x() as i64, self.raw_y() as i64),
        }
    }

    /// The cell one level up that holds this one (none at level 0).
    pub fn parent(self) -> Option<Self> {
        let level = self.level();
        if level == 0 {
            return None;
        }
        let (x, y) = self.xy();
        Some(match self.kind() {
            CellKind::Flat => Self::flat(level - 1, x.div_euclid(2) as i32, y.div_euclid(2) as i32),
            CellKind::Cube => Self::cube(self.face(), level - 1, (x / 2) as u32, (y / 2) as u32),
        })
    }

    /// The four cells one level down that make this one (none at the deepest level).
    pub fn children(self) -> Option<[Self; 4]> {
        let level = self.level();
        if level >= MAX_LEVEL {
            return None;
        }
        let (x, y) = self.xy();
        let child = |dx: i64, dy: i64| match self.kind() {
            CellKind::Flat => Self::flat(level + 1, (2 * x + dx) as i32, (2 * y + dy) as i32),
            CellKind::Cube => Self::cube(
                self.face(),
                level + 1,
                (2 * x + dx) as u32,
                (2 * y + dy) as u32,
            ),
        };
        Some([child(0, 0), child(1, 0), child(0, 1), child(1, 1)])
    }

    /// Whether `other` lies in this cell (itself included).
    pub fn contains(self, other: Self) -> bool {
        if self.kind() != other.kind()
            || self.face() != other.face()
            || other.level() < self.level()
        {
            return false;
        }
        let shift = u32::from(other.level() - self.level());
        let (x, y) = self.xy();
        let (ox, oy) = other.xy();
        ox.div_euclid(1 << shift) == x && oy.div_euclid(1 << shift) == y
    }
}

impl fmt::Debug for CellId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (x, y) = self.xy();
        match self.kind() {
            CellKind::Flat => write!(f, "flat L{} ({x}, {y})", self.level()),
            CellKind::Cube => write!(f, "cube face {} L{} ({x}, {y})", self.face(), self.level()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_keep_their_fields_and_stay_ordered_by_kind_and_level() {
        let flat = CellId::flat(12, -5, 300);
        assert_eq!(flat.kind(), CellKind::Flat);
        assert_eq!(flat.level(), 12);
        assert_eq!(flat.xy(), (-5, 300));
        let cube = CellId::cube(4, 20, 123_456, 7);
        assert_eq!(cube.kind(), CellKind::Cube);
        assert_eq!(
            (cube.face(), cube.level(), cube.xy()),
            (4, 20, (123_456, 7))
        );
        assert!(flat < cube);
        assert!(CellId::flat(3, 0, 0) < CellId::flat(4, 0, 0));
        assert_eq!(format!("{cube:?}"), "cube face 4 L20 (123456, 7)");
    }

    #[test]
    fn parents_hold_their_children() {
        let cell = CellId::flat(5, -3, 7);
        let parent = cell.parent().unwrap();
        assert_eq!(parent, CellId::flat(4, -2, 3));
        assert!(parent.contains(cell));
        assert!(parent.children().unwrap().contains(&cell));
        assert!(!cell.contains(parent));
        assert!(cell.contains(cell));
        assert!(!CellId::flat(4, -1, 3).contains(cell));
        let leaf = CellId::cube(2, MAX_LEVEL, 1, 2);
        assert!(leaf.children().is_none());
        assert_eq!(CellId::cube(2, 0, 0, 0).parent(), None);
        assert!(CellId::cube(2, 0, 0, 0).contains(leaf));
        assert!(!CellId::cube(3, 0, 0, 0).contains(leaf));
        let mut up = leaf;
        for _ in 0..MAX_LEVEL {
            up = up.parent().unwrap();
        }
        assert_eq!(up, CellId::cube(2, 0, 0, 0));
    }
}
