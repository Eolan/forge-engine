//! Reference frames (D-004): `f64` positions in a tree of frames, each frame's origin and
//! orientation given in its parent. The roots are the sectors of an integer grid
//! ([`SectorId`], [`SECTOR_SIZE`]); under a sector come star systems, under a system its bodies,
//! under a body its constructs (a ship, a station, a building). No frame holds a magnitude above
//! about 10¹³ m, so an `f64` resolves every position to millimetres (2 mm at 10¹³ m); the
//! renderer, physics and audio work in `f32` relative to an anchor taken from here: the camera's
//! cell for the GPU ([`crate::cells`]), a construct's origin for physics. There is no origin
//! rebasing (unsupported in multiplayer, D-004): each client asks the tree for positions
//! relative to its own camera.
//!
//! Dungeon Siege's rule (Bilas 2003, `docs/research/large-worlds.md` §1): there is no world
//! space. A position is a frame and an offset, and two positions meet only through the tree,
//! which walks up to their common ancestor. Two sectors meet through their integer ids: the
//! difference is exact.

use glam::{DQuat, DVec3, I64Vec3};

use crate::cells::CellPos;

/// Side of a sector, metres: 2⁴⁰ (1.1 × 10¹² m, 7.35 AU). A position inside a sector's frame
/// is at most 2⁴⁰ m from its corner, so an `f64` resolves it to 2⁻¹² m (0.24 mm); the product
/// of a sector difference with the size is exact up to 2⁵³ sectors, and `i64` sectors span
/// 2¹⁰³ m, far beyond the observable universe (about 2⁹⁰ m).
pub const SECTOR_SIZE: f64 = 1_099_511_627_776.0;

/// A sector of the integer grid the frames hang from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SectorId(pub I64Vec3);

impl SectorId {
    /// The sector holding a position given in metres from the grid's origin, as an `f64`
    /// (fine for positions near the origin; a far position is stated as a sector and an
    /// offset, never as one `f64`).
    pub fn of(position: DVec3) -> Self {
        Self((position / SECTOR_SIZE).floor().as_i64vec3())
    }

    /// `self − other` in metres, exact: the integer difference, then the size.
    pub fn offset_from(self, other: Self) -> DVec3 {
        (self.0 - other.0).as_dvec3() * SECTOR_SIZE
    }
}

/// What a frame stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// A root: a sector of the grid.
    Sector(SectorId),
    /// A star system, under a sector.
    System,
    /// A planet, moon, asteroid or star, under a system (or a body).
    Body,
    /// A ship, station, vehicle or building: things that move as one, under a body or a
    /// system.
    Construct,
}

/// A frame of the tree: where it stands and how it is turned in its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// What it stands for.
    pub kind: FrameKind,
    /// Its parent; none for a sector.
    pub parent: Option<FrameId>,
    /// Its origin in its parent's frame, metres.
    pub origin: DVec3,
    /// Its axes in its parent's frame: a point `p` of this frame is `rotation * p + origin` in
    /// the parent.
    pub rotation: DQuat,
}

/// A frame's index in its [`FrameTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FrameId(pub u32);

/// A position: a frame and an offset in it, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldPos {
    /// The frame the offset is given in.
    pub frame: FrameId,
    /// Metres from the frame's origin, along its axes.
    pub position: DVec3,
}

impl WorldPos {
    /// A position in `frame`.
    pub fn new(frame: FrameId, position: DVec3) -> Self {
        Self { frame, position }
    }
}

/// A frame's placement relative to an ancestor: `p_ancestor = rotation * p + origin`.
#[derive(Clone, Copy, Debug)]
struct Placement {
    origin: DVec3,
    rotation: DQuat,
}

impl Placement {
    const IDENTITY: Self = Self {
        origin: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    };

    /// `frame`'s placement applied after this one: a point of the inner frame, in the outer.
    fn then(self, frame: &Frame) -> Self {
        Self {
            origin: frame.rotation * self.origin + frame.origin,
            rotation: frame.rotation * self.rotation,
        }
    }
}

/// The tree of frames.
#[derive(Clone, Debug, Default)]
pub struct FrameTree {
    frames: Vec<Frame>,
}

