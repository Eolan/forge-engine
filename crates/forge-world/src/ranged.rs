//! Pieces shown by their own ranges (D-106): an authored world's placed pieces each say how far
//! they show and from which point that is measured, and a low-detail stand-in replaces a group of
//! them. Cells ([`crate::streaming`]) stay the unit of loading; inside the cells in, this set tests
//! every piece against the viewer and says which come and go.
//!
//! Each piece is resolved once into [`Crossing`]s, spheres round a point: the one inside which it
//! comes, and for a stand-in the one inside which it goes.
//!
//! - **A piece** comes within its `far` of its `reference`, a point that need not be its own place.
//! - **A group** swaps as one: its members and its stand-in share a crossing, about the middle of
//!   the members' references and wide enough for every member's own range. No group is drawn in
//!   part or twice, and a group whose reach stops short of the stand-in's near distance has no gap.
//! - **A stand-in with no members in the world** goes inside its own `near`.
//! - **Chains:** a stand-in may itself be a member of a coarser group, and comes with its siblings
//!   at that group's crossing. A coarser group's crossing reaches at least as far as its members'
//!   own groups', so a coarser stand-in never shows beside finer pieces.
//!
//! A crossing has a width, its band ([`FadeBand`]): the renderer dissolves one side into the other
//! across it from the camera's position each frame, so both must be in for that stretch. A piece
//! comes `lead` metres before the band's outer edge and a stand-in goes that far past its inner
//! edge, undrawn there: the band's first pixels are not waiting on the streamer. A piece shown keeps
//! showing `slack` metres past those, so a viewer standing on one does not flip it.
//!
//! A cell not yet in holds its pieces back while the stand-in over them goes as the viewer nears: a
//! moment of nothing after a jump across the map, until the worker's next edit.

use std::collections::HashMap;

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
    /// The group it is a member of: it comes at that group's crossing.
    pub member_of: Option<GroupId>,
    /// The group it stands in for.
    pub stands_for: Option<GroupId>,
}

/// How wide the bands of a set's crossings are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeBand {
    /// The band as a share of the crossing's distance.
    pub share: f32,
    /// The narrowest band, metres.
    pub least: f32,
    /// The widest band, metres.
    pub most: f32,
    /// How far ahead of the band a piece is brought in, metres: at least what the camera covers
    /// before the streamer's edit is on screen.
    pub lead: f32,
}

impl FadeBand {
    /// No band: every crossing a hard cut.
    pub const NONE: Self = Self {
        share: 0.0,
        least: 0.0,
        most: 0.0,
        lead: 0.0,
    };

    /// The band of a crossing at `distance`, never wider than it: a piece with a very short
    /// range still becomes whole near its point.
    pub fn of(self, distance: f32) -> f32 {
        (distance * self.share)
            .clamp(self.least, self.most)
            .min(distance.max(0.0))
    }
}

/// A sphere a piece comes or goes at: inside it with the viewer, the piece's side of a swap has
/// taken over, across `band` metres centred on `distance`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    /// The point it is measured from, metres in the partition's frame.
    pub centre: DVec3,
    /// How far from it the crossing is half done, metres.
    pub distance: f32,
    /// How wide it is, metres.
    pub band: f32,
    /// How far ahead of the band a piece is in the scene, metres.
    pub lead: f32,
}

impl Crossing {
    fn at(centre: DVec3, distance: f32, band: FadeBand) -> Self {
        Self {
            centre,
            distance,
            band: band.of(distance),
            lead: band.lead,
        }
    }

    /// How far from its point the piece is first needed, metres: the band's outer edge and the
    /// lead.
    pub fn reach(&self) -> f64 {
        f64::from(self.distance) + 0.5 * f64::from(self.band) + f64::from(self.lead)
    }

    /// Whether `viewer` is inside the reach, `extra` metres further out.
    fn reaches(&self, viewer: DVec3, extra: f64) -> bool {
        let edge = self.reach() + extra;
        self.centre.distance_squared(viewer) <= edge * edge
    }

    /// Whether `viewer` is outside the inner edge and the lead, `extra` metres further in.
    fn clear_of(&self, viewer: DVec3, extra: f64) -> bool {
        let edge =
            f64::from(self.distance) - 0.5 * f64::from(self.band) - f64::from(self.lead) - extra;
        edge <= 0.0 || self.centre.distance_squared(viewer) >= edge * edge
    }
}

