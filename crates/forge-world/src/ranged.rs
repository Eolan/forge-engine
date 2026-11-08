//! Pieces shown by their own ranges (D-106): an authored world's placed pieces each say how far
//! they show and from which point that is measured, and a low-detail stand-in replaces a group of
//! them. Cells ([`crate::streaming`]) stay the unit of loading; inside the cells in, this set tests
//! every piece against the viewer and says which come and go.
//!
//! - **A piece** shows while the viewer is within its `far` of its `reference`, a point that need
//!   not be the piece's own place.
//! - **A group** swaps as one: while any member is in range, all its members show and its stand-in
//!   waits; with none in range, the stand-in shows. No group is drawn in part or twice, and a group
//!   whose members' reach stops short of the stand-in's near distance has no gap.
//! - **A stand-in with no members in the world** obeys its own `near`: closer than that it hides.
//! - **Chains:** a stand-in may itself be a member of a coarser group. Showing, or with its own
//!   group occupied, it keeps the coarser one away; then all that group's members show, each past
//!   its own `far` if it must, so the coarser one's area stays covered.
//! - **Only pieces in cells count:** a stand-in shows until its members' cell has come in, so what
//!   is still loading is low detail and not a hole.
//!
//! A piece shown keeps showing `slack` metres past its limits, so a viewer standing on one does
//! not flip it.

use glam::DVec3;

/// A group's index: a stand-in and the pieces it replaces share one.
pub type GroupId = u32;

/// One placed piece.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangedPiece {
    /// The point its distances are measured from, metres in the partition's frame.
    pub reference: DVec3,
    /// Closer than this a stand-in with no members in the world hides, metres; 0 for none.
    pub near: f32,
    /// How far it shows, metres.
    pub far: f32,
    /// The group it is a member of: in range, it keeps that group's stand-in away.
    pub member_of: Option<GroupId>,
    /// The group it stands in for.
    pub stands_for: Option<GroupId>,
}

const LOADED: u8 = 1;
const SHOWN: u8 = 2;
const WITHIN_FAR: u8 = 4;
const PAST_NEAR: u8 = 8;
const IN_RANGE: u8 = WITHIN_FAR | PAST_NEAR;
/// Marks a slot free in [`RangedSet::slots`].
const NO_SLOT: u32 = u32::MAX;

/// What one [`RangedSet::refresh`] changed: piece indices, each list in no particular order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangedChanges<'a> {
    /// The pieces to bring in.
    pub add: &'a [u32],
    /// The pieces to take out.
    pub remove: &'a [u32],
}

/// A world's pieces and which of them show (see the module notes).
#[derive(Clone, Debug)]
pub struct RangedSet {
    pieces: Vec<RangedPiece>,
    /// Per group, how many members the world has.
    members: Vec<u32>,
    /// Per group, whether a member in a cell is in range: this refresh's.
    occupied: Vec<bool>,
    /// Per piece, its `LOADED`, `SHOWN`, `WITHIN_FAR` and `PAST_NEAR` bits.
    flags: Vec<u8>,
    /// The pieces in, and per piece its index there.
    loaded: Vec<u32>,
    slots: Vec<u32>,
    /// The stand-ins that are members of a coarser group.
    chained: Vec<u32>,
    slack: f64,
    add: Vec<u32>,
    remove: Vec<u32>,
}

impl RangedSet {
    /// A set over `pieces` in `groups` groups, none in yet; a piece shown keeps showing `slack`
    /// metres past its limits.
    ///
    /// # Panics
    ///
    /// If a piece names a group at or past `groups`.
    pub fn new(pieces: Vec<RangedPiece>, groups: u32, slack: f64) -> Self {
        let mut members = vec![0; groups as usize];
        let mut chained = Vec::new();
        for (i, p) in pieces.iter().enumerate() {
            if let Some(g) = p.member_of {
                members[g as usize] += 1;
                if p.stands_for.is_some() {
                    chained.push(i as u32);
                }
            }
            assert!(
                p.stands_for.is_none_or(|g| g < groups),
                "a stand-in names a group past the last"
            );
        }
        let n = pieces.len();
        Self {
            pieces,
            members,
            occupied: vec![false; groups as usize],
            flags: vec![0; n],
            loaded: Vec::new(),
            slots: vec![NO_SLOT; n],
            chained,
            slack,
            add: Vec::new(),
            remove: Vec::new(),
        }
    }

