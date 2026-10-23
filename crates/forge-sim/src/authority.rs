//! Authority over a world split between workers (#268, step 2's handoff; D-010): the
//! replication layer holds every entity's state, the worker that owns it and an epoch; workers
//! hold nothing of their own between ticks.
//!
//! - **[`Strips`]:** the world cut into regions along x, one a worker (D-010's first stage: static
//!   regions; splitting and merging them by load comes later). An entity stays with its owner
//!   until it is [`Strips::margin`] past the owner's border, so one walking along a border does not
//!   change hands every tick.
//! - **[`Layer`]:** the replication layer. Each tick it takes the owners' writes, each carrying the
//!   epoch its writer was granted, and refuses a write whose epoch is not the record's (a fencing
//!   token: a worker that has not yet heard that authority moved on writes in vain). At the tick's
//!   end it hands over every entity past its owner's border by the margin, the epoch one more:
//!   the handoff takes effect at the next tick's barrier, so it costs no tick.
//! - **[`Worker`]:** at each tick's barrier it takes from the layer the entities it owns, with their
//!   epochs, and the others within [`Strips::band`] of its region as read-only ghosts, then steps
//!   its own from that picture. A worker killed and started again takes the same picture: it holds
//!   nothing the layer does not. A worker gone can be covered: the layer gives its region to
//!   another ([`Layer::reassign`]) and back.
//!
//! A step reads the world as the last tick left it (each entity from its own state and its
//! neighbours' at the barrier): with a band as wide as the reach of what an entity reads, the split
//! world steps exactly as one worker holding everything would. Patents: `docs/research/netcode.md`
//! §9 (the search for authority handoff, 2026-10-23).

use std::collections::BTreeMap;

use glam::DVec3;

/// A worker's number.
pub type WorkerId = u16;

/// An entity's id in the replication layer.
pub type EntityId = u32;

/// What a worker simulates: a state with a place.
pub trait Placed: Clone + PartialEq {
    /// Where the entity stands, metres.
    fn position(&self) -> DVec3;
}

/// The world cut along x into strips, worker `k` owning the `k`-th (see the module's
/// documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct Strips {
    /// Where the strips meet along x, increasing: `n` cuts make `n + 1` strips.
    pub cuts: Vec<f64>,
    /// How far past its owner's border an entity goes before it changes hands, metres.
    pub margin: f64,
    /// How far beyond its region a worker sees the others' entities, metres: the reach of what an
    /// entity reads of its neighbours, and the margin.
    pub band: f64,
}

impl Strips {
    /// How many workers the strips need.
    pub fn workers(&self) -> usize {
        self.cuts.len() + 1
    }

    /// The worker whose strip holds `position`.
    pub fn region(&self, position: DVec3) -> WorkerId {
        self.cuts.iter().filter(|&&c| position.x >= c).count() as WorkerId
    }

    /// How far `position` lies outside `worker`'s strip, metres; 0 inside it.
    pub fn outside(&self, worker: WorkerId, position: DVec3) -> f64 {
        let k = usize::from(worker);
        let low = k.checked_sub(1).map_or(f64::NEG_INFINITY, |i| self.cuts[i]);
        let high = self.cuts.get(k).copied().unwrap_or(f64::INFINITY);
        (low - position.x).max(position.x - high).max(0.0)
    }
}

/// An entity as the replication layer holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Record<S> {
    /// Its state.
    pub state: S,
    /// The worker that owns it.
    pub owner: WorkerId,
    /// How many times authority over it has moved: a write must carry it.
    pub epoch: u32,
}

/// A write refused: its epoch is not the record's (authority moved on), or the entity is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stale {
    /// The entity.
    pub id: EntityId,
    /// The epoch the write carried.
    pub epoch: u32,
}

/// Authority over one entity moved from a worker to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handoff {
    /// The entity.
    pub id: EntityId,
    /// Its owner before.
    pub from: WorkerId,
    /// Its owner now.
    pub to: WorkerId,
    /// Its epoch now.
    pub epoch: u32,
}

/// The replication layer: every entity's record (see the module's documentation).
#[derive(Clone, Debug)]
pub struct Layer<S> {
    records: BTreeMap<EntityId, Record<S>>,
    /// Regions given to another worker while theirs is away ([`Layer::reassign`]).
    covered: BTreeMap<WorkerId, WorkerId>,
    /// Writes refused since the start.
    refused: u64,
}