const LOADED: u8 = 1;
const SHOWN: u8 = 2;
/// The piece has a crossing it goes at, in [`RangedSet::goes`].
const GOES: u8 = 4;
/// The piece swaps with others: hard cuts give it no slack, or both sides would show in the ring.
const SWAPS: u8 = 8;
/// What [`RangedSet::leave`] keeps of a piece's bits.
const LASTING: u8 = GOES | SWAPS;
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
    /// Per piece, the crossing it comes at.
    comes: Vec<Crossing>,
    /// By piece, the crossing it goes at: only stand-ins have one.
    goes: HashMap<u32, Crossing>,
    /// Per piece, its `LOADED`, `SHOWN`, `GOES` and `SWAPS` bits.
    flags: Vec<u8>,
    /// The pieces in, and per piece its index there.
    loaded: Vec<u32>,
    slots: Vec<u32>,
    slack: f64,
    /// The slack of a piece that swaps with others: none with hard cuts.
    swap_slack: f64,
    add: Vec<u32>,
    remove: Vec<u32>,
}

impl RangedSet {
    /// A set over `pieces` in `groups` groups, none in yet, its crossings `band` wide; a piece
    /// shown keeps showing `slack` metres past its limits.
    ///
    /// # Panics
    ///
    /// If a piece names a group at or past `groups`.
    pub fn new(pieces: Vec<RangedPiece>, groups: u32, slack: f64, band: FadeBand) -> Self {
        let n = groups as usize;
        let mut members = vec![0u32; n];
        let mut sum = vec![DVec3::ZERO; n];
        for p in &pieces {
            assert!(
                p.stands_for.is_none_or(|g| g < groups),
                "a stand-in names a group past the last"
            );
            if let Some(g) = p.member_of {
                members[g as usize] += 1;
                sum[g as usize] += p.reference;
            }
        }
        let centre: Vec<DVec3> = sum
            .iter()
            .zip(&members)
            .map(|(&s, &m)| s / f64::from(m.max(1)))
            .collect();
        // Far enough for every member's own range.
        let mut reach = vec![0.0f64; n];
        for p in &pieces {
            if let Some(g) = p.member_of {
                let needed = f64::from(p.far) + p.reference.distance(centre[g as usize]);
                reach[g as usize] = reach[g as usize].max(needed);
            }
        }
        // A coarser group at least as far as its members' own groups: each pass settles a level.
        let chained: Vec<(usize, usize)> = pieces
            .iter()
            .filter_map(|p| Some((p.stands_for? as usize, p.member_of? as usize)))
            .filter(|&(own, _)| members[own] > 0)
            .collect();
        for _ in 0..=chained.len() {
            let mut changed = false;
            for &(own, parent) in &chained {
                let needed = reach[own] + centre[own].distance(centre[parent]);
                if needed > reach[parent] {
                    reach[parent] = needed;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let of_group = |g: u32| Crossing::at(centre[g as usize], reach[g as usize] as f32, band);
        let mut goes = HashMap::new();
        let mut flags = vec![0u8; pieces.len()];
        let comes = pieces
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let going = match p.stands_for {
                    Some(g) if members[g as usize] > 0 => Some(of_group(g)),
                    Some(_) if p.near > 0.0 => Some(Crossing::at(p.reference, p.near, band)),
                    _ => None,
                };
                if p.member_of.is_some() || p.stands_for.is_some() {
                    flags[i] = SWAPS;
                }
                if let Some(crossing) = going {
                    goes.insert(i as u32, crossing);
                    flags[i] |= GOES;
                }
                match p.member_of {
                    Some(g) => of_group(g),
                    None => Crossing::at(p.reference, p.far, band),
                }
            })
            .collect::<Vec<_>>();
        let count = comes.len();
        Self {
            comes,
            goes,
            flags,
            loaded: Vec::new(),
            slots: vec![NO_SLOT; count],
            slack,
            swap_slack: if band == FadeBand::NONE { 0.0 } else { slack },
            add: Vec::new(),
            remove: Vec::new(),
        }
    }

    /// How many pieces the world has.
    pub fn len(&self) -> usize {
        self.comes.len()
    }

    /// Whether the world has no pieces.
    pub fn is_empty(&self) -> bool {
        self.comes.is_empty()
    }

    /// How many pieces are in cells that are in.
    pub fn loaded(&self) -> usize {
        self.loaded.len()
    }

    /// Whether piece `piece` shows.
    pub fn is_shown(&self, piece: u32) -> bool {
        self.flags[piece as usize] & SHOWN != 0
    }

    /// The crossing piece `piece` comes at, and the one it goes at if it is a stand-in that does.
    pub fn crossings(&self, piece: u32) -> (Crossing, Option<Crossing>) {
        (self.comes[piece as usize], self.goes.get(&piece).copied())
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
            self.flags[i] &= LASTING;
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
        for &p in &self.loaded {
            let i = p as usize;
            let bits = self.flags[i];
            let shown = bits & SHOWN != 0;
            let extra = match (shown, bits & SWAPS != 0) {
                (false, _) => 0.0,
                (true, false) => self.slack,
                (true, true) => self.swap_slack,
            };
            let want = self.comes[i].reaches(viewer, extra)
                && (bits & GOES == 0 || self.goes[&p].clear_of(viewer, extra));
            if want != shown {
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

    /// A set with hard cuts.
    fn hard(pieces: Vec<RangedPiece>, groups: u32, slack: f64) -> RangedSet {
        RangedSet::new(pieces, groups, slack, FadeBand::NONE)
    }

    /// (added, removed) at `x`, sorted.
    fn step(set: &mut RangedSet, x: f64) -> (Vec<u32>, Vec<u32>) {
        let c = set.refresh(at(x));
        (sorted(c.add.to_vec()), sorted(c.remove.to_vec()))
    }

    #[test]
    fn a_piece_shows_within_its_range_and_only_once_in() {
        let mut set = hard(vec![piece(0.0, 10.0)], 0, 0.0);
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
        let mut set = hard(vec![piece(0.0, 10.0)], 0, 2.0);
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
        let mut set = hard(vec![p], 0, 0.0);
        set.enter(&[0]);
        assert_eq!(step(&mut set, 0.0).0, Vec::<u32>::new(), "11.3 m away");
    }

    #[test]
    fn a_stand_in_with_no_members_obeys_its_near_distance() {
        let mut p = piece(0.0, 100.0);
        p.near = 20.0;
        p.stands_for = Some(0);
        let mut set = hard(vec![p], 1, 0.0);
        set.enter(&[0]);
        assert_eq!(step(&mut set, 10.0), (vec![], vec![]), "closer than near");
        assert_eq!(step(&mut set, 30.0), (vec![0], vec![]));
        assert_eq!(step(&mut set, 19.0), (vec![], vec![0]));
    }

    /// Pieces 0 and 1 and their stand-in 2, in the group 0.
    fn group_pieces(stand_near: f32, member_far: f32) -> Vec<RangedPiece> {
        let mut a = piece(0.0, member_far);
        let mut b = piece(0.0, member_far);
        // The second member's point lies 10 m further on.
        b.reference.x = 10.0;
        a.member_of = Some(0);
        b.member_of = Some(0);
        let mut s = piece(0.0, 1000.0);
        s.near = stand_near;
        s.stands_for = Some(0);
        vec![a, b, s]
    }

    fn group(stand_near: f32, member_far: f32) -> RangedSet {
        hard(group_pieces(stand_near, member_far), 1, 0.0)
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
    fn a_group_reaches_as_far_as_its_farthest_member() {
        let mut set = group(40.0, 50.0);
        set.enter(&[0, 1, 2]);
        // 55 m from the first member's point, 45 m from the second's, 50 m from their middle.
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
    fn hard_cuts_show_only_one_side_of_a_swap_whatever_the_slack() {
        let mut set = hard(group_pieces(40.0, 50.0), 1, 2.0);
        set.enter(&[0, 1, 2]);
        // The crossing is 55 m from x = 5.
        assert_eq!(step(&mut set, 61.0), (vec![2], vec![]), "outside");
        assert_eq!(step(&mut set, 59.5), (vec![0, 1], vec![2]), "54.5 m in");
        assert_eq!(step(&mut set, 60.5), (vec![2], vec![0, 1]), "55.5 m out");
    }

    /// The band the fade tests use: a tenth of the distance, 8 to 40 m, `lead` metres ahead.
    fn band(lead: f32) -> FadeBand {
        FadeBand {
            share: 0.1,
            least: 8.0,
            most: 40.0,
            lead,
        }
    }

    #[test]
    fn a_group_and_its_stand_in_share_one_crossing() {
        let set = RangedSet::new(group_pieces(40.0, 50.0), 1, 0.0, band(0.0));
        let (member, none) = set.crossings(0);
        let (other, _) = set.crossings(1);
        let (_, going) = set.crossings(2);
        assert_eq!(none, None, "a member does not go");
        assert_eq!(member, other, "the members together");
        assert_eq!(Some(member), going, "and the stand-in with them");
        assert_eq!(member.centre, at(5.0), "about the middle of the members");
        assert_eq!(member.distance, 55.0, "far enough for the farther one");
        assert_eq!(member.band, 8.0, "the least band");
    }

    #[test]
    fn both_sides_are_in_across_the_band() {
        let mut set = RangedSet::new(group_pieces(40.0, 50.0), 1, 0.0, band(0.0));
        set.enter(&[0, 1, 2]);
        // The crossing is 55 m from x = 5, 8 m wide.
        assert_eq!(step(&mut set, 70.0), (vec![2], vec![]), "outside the band");
        assert_eq!(
            step(&mut set, 62.0),
            (vec![0, 1], vec![]),
            "in the band: the members come, the stand-in stays"
        );
        assert_eq!(
            step(&mut set, 55.0),
            (vec![], vec![2]),
            "past the inner edge the stand-in goes"
        );
    }

    #[test]
    fn the_lead_brings_a_side_in_before_its_band_and_keeps_it_after() {
        let mut set = RangedSet::new(group_pieces(40.0, 50.0), 1, 0.0, band(8.0));
        set.enter(&[0, 1, 2]);
        // The crossing is 55 m from x = 5, 8 m wide: the band from 51 to 59, in from 67 to 43.
        assert_eq!(
            step(&mut set, 74.0),
            (vec![2], vec![]),
            "69 m: the stand-in"
        );
        assert_eq!(
            step(&mut set, 70.0),
            (vec![0, 1], vec![]),
            "65 m, before the band: the members are in, undrawn"
        );
        assert_eq!(
            step(&mut set, 50.0),
            (vec![], vec![]),
            "45 m, past the band: the stand-in stays, undrawn"
        );
        assert_eq!(step(&mut set, 47.0), (vec![], vec![2]), "42 m: it goes");
    }

    #[test]
    fn a_band_is_clamped_and_a_short_range_still_becomes_whole() {
        let band = band(0.0);
        assert_eq!(band.of(1000.0), 40.0);
        assert_eq!(band.of(200.0), 20.0);
        assert_eq!(band.of(50.0), 8.0);
        assert_eq!(band.of(3.0), 3.0, "at most the distance");
        assert_eq!(FadeBand::NONE.of(500.0), 0.0);
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
        let mut set = hard(vec![tile, fine, coarse], 2, 0.0);
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
        hard(vec![tile, fine, coarse], 2, 0.0)
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

    #[test]
    fn a_tile_reaching_past_its_chain_keeps_the_coarser_one_away() {
        // The tile reaches 500 m, past the fine one's 300: the coarse one goes at 500 too.
        let mut set = chain(500.0);
        set.enter(&[0, 1, 2]);
        assert_eq!(
            step(&mut set, 400.0),
            (vec![0], vec![]),
            "the tile alone: no stand-in over it"
        );
        assert_eq!(step(&mut set, 600.0), (vec![2], vec![0]));
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
        let mut set = hard(
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
            "out of reach: the coarse one back"
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
        let mut set = hard(vec![chained(0.0, 600.0, 0, 1), tile, hull, coarse], 3, 0.0);
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
    fn leaving_forgets_a_pieces_state_and_entering_twice_counts_once() {
        let mut set = hard(vec![piece(0.0, 10.0), piece(1.0, 10.0)], 0, 0.0);
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

    #[test]
    fn leaving_keeps_what_a_stand_in_goes_at() {
        let mut set = group(40.0, 50.0);
        set.enter(&[2]);
        assert_eq!(step(&mut set, 500.0), (vec![2], vec![]));
        assert_eq!(set.leave(&[2]), vec![2]);
        set.enter(&[2]);
        assert_eq!(
            step(&mut set, 30.0),
            (vec![], vec![]),
            "still goes inside the group's crossing"
        );
    }
}
