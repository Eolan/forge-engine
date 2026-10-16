//! The server's side of replication: for each client, which entities to send, how, and when.
//!
//! Each tick the server hands the [`Replicator`] its world ([`Replicator::set_world`]): the
//! states are quantised and bucketed by cell. For each client, [`Replicator::snapshot`] then:
//!
//! 1. **interest** ([`crate::interest`]): the entities within [`ReplicationConfig::radius`] of
//!    the client's focus join its set and get a slot; one leaves once beyond the radius and its
//!    margin, or gone from the world, and the client is told to forget it;
//! 2. **priority**: every entity in the set adds its weight to its accumulated priority, its
//!    importance over `1 + (d / falloff)²` at `d` metres (Fiedler's priority accumulator,
//!    Tribes' scoped and prioritised updates, Halo: Reach's per-client priority);
//! 3. **selection under the budget** ([`ReplicationConfig::packet_budget`]): highest priority
//!    first, each entity costing what its block takes; an entity whose state is the one the
//!    client acknowledged costs nothing and is skipped, its priority cleared;
//! 4. **initial states first**: while an entity in the set has never had its full state sent
//!    (or its last one went unacknowledged for [`ReplicationConfig::initial_resend`] packets),
//!    full states fill the packet and no delta is sent. So every entity a client hears of gets
//!    its first state before any other entity's update: the order Verizon's US 11,810,245
//!    claims (after a join, an asset's initial data, then its updates, before another asset's
//!    initial data) never occurs. The owner is asked to confirm this reading
//!    (`docs/research/netcode.md`, §9);
//! 5. **deltas against acknowledged baselines**: an entity's block is a delta against the last
//!    state of it the client acknowledged, if that is within [`BASELINE_WINDOW`] packets, else a
//!    full state ([`crate::snapshot`]).
//!
//! Acknowledgements come back through [`Replicator::acknowledge`]. An entity that leaves a
//! client's interest and comes back is a new incarnation: acknowledgements of packets from
//! before it left give it no baseline, so it starts again from a full state. Everything is
//! iterated in id or slot order, so the same world, focuses and acks give the same bytes
//! (D-016).

use std::collections::{BTreeMap, BTreeSet};

use glam::DVec3;

use crate::bits::BitWriter;
use crate::interest::InterestGrid;
use crate::quantize::{EntityState, QState, Quantization};
use crate::snapshot::{
    Ack, BASELINE_WINDOW, Block, EntityId, Header, Seq, cell_of, gap_bits, newer, write_body,
    write_delta, write_entity_id, write_full, write_header,
};

/// How the replicator chooses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplicationConfig {
    /// The quantisation, both ends' alike.
    pub quantization: Quantization,
    /// Entities within this many metres of a client's focus join its interest.
    pub radius: f64,
    /// They leave it this many metres further out.
    pub margin: f64,
    /// The interest grid's cell, metres.
    pub grid_cell: f64,
    /// Bytes a snapshot may take (the datagram's payload).
    pub packet_budget: usize,
    /// The distance at which an entity's weight has halved, metres.
    pub falloff: f64,
    /// Packets after which an unacknowledged full state is sent again.
    pub initial_resend: u16,
    /// Removals a packet carries at most.
    pub max_removals: usize,
}

impl Default for ReplicationConfig {
    /// 150 m of interest (#250's near tier), cells of 32 m, 720 bytes a snapshot (D-010's
    /// 24 kB/s at 30 Hz once QUIC's and IPv4's headers are paid, `docs/demos/net.md`).
    fn default() -> Self {
        Self {
            quantization: Quantization::default(),
            radius: 150.0,
            margin: 10.0,
            grid_cell: 32.0,
            packet_budget: 720,
            falloff: 20.0,
            initial_resend: 8,
            max_removals: 64,
        }
    }
}

/// An entity of this tick's world.
#[derive(Clone, Copy, Debug)]
struct Entity {
    id: EntityId,
    q: QState,
    position: DVec3,
    importance: f32,
}

/// What the replicator knows of an entity for one client.
#[derive(Clone, Copy, Debug)]
struct Known {
    id: EntityId,
    slot: u32,
    incarnation: u32,
    priority: f32,
    /// The newest state the client acknowledged, and the packet it came in.
    baseline: Option<(Seq, QState)>,
    /// The last packet that carried its full state, while unacknowledged.
    initial_sent: Option<Seq>,
}

impl Known {
    fn was_sent(&self) -> bool {
        self.baseline.is_some() || self.initial_sent.is_some()
    }
}

