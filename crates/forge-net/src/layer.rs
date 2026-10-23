//! The replication layer and its workers over the transport (#268, step 2's handoff; D-010):
//! [`forge_sim::authority`]'s layer on a server, each worker a client.
//!
//! Each tick, in lockstep:
//! 1. The layer sends each worker its picture of the tick ([`Layer::picture`]): the entities it
//!    owns with their epochs, and the ghosts within its band. One message a tick, all of it.
//! 2. Each worker steps its own from that picture and sends its writes back ([`serve`]).
//! 3. Once every worker has answered, the layer takes the writes (each at its epoch) and hands
//!    over the entities past their owner's border. The handoffs take effect with the next
//!    pictures: no notice goes ahead of them (`docs/research/netcode.md` §9).
//!
//! A worker whose session closes, or that stays silent past the host's patience, is covered: the
//! lowest-numbered worker left takes its region ([`Layer::reassign`]), the tick's writes are set
//! aside, and the tick is stepped again from the same pictures. No tick is lost: a step reads only
//! the picture, so the covering worker steps the gone worker's entities exactly as it would have.
//! A worker that comes back is attached again ([`LayerHost::attach`]) and takes its region back.
//!
//! The messages, on [`CHANNEL`] (reliable, ordered), little-endian:
//! - a picture: the frame (u64, one more each picture the layer sends), the tick (u64), the count
//!   (u32), then each entity: its id (u32), 1 and its epoch (u32) when owned or 0 for a ghost, its
//!   state ([`Codec`]);
//! - writes: the frame they answer (u64), the tick (u64), the count (u32), then each: the id, the
//!   epoch, the state.
//!
//! A worker's answer to an earlier frame (a tick stepped again) is recognised by its frame and
//! dropped.

use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use forge_sim::Codec;
use forge_sim::authority::{EntityId, Handoff, Layer, Placed, Seen, Strips, WorkerId};

use crate::channel::ChannelId;
use crate::session::Session;

/// The channel the layer and its workers talk on: the first, reliable and ordered.
pub const CHANNEL: ChannelId = ChannelId(0);

/// A worker's writes of a tick: each entity it owns, the epoch it was granted, its new state.
pub type Writes<S> = Vec<(EntityId, u32, S)>;

fn take_u32(bytes: &mut &[u8]) -> Option<u32> {
    let (head, rest) = bytes.split_first_chunk::<4>()?;
    *bytes = rest;
    Some(u32::from_le_bytes(*head))
}

fn take_u64(bytes: &mut &[u8]) -> Option<u64> {
    let (head, rest) = bytes.split_first_chunk::<8>()?;
    *bytes = rest;
    Some(u64::from_le_bytes(*head))
}

/// A picture's bytes (see the module's documentation).
pub fn encode_picture<S: Codec>(frame: u64, tick: u64, picture: &[Seen<S>]) -> Bytes {
    let mut out = Vec::with_capacity(20 + picture.len() * 64);
    out.extend_from_slice(&frame.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&(picture.len() as u32).to_le_bytes());
    for seen in picture {
        out.extend_from_slice(&seen.id.to_le_bytes());
        match seen.epoch {
            Some(epoch) => {
                out.push(1);
                out.extend_from_slice(&epoch.to_le_bytes());
            }
            None => out.push(0),
        }
        seen.state.encode(&mut out);
    }
    out.into()
}

/// A picture from its bytes: its frame, its tick and its entities; none when they are not one.
pub fn decode_picture<S: Codec>(mut bytes: &[u8]) -> Option<(u64, u64, Vec<Seen<S>>)> {
    let frame = take_u64(&mut bytes)?;
    let tick = take_u64(&mut bytes)?;
    let count = take_u32(&mut bytes)? as usize;
    let mut picture = Vec::with_capacity(count.min(bytes.len()));
    for _ in 0..count {
        let id = take_u32(&mut bytes)?;
        let (&owned, rest) = bytes.split_first()?;
        bytes = rest;
        let epoch = match owned {
            0 => None,
            1 => Some(take_u32(&mut bytes)?),
            _ => return None,
        };
        let state = S::decode(&mut bytes)?;
        picture.push(Seen { id, epoch, state });
    }
    bytes.is_empty().then_some((frame, tick, picture))
}

/// Writes' bytes (see the module's documentation).
pub fn encode_writes<S: Codec>(frame: u64, tick: u64, writes: &[(EntityId, u32, S)]) -> Bytes {
    let mut out = Vec::with_capacity(20 + writes.len() * 64);
    out.extend_from_slice(&frame.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&(writes.len() as u32).to_le_bytes());
    for (id, epoch, state) in writes {
        out.extend_from_slice(&id.to_le_bytes());
        out.extend_from_slice(&epoch.to_le_bytes());
        state.encode(&mut out);
    }
    out.into()
}

/// Writes from their bytes: the frame they answer, the tick and the writes; none when they are
/// not.
pub fn decode_writes<S: Codec>(mut bytes: &[u8]) -> Option<(u64, u64, Writes<S>)> {
    let frame = take_u64(&mut bytes)?;
    let tick = take_u64(&mut bytes)?;
    let count = take_u32(&mut bytes)? as usize;
    let mut writes = Vec::with_capacity(count.min(bytes.len()));
    for _ in 0..count {
        let id = take_u32(&mut bytes)?;
        let epoch = take_u32(&mut bytes)?;
        writes.push((id, epoch, S::decode(&mut bytes)?));
    }
    bytes.is_empty().then_some((frame, tick, writes))
}