    /// How many pieces the world has.
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    /// Whether the world has no pieces.
    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    /// How many pieces are in cells that are in.
    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }

    /// Whether piece `piece` shows.
    pub fn is_shown(&self, piece: u32) -> bool {
        self.flags[piece as usize] & SHOWN != 0
    }

    /// A cell's pieces have come in. They show from the next [`Self::refresh`].
    pub fn enter(&mut self, pieces: &[u32]) {
        for &p in pieces {
            let i = p as usize;
            if self.flags[i] & LOADED == 0 {
                self.flags[i] |= LOADED;
                self.slots[i] = self.loaded.len() as u32;
                self.loaded.push(p);
            }
        }
    }

    /// A cell's pieces have left: those that showed, for the caller to take out.
    pub fn leave(&mut self, pieces: &[u32]) -> Vec<u32> {
        let mut shown = Vec::new();
        for &p in pieces {
            let i = p as usize;
            if self.flags[i] & LOADED == 0 {
                continue;
            }
            if self.flags[i] & SHOWN != 0 {
                shown.push(p);
            }
            self.flags[i] = 0;
            let slot = self.slots[i] as usize;
            self.slots[i] = NO_SLOT;
            self.loaded.swap_remove(slot);
            if let Some(&moved) = self.loaded.get(slot) {
                self.slots[moved as usize] = slot as u32;
            }
        }
        shown
    }

    /// Tests every piece in against `viewer` and says which come and go.
    pub fn refresh(&mut self, viewer: DVec3) -> RangedChanges<'_> {
        self.add.clear();
        self.remove.clear();
        self.occupied.fill(false);
        for &p in &self.loaded {
            let i = p as usize;
            let piece = &self.pieces[i];
            let extra = if self.flags[i] & SHOWN != 0 {
                self.slack
            } else {
                0.0
            };
            let d2 = piece.reference.distance_squared(viewer);
            let far = f64::from(piece.far) + extra;
            let near = (f64::from(piece.near) - extra).max(0.0);
            let mut bits = self.flags[i] & !IN_RANGE;
            if d2 <= far * far {
                bits |= WITHIN_FAR;
            }
            if d2 >= near * near {
                bits |= PAST_NEAR;
            }
            self.flags[i] = bits;
            if bits & IN_RANGE == IN_RANGE
                && piece.stands_for.is_none()
                && let Some(g) = piece.member_of
            {
                self.occupied[g as usize] = true;
            }
        }
        // Up the chains: each pass settles a level at least.
        for _ in 0..=self.chained.len() {
            let mut changed = false;
            for &p in &self.chained {
                let i = p as usize;
                let piece = &self.pieces[i];
                let Some(group) = piece.member_of else {
                    continue;
                };
                if self.occupied[group as usize] {
                    continue;
                }
                // Its own group occupied counts even with this stand-in's cell not in: finer
                // pieces show. Otherwise, whether it would show itself.
                let own = piece.stands_for.is_some_and(|g| self.occupied[g as usize]);
                let bits = self.flags[i];
                let would_show = bits & LOADED != 0
                    && match piece.stands_for {
                        Some(g) if self.members[g as usize] > 0 => bits & WITHIN_FAR != 0,
                        _ => bits & IN_RANGE == IN_RANGE,
                    };
                if own || would_show {
                    self.occupied[group as usize] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for &p in &self.loaded {
            let i = p as usize;
            let piece = &self.pieces[i];
            let bits = self.flags[i];
            // Its coarser group taken over by a sibling: it shows too, however far, or its part of
            // that group would have neither level drawn.
            let sibling = piece.member_of.is_some_and(|g| self.occupied[g as usize]);
            let in_reach = bits & WITHIN_FAR != 0 || sibling;
            let want = match (piece.stands_for, piece.member_of) {
                (Some(g), _) if self.members[g as usize] > 0 => {
                    in_reach && !self.occupied[g as usize]
                }
                (None, Some(g)) => self.occupied[g as usize],
                _ => in_reach && bits & PAST_NEAR != 0,
            };
            if want != (bits & SHOWN != 0) {
                self.flags[i] ^= SHOWN;
                if want {
                    self.add.push(p);
                } else {
                    self.remove.push(p);
                }
            }
        }
        RangedChanges {
            add: &self.add,
            remove: &self.remove,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(x: f64, far: f32) -> RangedPiece {
        RangedPiece {
            reference: DVec3::new(x, 0.0, 0.0),
            near: 0.0,
            far,
            member_of: None,
            stands_for: None,
        }
    }

    fn at(x: f64) -> DVec3 {
        DVec3::new(x, 0.0, 0.0)
    }

    fn sorted(mut v: Vec<u32>) -> Vec<u32> {
        v.sort_unstable();
        v
    }

    /// (added, removed) at `x`, sorted.
    fn step(set: &mut RangedSet, x: f64) -> (Vec<u32>, Vec<u32>) {
        let c = set.refresh(at(x));
        (sorted(c.add.to_vec()), sorted(c.remove.to_vec()))
    }

    #[test]
    fn a_piece_shows_within_its_range_and_only_once_in() {
        let mut set = RangedSet::new(vec![piece(0.0, 10.0)], 0, 0.0);
        assert_eq!(step(&mut set, 5.0), (vec![], vec![]), "its cell is not in");
        set.enter(&[0]);
        assert_eq!(step(&mut set, 5.0), (vec![0], vec![]));
        assert_eq!(step(&mut set, 6.0), (vec![], vec![]), "nothing changes");
        assert!(set.is_shown(0));
        assert_eq!(step(&mut set, 11.0), (vec![], vec![0]));
        assert!(!set.is_shown(0));
    }

    #[test]
    fn the_slack_keeps_a_viewer_on_a_limit_from_flipping_a_piece() {
        let mut set = RangedSet::new(vec![piece(0.0, 10.0)], 0, 2.0);
        set.enter(&[0]);
        assert_eq!(
            step(&mut set, 11.0),
            (vec![], vec![]),
            "outside, not shown: stays out"
        );
        assert_eq!(step(&mut set, 9.0), (vec![0], vec![]));
        assert_eq!(step(&mut set, 11.5), (vec![], vec![]), "within the slack");
        assert_eq!(step(&mut set, 12.5), (vec![], vec![0]));
    }

    #[test]
    fn distances_are_from_the_reference_not_the_viewers_axis() {
        let mut p = piece(0.0, 10.0);
        p.reference = DVec3::new(0.0, 8.0, 8.0);
        let mut set = RangedSet::new(vec![p], 0, 0.0);
        set.enter(&[0]);
        assert_eq!(step(&mut set, 0.0).0, Vec::<u32>::new(), "11.3 m away");
    }

    #[test]
    fn a_stand_in_with_no_members_obeys_its_near_distance() {
        let mut p = piece(0.0, 100.0);
        p.near = 20.0;
        p.stands_for = Some(0);
        let mut set = RangedSet::new(vec![p], 1, 0.0);
        set.enter(&[0]);
        assert_eq!(step(&mut set, 10.0), (vec![], vec![]), "closer than near");
        assert_eq!(step(&mut set, 30.0), (vec![0], vec![]));
        assert_eq!(step(&mut set, 19.0), (vec![], vec![0]));
    }

    /// Pieces 0 and 1 and their stand-in 2, in the group 0.
    fn group(stand_near: f32, member_far: f32) -> RangedSet {
        let mut a = piece(0.0, member_far);
        let mut b = piece(0.0, member_far);
        // The second member's point lies 10 m further on.
        b.reference.x = 10.0;
        a.member_of = Some(0);
        b.member_of = Some(0);
        let mut s = piece(0.0, 1000.0);
        s.near = stand_near;
        s.stands_for = Some(0);
        RangedSet::new(vec![a, b, s], 1, 0.0)
    }

    #[test]
    fn a_group_and_its_stand_in_swap_in_one_refresh() {
        let mut set = group(40.0, 50.0);
        set.enter(&[0, 1, 2]);
        assert_eq!(
            step(&mut set, 500.0),
            (vec![2], vec![]),
            "far: the stand-in"
        );
        assert_eq!(
            step(&mut set, 30.0),
            (vec![0, 1], vec![2]),
            "near: both members, no stand-in"
        );
        assert_eq!(step(&mut set, 500.0), (vec![2], vec![0, 1]));
    }

    #[test]
    fn one_member_in_range_brings_the_whole_group() {
        let mut set = group(40.0, 50.0);
        set.enter(&[0, 1, 2]);
        // 55 m from the first member's point, 45 m from the second's.
        assert_eq!(
            step(&mut set, 55.0),
            (vec![0, 1], vec![]),
            "the stand-in was never in"
        );
        assert!(!set.is_shown(2));
    }

    #[test]
    fn a_group_short_of_its_stand_ins_near_leaves_no_gap() {
        let mut set = group(40.0, 15.0);
        set.enter(&[0, 1, 2]);
        // 30 m away: past both members' reach and inside the stand-in's near.
        assert_eq!(
            step(&mut set, -30.0),
            (vec![2], vec![]),
            "the stand-in, near or not"
        );
    }

    #[test]
    fn the_stand_in_shows_until_its_members_cell_is_in() {
        let mut set = group(40.0, 50.0);
        set.enter(&[2]);
        assert_eq!(
            step(&mut set, 30.0),
            (vec![2], vec![]),
            "low detail, not a hole"
        );
        set.enter(&[0, 1]);
        assert_eq!(step(&mut set, 30.0), (vec![0, 1], vec![2]));
        let out = sorted(set.leave(&[0, 1]));
        assert_eq!(out, vec![0, 1], "those that showed");
        assert_eq!(
            step(&mut set, 30.0),
            (vec![2], vec![]),
            "back to the stand-in"
        );
    }

    #[test]
    fn a_chain_keeps_the_coarser_stand_in_away() {
        // A tile (0), its stand-in (1, a member of the coarser group 1), the coarser stand-in (2).
        let mut tile = piece(0.0, 50.0);
        tile.member_of = Some(0);
        let mut fine = piece(0.0, 300.0);
        fine.near = 40.0;
        fine.stands_for = Some(0);
        fine.member_of = Some(1);
        let mut coarse = piece(0.0, 3000.0);
        coarse.near = 200.0;
        coarse.stands_for = Some(1);
        let mut set = RangedSet::new(vec![tile, fine, coarse], 2, 0.0);
        set.enter(&[0, 1, 2]);
        assert_eq!(
            step(&mut set, 1000.0),
            (vec![2], vec![]),
            "far: the coarse one"
        );
        assert_eq!(
            step(&mut set, 200.0),
            (vec![1], vec![2]),
            "the fine one takes over"
        );
        assert_eq!(
            step(&mut set, 30.0),
            (vec![0], vec![1]),
            "the tile, the coarse one still away"
        );
    }

    /// A tile (0) reaching `tile_far`, its stand-in (1) near 40, the coarser stand-in (2).
    fn chain(tile_far: f32) -> RangedSet {
        let mut tile = piece(0.0, tile_far);
        tile.member_of = Some(0);
        let mut fine = piece(0.0, 300.0);
        fine.near = 40.0;
        fine.stands_for = Some(0);
        fine.member_of = Some(1);
        let mut coarse = piece(0.0, 3000.0);
        coarse.near = 10.0;
        coarse.stands_for = Some(1);
        RangedSet::new(vec![tile, fine, coarse], 2, 0.0)
    }

    #[test]
    fn a_chained_stand_in_shown_inside_its_near_keeps_the_coarser_one_away() {
        let mut set = chain(15.0);
        set.enter(&[0, 1, 2]);
        assert_eq!(
            step(&mut set, 30.0),
            (vec![1], vec![]),
            "past the tile's reach: the fine one only"
        );
    }

    /// A stand-in (`stands_for`) that is a member of the coarser group `of`.
    fn chained(x: f64, far: f32, stands_for: GroupId, of: GroupId) -> RangedPiece {
        let mut p = piece(x, far);
        p.stands_for = Some(stands_for);
        p.member_of = Some(of);
        p
    }

    #[test]
    fn a_coarser_stand_ins_members_all_come_in_when_one_does() {
        // Two stand-ins side by side (0, 1), the coarser one over both (2), a tile under each (3, 4).
        let mut coarse = piece(200.0, 3000.0);
        coarse.stands_for = Some(2);
        let mut tile_a = piece(0.0, 50.0);
        tile_a.member_of = Some(0);
        let mut tile_b = piece(400.0, 50.0);
        tile_b.member_of = Some(1);
        let mut set = RangedSet::new(
            vec![
                chained(0.0, 300.0, 0, 2),
                chained(400.0, 300.0, 1, 2),
                coarse,
                tile_a,
                tile_b,
            ],
            3,
            0.0,
        );
        set.enter(&[0, 1, 2, 3, 4]);
        assert_eq!(
            step(&mut set, 1000.0),
            (vec![2], vec![]),
            "far: the coarse one"
        );
        assert_eq!(
            step(&mut set, -200.0),
            (vec![0, 1], vec![2]),
            "one in reach: both, the far one 600 m off"
        );
        assert_eq!(
            step(&mut set, -400.0),
            (vec![2], vec![0, 1]),
            "neither in reach: the coarse one back"
        );
        assert_eq!(
            step(&mut set, -10.0),
            (vec![1, 3], vec![2]),
            "at a tile: it, and the other stand-in"
        );
    }

    #[test]
    fn a_sibling_with_no_members_still_keeps_out_of_its_near() {
        // A stand-in with a tile (0, 1), a hull (2) with no members and near 50, the coarser one (3).
        let mut tile = piece(0.0, 10.0);
        tile.member_of = Some(0);
        let mut hull = chained(500.0, 100.0, 2, 1);
        hull.near = 50.0;
        let mut coarse = piece(0.0, 3000.0);
        coarse.stands_for = Some(1);
        let mut set = RangedSet::new(vec![chained(0.0, 600.0, 0, 1), tile, hull, coarse], 3, 0.0);
        set.enter(&[0, 1, 2, 3]);
        assert_eq!(
            step(&mut set, 200.0),
            (vec![0, 2], vec![]),
            "the hull past its far, with its sibling"
        );
        assert_eq!(
            step(&mut set, 480.0),
            (vec![], vec![2]),
            "inside its near it hides"
        );
    }

    #[test]
    fn a_tile_showing_keeps_the_coarser_one_away_before_its_stand_ins_cell_is_in() {
        let mut set = chain(50.0);
        set.enter(&[0, 2]);
        assert_eq!(
            step(&mut set, 30.0),
            (vec![0], vec![]),
            "the coarse one stays away"
        );
    }

    #[test]
    fn leaving_forgets_a_pieces_state_and_entering_twice_counts_once() {
        let mut set = RangedSet::new(vec![piece(0.0, 10.0), piece(1.0, 10.0)], 0, 0.0);
        set.enter(&[0, 1]);
        set.enter(&[0]);
        assert_eq!(set.loaded(), 2);
        assert_eq!(step(&mut set, 0.0).0, vec![0, 1]);
        assert_eq!(set.leave(&[0]), vec![0]);
        assert_eq!(set.leave(&[0]), Vec::<u32>::new(), "already gone");
        assert_eq!(set.loaded(), 1);
        set.enter(&[0]);
        assert_eq!(step(&mut set, 0.0).0, vec![0], "added again");
    }
}