/// A packet sent, kept until acknowledged or overwritten.
#[derive(Clone, Debug)]
struct Sent {
    seq: Seq,
    /// `(entity, incarnation, state)`.
    entities: Vec<(EntityId, u32, QState)>,
    removals: Vec<EntityId>,
    acked: bool,
}

/// Packets remembered for acknowledgements.
const SENT_RING: usize = 64;

/// One client's state at the replicator.
#[derive(Debug)]
struct ClientView {
    focus: DVec3,
    /// The entities it hears of, sorted by id.
    known: Vec<Known>,
    /// The next tick's `known`, kept for its storage.
    spare: Vec<Known>,
    removals: BTreeSet<EntityId>,
    sent: Vec<Option<Sent>>,
    next_seq: Seq,
    free_slots: BTreeSet<u32>,
    next_slot: u32,
    incarnations: u32,
}

impl ClientView {
    fn take_slot(&mut self) -> u32 {
        self.free_slots.pop_first().unwrap_or_else(|| {
            self.next_slot += 1;
            self.next_slot - 1
        })
    }

    fn find(&self, id: EntityId) -> Option<usize> {
        self.known.binary_search_by_key(&id, |k| k.id).ok()
    }
}

/// What one snapshot held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotStats {
    /// Its bytes.
    pub bytes: usize,
    /// Entities in the client's interest.
    pub interest: usize,
    /// Full states it carried.
    pub full: usize,
    /// Deltas it carried.
    pub deltas: usize,
    /// Removals it carried.
    pub removals: usize,
    /// Entities that changed but did not fit.
    pub left_out: usize,
    /// Entities waiting for their first state after this packet.
    pub waiting: usize,
}

/// The server's side of replication (see the module's documentation).
#[derive(Debug)]
pub struct Replicator {
    config: ReplicationConfig,
    grid: InterestGrid,
    entities: Vec<Entity>,
    clients: BTreeMap<u64, ClientView>,
    scratch: Vec<u32>,
    /// `(id, entity index, distance)` of the entities near a client, reused.
    near: Vec<(EntityId, u32, f64)>,
    /// A snapshot's block bodies end to end, reused.
    bodies: BitWriter,
}

/// An entity whose block competes for a packet.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    /// Its place in the client's `known`.
    known: usize,
    /// Its place in the world.
    index: u32,
    priority: f32,
    full: bool,
}

/// A block chosen for the packet: where its body lies in `bodies`.
#[derive(Clone, Copy, Debug)]
struct Chosen {
    known: usize,
    index: u32,
    slot: u32,
    baseline: u16,
    start: usize,
    len: usize,
}

impl Replicator {
    /// A replicator with no world and no client.
    pub fn new(config: ReplicationConfig) -> Self {
        Self {
            config,
            grid: InterestGrid::new(config.grid_cell),
            entities: Vec::new(),
            clients: BTreeMap::new(),
            scratch: Vec::new(),
            near: Vec::new(),
            bodies: BitWriter::new(),
        }
    }

    /// Its settings.
    pub fn config(&self) -> &ReplicationConfig {
        &self.config
    }