/// What a tick did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickReport {
    /// The entities handed over at its end.
    pub handoffs: Vec<Handoff>,
    /// The workers that went away during it, each covered by another.
    pub covered: Vec<(WorkerId, WorkerId)>,
    /// The entities handed over by those covers.
    pub reassigned: usize,
    /// Writes the layer refused (a stale epoch).
    pub refused: u64,
}

/// The layer's side: the layer, its strips and its workers' sessions (see the module's
/// documentation).
pub struct LayerHost<S> {
    layer: Layer<S>,
    strips: Strips,
    workers: BTreeMap<WorkerId, Session>,
    tick: u64,
    frame: u64,
}

impl<S: Placed + Codec> LayerHost<S> {
    /// A host for `layer` over `strips`, with no worker yet.
    pub fn new(layer: Layer<S>, strips: Strips) -> Self {
        Self {
            layer,
            strips,
            workers: BTreeMap::new(),
            tick: 0,
            frame: 0,
        }
    }

    /// The layer.
    pub fn layer(&self) -> &Layer<S> {
        &self.layer
    }

    /// The tick the next [`LayerHost::tick`] steps.
    pub fn now(&self) -> u64 {
        self.tick
    }

    /// The workers attached.
    pub fn workers(&self) -> impl Iterator<Item = WorkerId> + '_ {
        self.workers.keys().copied()
    }

    /// Worker `worker` joined, or came back, on `session`: its region is given back to it. The
    /// handoffs that made.
    pub fn attach(&mut self, worker: WorkerId, session: Session) -> Vec<Handoff> {
        self.workers.insert(worker, session);
        self.layer.reassign(&self.strips, worker, worker)
    }

    /// Where each worker holds.
    pub fn strips(&self) -> &Strips {
        &self.strips
    }

    /// The workers are now `members`, west to east: one left for good, or one joined (attached
    /// first). The strips are cut again over where the entities stand ([`Strips::balanced`]), a
    /// worker no longer among them is let go, and the handoffs are made at once, between ticks.
    /// For a change of workers only, never for how loaded they are (`docs/research/netcode.md`
    /// §9).
    pub fn recut(&mut self, members: Vec<WorkerId>) -> Vec<Handoff> {
        let xs = self.layer.records().values().map(|r| r.state.position().x);
        self.strips = Strips::balanced(members, xs, self.strips.margin, self.strips.band);
        let members = &self.strips.members;
        self.workers.retain(|w, session| {
            let kept = members.contains(w);
            if !kept {
                session.close();
            }
            kept
        });
        self.layer.rebalance(&self.strips)
    }

    /// One tick (see the module's documentation): the pictures, the writes, the handoffs. A worker
    /// that closes, or stays silent past `patience`, is covered and the tick stepped again.
    /// Panics when no worker is left.
    pub async fn tick(&mut self, patience: Duration) -> TickReport {
        let mut report = TickReport::default();
        let refused_before = self.layer.refused();
        let writes = loop {
            assert!(!self.workers.is_empty(), "no worker left to step the world");
            self.frame += 1;
            let frame = self.frame;
            let mut gone = Vec::new();
            for (&w, session) in &self.workers {
                let picture = self.layer.picture(w, &self.strips);
                if session
                    .send(CHANNEL, encode_picture(frame, self.tick, &picture))
                    .is_err()
                {
                    gone.push(w);
                }
            }
            let mut writes: Vec<(WorkerId, Writes<S>)> = Vec::new();
            for (&w, session) in &mut self.workers {
                if gone.contains(&w) {
                    continue;
                }
                match answer::<S>(session, frame, patience).await {
                    Some(list) => writes.push((w, list)),
                    None => gone.push(w),
                }
            }
            if gone.is_empty() {
                break writes;
            }
            // Covered by the lowest worker left; the tick stepped again from the same pictures.
            for w in gone {
                if let Some(session) = self.workers.remove(&w) {
                    session.close();
                }
                let Some(&to) = self.workers.keys().next() else {
                    panic!("no worker left to step the world");
                };
                report.reassigned += self.layer.reassign(&self.strips, w, to).len();
                report.covered.push((w, to));
            }
        };
        for (w, list) in writes {
            for (id, epoch, state) in list {
                let _ = self.layer.write(w, id, epoch, state);
            }
        }
        report.handoffs = self.layer.rebalance(&self.strips);
        report.refused = self.layer.refused() - refused_before;
        self.tick += 1;
        report
    }
}

/// `session`'s writes answering frame `frame`, earlier answers dropped; none when it closes, sends
/// what is not writes, or stays silent past `patience`.
async fn answer<S: Codec>(
    session: &mut Session,
    frame: u64,
    patience: Duration,
) -> Option<Writes<S>> {
    loop {
        let message = tokio::time::timeout(patience, session.recv())
            .await
            .ok()
            .flatten()?;
        let (answers, _, writes) = decode_writes::<S>(&message.payload)?;
        if answers == frame {
            return Some(writes);
        }
    }
}

/// A worker's side, worker `id` on `session`: takes each picture, steps its own entities with
/// `step` and sends the writes, until the session closes. The ticks it stepped.
pub async fn serve<S: Placed + Codec>(
    id: WorkerId,
    session: &mut Session,
    mut step: impl FnMut(EntityId, &S, &BTreeMap<EntityId, &S>) -> S,
) -> u64 {
    let mut worker = forge_sim::authority::Worker::new(id);
    let mut stepped = 0;
    while let Some(message) = session.recv().await {
        let Some((frame, tick, picture)) = decode_picture::<S>(&message.payload) else {
            continue;
        };
        worker.take(picture);
        let writes = worker.step(&mut step);
        if session
            .send(CHANNEL, encode_writes(frame, tick, &writes))
            .is_err()
        {
            break;
        }
        stepped += 1;
    }
    stepped
}