impl FrameTree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a sector, a root.
    pub fn add_sector(&mut self, sector: SectorId) -> FrameId {
        self.push(Frame {
            kind: FrameKind::Sector(sector),
            parent: None,
            origin: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
        })
    }

    /// Adds a frame under `parent`, its origin and axes given in the parent's frame.
    pub fn add(
        &mut self,
        parent: FrameId,
        kind: FrameKind,
        origin: DVec3,
        rotation: DQuat,
    ) -> FrameId {
        assert!(
            !matches!(kind, FrameKind::Sector(_)),
            "a sector is a root: add_sector"
        );
        assert!(
            (parent.0 as usize) < self.frames.len(),
            "unknown parent frame"
        );
        self.push(Frame {
            kind,
            parent: Some(parent),
            origin,
            rotation: rotation.normalize(),
        })
    }

    fn push(&mut self, frame: Frame) -> FrameId {
        let id = FrameId(self.frames.len() as u32);
        self.frames.push(frame);
        id
    }

    /// The frame.
    pub fn frame(&self, id: FrameId) -> &Frame {
        &self.frames[id.0 as usize]
    }

    /// How many frames the tree holds.
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Whether the tree is empty.
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Moves a frame in its parent (a body on its orbit, a ship under way).
    pub fn set_origin(&mut self, id: FrameId, origin: DVec3) {
        self.frames[id.0 as usize].origin = origin;
    }

    /// Turns a frame in its parent (a rotating planet, a banking ship).
    pub fn set_rotation(&mut self, id: FrameId, rotation: DQuat) {
        self.frames[id.0 as usize].rotation = rotation.normalize();
    }

    /// The frame and its ancestors, the root last.
    fn lineage(&self, id: FrameId) -> Vec<FrameId> {
        let mut chain = vec![id];
        while let Some(parent) = self.frame(*chain.last().expect("a frame")).parent {
            chain.push(parent);
        }
        chain
    }

    /// The sector a frame hangs from.
    pub fn sector(&self, id: FrameId) -> SectorId {
        let root = *self.lineage(id).last().expect("a frame");
        match self.frame(root).kind {
            FrameKind::Sector(sector) => sector,
            _ => unreachable!("a root is a sector"),
        }
    }

    /// The frame's placement relative to `ancestor` (`None`: its root), composed upwards.
    fn placement(&self, id: FrameId, ancestor: Option<FrameId>) -> Placement {
        let mut at = Placement::IDENTITY;
        let mut current = id;
        while Some(current) != ancestor {
            let frame = self.frame(current);
            at = at.then(frame);
            match frame.parent {
                Some(parent) => current = parent,
                None => break,
            }
        }
        at
    }

    /// `pos` expressed in `target`'s frame: up from `pos.frame` to the lowest ancestor the
    /// two share (Dungeon Siege's space walk: a ship never meets its star's numbers to be
    /// placed on its planet), across sectors by the exact integer difference when they share
    /// none, then down into `target`.
    pub fn relative(&self, pos: WorldPos, target: FrameId) -> DVec3 {
        let up_chain = self.lineage(pos.frame);
        let down_chain = self.lineage(target);
        // The deepest common frame: the chains agree from their roots down to it.
        let common = up_chain
            .iter()
            .rev()
            .zip(down_chain.iter().rev())
            .take_while(|(a, b)| a == b)
            .last()
            .map(|(a, _)| *a);
        let up = self.placement(pos.frame, common);
        let down = self.placement(target, common);
        let mut in_common = up.rotation * pos.position + up.origin;
        if common.is_none() {
            in_common += self.sector(pos.frame).offset_from(self.sector(target));
        }
        down.rotation.inverse() * (in_common - down.origin)
    }

    /// `pos` restated in `target`'s frame.
    pub fn to_frame(&self, pos: WorldPos, target: FrameId) -> WorldPos {
        WorldPos::new(target, self.relative(pos, target))
    }

    /// Metres between two positions.
    pub fn distance(&self, a: WorldPos, b: WorldPos) -> f64 {
        (self.relative(a, b.frame) - b.position).length()
    }

    /// `pos` as the GPU stores it ([`CellPos`]): cells and an `f32` offset from `anchor`'s
    /// origin, along `anchor`'s axes. A scene's instances take the scene's frame as their
    /// anchor, the camera the same, and the shaders subtract the two.
    pub fn cell_pos(&self, pos: WorldPos, anchor: FrameId) -> CellPos {
        CellPos::from_f64(self.relative(pos, anchor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sector with a star system 10¹¹ m into it, a planet 1.5 × 10¹¹ m from the star turned a
    /// quarter about +Y, and a ship 7 × 10⁶ m from the planet's centre.
    fn system() -> (FrameTree, FrameId, FrameId, FrameId, FrameId) {
        let mut tree = FrameTree::new();
        let sector = tree.add_sector(SectorId(I64Vec3::new(3, 0, -2)));
        let star = tree.add(
            sector,
            FrameKind::System,
            DVec3::splat(1e11),
            DQuat::IDENTITY,
        );
        let planet = tree.add(
            star,
            FrameKind::Body,
            DVec3::new(1.5e11, 0.0, 0.0),
            DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2),
        );
        let ship = tree.add(
            planet,
            FrameKind::Construct,
            DVec3::new(0.0, 0.0, 7e6),
            DQuat::IDENTITY,
        );
        (tree, sector, star, planet, ship)
    }

    #[test]
    fn positions_walk_the_tree_through_the_common_ancestor() {
        let (tree, _sector, star, planet, ship) = system();
        // The ship's origin, in the planet's frame, is where it was put.
        let origin = WorldPos::new(ship, DVec3::ZERO);
        assert_eq!(tree.relative(origin, planet), DVec3::new(0.0, 0.0, 7e6));
        // In the star's frame: the planet's quarter turn takes +Z to +X.
        let in_star = tree.relative(origin, star);
        assert!((in_star - DVec3::new(1.5e11 + 7e6, 0.0, 0.0)).length() < 1e-3);
        // Within the planet, the star's 10¹¹ m never enter: the walk stops at the planet.
        let bridge = WorldPos::new(ship, DVec3::new(0.0, 0.0, -50.0));
        assert_eq!(
            tree.relative(bridge, planet),
            DVec3::new(0.0, 0.0, 7e6 - 50.0)
        );
        // A point on the ship, seen from the planet, and back.
        let bow = WorldPos::new(ship, DVec3::new(0.0, 0.0, -50.0));
        let round = tree.to_frame(tree.to_frame(bow, star), ship);
        assert!((round.position - bow.position).length() < 1e-6);
        assert_eq!(tree.sector(ship), SectorId(I64Vec3::new(3, 0, -2)));
        assert_eq!(tree.len(), 4);
    }

    #[test]
    fn two_sectors_meet_through_their_integer_difference() {
        let mut tree = FrameTree::new();
        let a = tree.add_sector(SectorId(I64Vec3::new(0, 0, 0)));
        let b = tree.add_sector(SectorId(I64Vec3::new(1, 0, 0)));
        // A point a metre inside sector b, seen from sector a's frame: exactly one sector plus
        // a metre, although 10¹² m apart.
        let p = WorldPos::new(b, DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(tree.relative(p, a), DVec3::new(SECTOR_SIZE + 1.0, 0.0, 0.0));
        // Two ships a metre apart across the border: the distance is exact.
        let left = WorldPos::new(a, DVec3::new(SECTOR_SIZE - 0.5, 0.0, 0.0));
        let right = WorldPos::new(b, DVec3::new(0.5, 0.0, 0.0));
        assert_eq!(tree.distance(left, right), 1.0);
        assert_eq!(
            SectorId(I64Vec3::new(5, -7, 2)).offset_from(SectorId(I64Vec3::new(4, -7, 3))),
            DVec3::new(SECTOR_SIZE, 0.0, -SECTOR_SIZE)
        );
        assert_eq!(
            SectorId::of(DVec3::new(-1.0, 0.0, SECTOR_SIZE * 2.5)),
            SectorId(I64Vec3::new(-1, 0, 2))
        );
    }

    #[test]
    fn the_gpu_record_is_the_position_relative_to_the_anchor() {
        let (tree, _sector, _star, planet, ship) = system();
        // The scene's anchor is the planet's frame: a point 1.25 m ahead of the ship, 7 000 km
        // from the planet's centre, lands in the cell 6835 (7e6 = 6835 × 1024 + 960) with an
        // offset exact to the f32.
        let ahead = WorldPos::new(ship, DVec3::new(0.0, 0.0, 1.25));
        let cell = tree.cell_pos(ahead, planet);
        assert_eq!(cell.cell, glam::IVec3::new(0, 0, 6835));
        assert_eq!(cell.local, glam::Vec3::new(0.0, 0.0, 961.25));
        // Relative to the ship itself: the cell record is the small offset.
        let own = tree.cell_pos(ahead, ship);
        assert_eq!(own.cell, glam::IVec3::ZERO);
        assert_eq!(own.local, glam::Vec3::new(0.0, 0.0, 1.25));
    }

    #[test]
    fn frames_move_and_turn() {
        let (mut tree, _sector, star, planet, ship) = system();
        tree.set_origin(planet, DVec3::new(0.0, 0.0, 1.5e11));
        tree.set_rotation(planet, DQuat::IDENTITY);
        let origin = WorldPos::new(ship, DVec3::ZERO);
        assert!((tree.relative(origin, star) - DVec3::new(0.0, 0.0, 1.5e11 + 7e6)).length() < 1e-3);
    }
}