impl<S: Placed> Layer<S> {
    /// A layer holding `entities`, each owned by the worker whose region holds it, at epoch 0.
    pub fn new(strips: &Strips, entities: impl IntoIterator<Item = (EntityId, S)>) -> Self {
        Self {
            records: entities
                .into_iter()
                .map(|(id, state)| {
                    let owner = strips.region(state.position());
                    (
                        id,
                        Record {
                            state,
                            owner,
                            epoch: 0,
                        },
                    )
                })
                .collect(),
            covered: BTreeMap::new(),
            refused: 0,
        }
    }

    /// Every record, by id.
    pub fn records(&self) -> &BTreeMap<EntityId, Record<S>> {
        &self.records
    }

    /// Writes refused since the start.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// `worker`'s write of entity `id` at `epoch`: taken when the epoch is the record's and the
    /// writer its owner, refused otherwise.
    pub fn write(
        &mut self,
        worker: WorkerId,
        id: EntityId,
        epoch: u32,
        state: S,
    ) -> Result<(), Stale> {
        match self.records.get_mut(&id) {
            Some(r) if r.epoch == epoch && r.owner == worker => {
                r.state = state;
                Ok(())
            }
            _ => {
                self.refused += 1;
                Err(Stale { id, epoch })
            }
        }
    }

    /// The worker that owns region `region` now: its own, or the one covering it.
    pub fn owner_of_region(&self, region: WorkerId) -> WorkerId {
        self.covered.get(&region).copied().unwrap_or(region)
    }

    /// At the tick's end: every entity past its owner's region by the margin goes to the owner of
    /// the region it stands in, its epoch one more. In id order.
    pub fn rebalance(&mut self, strips: &Strips) -> Vec<Handoff> {
        let mut handoffs = Vec::new();
        let covered = &self.covered;
        let owner_of = |region: WorkerId| covered.get(&region).copied().unwrap_or(region);
        for (&id, r) in &mut self.records {
            let position = r.state.position();
            let region = strips.region(position);
            let to = owner_of(region);
            if to == r.owner {
                continue;
            }
            // Its owner keeps it within the margin of one of the regions it holds.
            let held = (0..strips.workers() as WorkerId)
                .filter(|&w| owner_of(w) == r.owner)
                .any(|w| strips.outside(w, position) <= strips.margin);
            if held {
                continue;
            }
            r.epoch += 1;
            handoffs.push(Handoff {
                id,
                from: r.owner,
                to,
                epoch: r.epoch,
            });
            r.owner = to;
        }
        handoffs
    }

    /// Gives region `away`'s entities, and the region, to worker `to` (a worker gone; `to ==
    /// away` gives them back), each handed over with its epoch one more. In id order.
    pub fn reassign(&mut self, strips: &Strips, away: WorkerId, to: WorkerId) -> Vec<Handoff> {
        if to == away {
            self.covered.remove(&away);
        } else {
            self.covered.insert(away, to);
        }
        let covered = &self.covered;
        let owner_of = |region: WorkerId| covered.get(&region).copied().unwrap_or(region);
        let mut handoffs = Vec::new();
        for (&id, r) in &mut self.records {
            let to = owner_of(strips.region(r.state.position()));
            // The region's entities, and the gone worker's wherever they stand.
            let moves =
                (strips.region(r.state.position()) == away || r.owner == away) && to != r.owner;
            if moves {
                r.epoch += 1;
                handoffs.push(Handoff {
                    id,
                    from: r.owner,
                    to,
                    epoch: r.epoch,
                });
                r.owner = to;
            }
        }
        handoffs
    }
}

/// A worker: the entities it owns and those it sees, taken from the layer at each tick's barrier
/// (see the module's documentation).
#[derive(Clone, Debug)]
pub struct Worker<S> {
    id: WorkerId,
    /// Its own entities, with the epochs it was granted.
    owned: BTreeMap<EntityId, (u32, S)>,
    /// The others it sees, read-only.
    ghosts: BTreeMap<EntityId, S>,
}

impl<S: Placed> Worker<S> {
    /// Worker `id`, holding nothing yet.
    pub fn new(id: WorkerId) -> Self {
        Self {
            id,
            owned: BTreeMap::new(),
            ghosts: BTreeMap::new(),
        }
    }

    /// Its number.
    pub fn id(&self) -> WorkerId {
        self.id
    }