    /// Entities in this tick's world.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether this tick's world is empty.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// This tick's world: each entity's id, state and importance (1 for most; more for what
    /// matters more, such as the client's own avatar). Ids are unique.
    pub fn set_world<'a>(&mut self, world: impl Iterator<Item = (EntityId, &'a EntityState, f32)>) {
        let q = self.config.quantization;
        self.entities.clear();
        for (id, state, importance) in world {
            self.entities.push(Entity {
                id,
                q: q.quantize(state),
                position: state.position,
                importance,
            });
        }
        self.grid.rebuild(self.entities.iter().map(|e| e.position));
    }

    /// Adds a client looking from `focus`.
    pub fn add_client(&mut self, client: u64, focus: DVec3) {
        self.clients.insert(
            client,
            ClientView {
                focus,
                known: Vec::new(),
                spare: Vec::new(),
                removals: BTreeSet::new(),
                sent: vec![None; SENT_RING],
                next_seq: 0,
                free_slots: BTreeSet::new(),
                next_slot: 0,
                incarnations: 0,
            },
        );
    }

    /// Forgets a client.
    pub fn remove_client(&mut self, client: u64) {
        self.clients.remove(&client);
    }

    /// Moves a client's focus (its avatar or camera).
    pub fn set_focus(&mut self, client: u64, focus: DVec3) {
        if let Some(c) = self.clients.get_mut(&client) {
            c.focus = focus;
        }
    }

    /// Takes a client's acknowledgement: the states in the packets it names become baselines.
    pub fn acknowledge(&mut self, client: u64, ack: Ack) {
        let Some(c) = self.clients.get_mut(&client) else {
            return;
        };
        for seq in ack.sequences() {
            let slot = seq as usize % SENT_RING;
            let Some(sent) = c.sent[slot].as_mut() else {
                continue;
            };
            if sent.seq != seq || sent.acked {
                continue;
            }
            sent.acked = true;
            let sent = c.sent[slot].take().expect("present");
            for (id, incarnation, q) in &sent.entities {
                if let Some(k) = c.find(*id) {
                    let k = &mut c.known[k];
                    if k.incarnation == *incarnation
                        && k.baseline.is_none_or(|(b, _)| newer(seq, b))
                    {
                        k.baseline = Some((seq, *q));
                        k.initial_sent = None;
                    }
                }
            }
            for id in &sent.removals {
                // Forgotten, unless it came back in the meantime.
                if c.find(*id).is_none() {
                    c.removals.remove(id);
                }
            }
            c.sent[slot] = Some(Sent {
                entities: Vec::new(),
                removals: Vec::new(),
                ..sent
            });
        }
    }

    /// Brings a client's interest up to date: who joins (with a slot), who leaves (to be
    /// removed), each one's distance in `self.near`, aligned with `known`.
    fn update_interest(&mut self, client: u64) {
        let config = self.config;
        let c = self.clients.get_mut(&client).expect("a known client");
        self.grid
            .query(c.focus, config.radius + config.margin, &mut self.scratch);
        let near = &mut self.near;
        near.clear();
        for &k in &self.scratch {
            let e = &self.entities[k as usize];
            let d = e.position.distance(c.focus);
            if d <= config.radius + config.margin {
                near.push((e.id, k, d));
            }
        }
        near.sort_unstable_by_key(|e| e.0);
        // Merge the two sorted lists into the next `known`.
        let mut next = std::mem::take(&mut c.spare);
        next.clear();
        let old = std::mem::take(&mut c.known);
        let (mut i, mut j) = (0, 0);
        let mut kept = 0;
        while i < old.len() || j < near.len() {
            let take_old = j == near.len() || (i < old.len() && old[i].id < near[j].0);
            if take_old {
                // Out of reach or gone: forget it, and tell the client if it heard of it.
                c.free_slots.insert(old[i].slot);
                if old[i].was_sent() {
                    c.removals.insert(old[i].id);
                }
                i += 1;
            } else if i < old.len() && old[i].id == near[j].0 {
                next.push(old[i]);
                near[kept] = near[j];
                kept += 1;
                i += 1;
                j += 1;
            } else {
                let (id, index, d) = near[j];
                if d <= config.radius {
                    let slot = c.take_slot();
                    c.incarnations = c.incarnations.wrapping_add(1);
                    next.push(Known {
                        id,
                        slot,
                        incarnation: c.incarnations,
                        priority: 0.0,
                        baseline: None,
                        initial_sent: None,
                    });
                    near[kept] = (id, index, d);
                    kept += 1;
                    // Back before its removal went out: no removal.
                    c.removals.remove(&id);
                }
                j += 1;
            }
        }
        near.truncate(kept);
        c.spare = old;
        c.known = next;
    }

    /// The next snapshot for `client` at the server's `tick`, and what it held; `None` for an
    /// unknown client.
    pub fn snapshot(&mut self, client: u64, tick: u32) -> Option<(Vec<u8>, SnapshotStats)> {
        if !self.clients.contains_key(&client) {
            return None;
        }
        self.update_interest(client);
        let config = self.config;
        let q = config.quantization;
        let entities = &self.entities;
        let near = &self.near;
        let c = self.clients.get_mut(&client).expect("checked");
        let seq = c.next_seq;
        c.next_seq = c.next_seq.wrapping_add(1);
        let mut stats = SnapshotStats {
            interest: c.known.len(),
            ..SnapshotStats::default()
        };

        // Priorities, and who competes.
        let mut initial = Vec::new();
        let mut updates = Vec::new();
        for (k, (known, &(_, index, d))) in c.known.iter_mut().zip(near).enumerate() {
            let e = &entities[index as usize];
            let ratio = d / config.falloff;
            known.priority += e.importance / (1.0 + ratio * ratio) as f32;
            let candidate = |full| Candidate {
                known: k,
                index,
                priority: known.priority,
                full,
            };
            match known.baseline {
                None => {
                    let in_flight = known
                        .initial_sent
                        .is_some_and(|s| seq.wrapping_sub(s) < config.initial_resend);
                    if !in_flight {
                        initial.push(candidate(true));
                    }
                }
                // Too old a baseline: the client may no longer have it.
                Some((b, base)) if base != e.q => {
                    updates.push(candidate(seq.wrapping_sub(b) > BASELINE_WINDOW));
                }
                Some(_) => known.priority = 0.0,
            }
        }
        let known = &c.known;
        let by_priority = |a: &Candidate, b: &Candidate| {
            b.priority
                .total_cmp(&a.priority)
                .then(known[a.known].id.cmp(&known[b.known].id))
        };
        initial.sort_unstable_by(by_priority);
        updates.sort_unstable_by(by_priority);

        // The header and the removals come first; blocks fill what is left.
        let removals: Vec<EntityId> = c
            .removals
            .iter()
            .take(config.max_removals)
            .copied()
            .collect();
        let focus_steps = q
            .quantize(&EntityState {
                position: c.focus,
                ..EntityState::default()
            })
            .position;
        let header = Header {
            seq,
            tick,
            origin: cell_of(&q, &focus_steps),
        };
        let mut packet = BitWriter::new();
        write_header(&mut packet, &header);
        let mut tail = BitWriter::new();
        write_body(&mut tail, &BitWriter::new(), std::iter::empty(), &removals);
        let budget = config.packet_budget * 8;
        // The block count's varbits: three groups of 6 hold up to 2^18 blocks.
        let mut used = packet.bits() + tail.bits() + 21;
        let bodies = &mut self.bodies;
        *bodies = BitWriter::new();
        let mut chosen: Vec<Chosen> = Vec::new();
        let mut slots_taken = BTreeSet::new();
        let mut misses = 0;
        let mut initial_sent = 0;
        let lists: [&[Candidate]; 2] = [&initial, &updates];
        for (list, candidates) in lists.into_iter().enumerate() {
            if list == 1 && initial_sent < initial.len() {
                // Initial states first: no delta while one waits.
                stats.left_out += candidates.len();
                break;
            }
            for cand in candidates {
                if misses >= 16 {
                    stats.left_out += 1;
                    continue;
                }
                let e = &entities[cand.index as usize];
                let known = &known[cand.known];
                let start = bodies.bits();
                let baseline = match known.baseline {
                    Some((b, base)) if !cand.full => {
                        write_delta(bodies, &q, &e.q, &base, &header.origin);
                        seq.wrapping_sub(b)
                    }
                    _ => {
                        write_entity_id(bodies, known.id);
                        write_full(bodies, &q, &e.q, &header.origin);
                        0
                    }
                };
                let len = bodies.bits() - start;
                // The slot's gap from the block before it once sorted: at most this, since
                // the gap of the block after it can only shrink.
                let before = slots_taken.range(..known.slot).next_back();
                let gap = before.map_or(known.slot, |b| known.slot - b - 1);
                let cost = gap_bits(gap) + 5 + len;
                if used + cost > budget {
                    misses += 1;
                    stats.left_out += 1;
                    continue;
                }
                used += cost;
                if list == 0 {
                    initial_sent += 1;
                }
                slots_taken.insert(known.slot);
                chosen.push(Chosen {
                    known: cand.known,
                    index: cand.index,
                    slot: known.slot,
                    baseline,
                    start,
                    len,
                });
            }
        }
        stats.waiting = initial.len() - initial_sent;
        chosen.sort_unstable_by_key(|b| b.slot);

        // The packet, and what it carried.
        let blocks = chosen.iter().map(|b| Block {
            slot: b.slot,
            baseline: b.baseline,
            start: b.start,
            len: b.len,
        });
        write_body(&mut packet, bodies, blocks, &removals);
        let mut carried = Vec::with_capacity(chosen.len());
        for b in &chosen {
            let known = &mut c.known[b.known];
            known.priority = 0.0;
            if b.baseline == 0 {
                stats.full += 1;
                if known.baseline.is_none() {
                    known.initial_sent = Some(seq);
                }
            } else {
                stats.deltas += 1;
            }
            carried.push((known.id, known.incarnation, entities[b.index as usize].q));
        }
        stats.removals = removals.len();
        c.sent[seq as usize % SENT_RING] = Some(Sent {
            seq,
            entities: carried,
            removals,
            acked: false,
        });
        let bytes = packet.finish();
        stats.bytes = bytes.len();
        Some((bytes, stats))
    }
}