    /// The entities it owns, with their epochs.
    pub fn owned(&self) -> &BTreeMap<EntityId, (u32, S)> {
        &self.owned
    }

    /// The others it sees.
    pub fn ghosts(&self) -> &BTreeMap<EntityId, S> {
        &self.ghosts
    }

    /// At the tick's barrier: takes from `layer` the entities it owns, with their epochs, and as
    /// ghosts the others within the band of a region it holds.
    pub fn sync(&mut self, layer: &Layer<S>, strips: &Strips) {
        self.owned.clear();
        self.ghosts.clear();
        let regions: Vec<WorkerId> = (0..strips.workers() as WorkerId)
            .filter(|&w| layer.owner_of_region(w) == self.id)
            .collect();
        for (&id, r) in layer.records() {
            if r.owner == self.id {
                self.owned.insert(id, (r.epoch, r.state.clone()));
            } else if regions
                .iter()
                .any(|&w| strips.outside(w, r.state.position()) <= strips.band)
            {
                self.ghosts.insert(id, r.state.clone());
            }
        }
    }

    /// Takes back an entity a revoke has not reached yet: it keeps writing it at its old epoch,
    /// which the layer refuses (a late revoke, for the tests of the fencing).
    pub fn keep(&mut self, id: EntityId, epoch: u32, state: S) {
        self.owned.insert(id, (epoch, state));
        self.ghosts.remove(&id);
    }

    /// Steps each entity it owns with `step(id, state, world)`, the world being its picture at the
    /// barrier (its own and its ghosts, by id): the writes, each with its epoch, in id order.
    pub fn step(
        &self,
        mut step: impl FnMut(EntityId, &S, &BTreeMap<EntityId, &S>) -> S,
    ) -> Vec<(EntityId, u32, S)> {
        let world: BTreeMap<EntityId, &S> = self
            .owned
            .iter()
            .map(|(&id, (_, s))| (id, s))
            .chain(self.ghosts.iter().map(|(&id, s)| (id, s)))
            .collect();
        self.owned
            .iter()
            .map(|(&id, (epoch, s))| (id, *epoch, step(id, s, &world)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct At(DVec3);

    impl Placed for At {
        fn position(&self) -> DVec3 {
            self.0
        }
    }

    /// An entity changes hands only past the margin, its epoch one more; a write at the old epoch
    /// is refused, one at the new taken.
    #[test]
    fn authority_moves_past_the_margin_and_fences_the_old_owner() {
        let strips = Strips {
            cuts: vec![0.0],
            margin: 2.0,
            band: 3.0,
        };
        assert_eq!(strips.region(DVec3::new(-0.1, 0.0, 0.0)), 0);
        assert_eq!(strips.region(DVec3::new(0.0, 0.0, 0.0)), 1);
        assert_eq!(strips.outside(0, DVec3::new(1.5, 0.0, 0.0)), 1.5);
        assert_eq!(strips.outside(1, DVec3::new(1.5, 0.0, 0.0)), 0.0);
        let mut layer = Layer::new(&strips, [(5, At(DVec3::new(-1.0, 0.0, 0.0)))]);
        assert_eq!(layer.records()[&5].owner, 0);
        layer.write(0, 5, 0, At(DVec3::new(1.5, 0.0, 0.0))).unwrap();
        assert!(layer.rebalance(&strips).is_empty(), "within the margin");
        layer.write(0, 5, 0, At(DVec3::new(2.5, 0.0, 0.0))).unwrap();
        let handoffs = layer.rebalance(&strips);
        assert_eq!(
            handoffs,
            vec![Handoff {
                id: 5,
                from: 0,
                to: 1,
                epoch: 1
            }]
        );
        assert_eq!(
            layer.write(0, 5, 0, At(DVec3::ZERO)),
            Err(Stale { id: 5, epoch: 0 })
        );
        assert_eq!(layer.refused(), 1);
        layer.write(1, 5, 1, At(DVec3::new(3.0, 0.0, 0.0))).unwrap();
        // The new owner sees it as its own, the old as a ghost within its band.
        let (mut w0, mut w1) = (Worker::new(0), Worker::new(1));
        w0.sync(&layer, &strips);
        w1.sync(&layer, &strips);
        assert!(w1.owned().contains_key(&5));
        assert!(w0.ghosts().contains_key(&5));
    }
}
