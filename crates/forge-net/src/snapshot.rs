//! Snapshot packets: entity states packed to the bit, as deltas against what the client
//! acknowledged (D-010; Fiedler, "Snapshot Compression"; the Quake III model).
//!
//! A snapshot is one unreliable datagram. Its header names the packet's sequence number, the
//! server's tick and an origin cell (the client's), then come entity blocks sorted by slot and
//! the ids the client must forget:
//!
//! | field | bits |
//! |---|---|
//! | sequence | 16 |
//! | tick | 32 |
//! | origin cell, x y z | zigzag, groups of 6 bits |
//! | blocks | a count, then each block |
//! | removals | a count, then each entity id as a gap from the last |
//!
//! An entity is named in a block by its **slot**: a small number the server gives it for this
//! client while the client hears of it (the lowest free one), so that blocks sorted by slot
//! cost a few bits each for their gap instead of a world id's twenty or more. A block is the
//! slot's gap from the previous block's, a 5-bit baseline (0: a full state; `k`: a delta
//! against the state this client acknowledged in packet `sequence − k`), and the state. A full
//! state starts with the entity's id, then a position as a cell (from the origin's) and an
//! offset in it, the 29-bit orientation, the velocity and two 16-bit words; it also gives the
//! slot to that entity, and the client forgets whatever held it. A delta starts with five bits
//! saying which fields changed; a changed position is three differences in steps, each in a
//! 2-bit size class (0, 6, 12 or 24 bits, zigzag); a changed velocity likewise (0, 5, 9 or 17).
//! A walker moving in a straight line at 1.4 m/s costs about 4 bytes a delta with its slot and
//! baseline. Removals name entity ids, so that one sent again after its slot was given to
//! another entity removes the right one.
//!
//! [`SnapshotReceiver`] is the client's side: it decodes, keeps the packets of the last
//! [`HISTORY`] it took (a delta's baseline is one of them), drops a packet older than the
//! newest it took, and acknowledges what it took ([`Ack`]).

use std::collections::BTreeMap;

use crate::bits::{BadPacket, BitReader, BitWriter, unzigzag, zigzag};
use crate::quantize::{QState, Quantization};

/// An entity's id, unique in the server's world.
pub type EntityId = u32;
/// A snapshot's sequence number; it wraps.
pub type Seq = u16;

/// How many packets back a delta's baseline may be: the 5-bit field's largest value.
pub const BASELINE_WINDOW: u16 = 31;
/// Packets the receiver keeps for baselines.
pub const HISTORY: usize = 32;

const POSITION_CLASSES: [u32; 4] = [0, 6, 12, 24];
const VELOCITY_CLASSES: [u32; 4] = [0, 5, 9, 16];
/// Bits of an id gap's groups.
const ID_GROUP: u32 = 5;
/// Bits of a cell's groups.
const CELL_GROUP: u32 = 4;

/// Whether `a` comes after `b`, sequence numbers wrapping (RFC 1982's serial arithmetic).
pub fn newer(a: Seq, b: Seq) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000
}

/// Writes `v` in the smallest class that holds its zigzag; `false` if none does.
fn write_classed(w: &mut BitWriter, v: i64, classes: &[u32; 4]) -> bool {
    let z = zigzag(v);
    for (c, &bits) in classes.iter().enumerate() {
        if bits == 64 || z < 1u64 << bits {
            w.write(c as u64, 2);
            w.write(z, bits);
            return true;
        }
    }
    false
}

fn fits_classed(v: i64, classes: &[u32; 4]) -> bool {
    zigzag(v) < 1u64 << classes[3]
}

fn read_classed(r: &mut BitReader, classes: &[u32; 4]) -> Result<i64, BadPacket> {
    let c = r.read(2)? as usize;
    Ok(unzigzag(r.read(classes[c])?))
}

/// The cell of each axis of a position, in a quantisation's cells.
pub fn cell_of(q: &Quantization, position: &[i64; 3]) -> [i64; 3] {
    position.map(|p| p.div_euclid(q.cell_steps))
}

fn write_position(w: &mut BitWriter, q: &Quantization, p: &[i64; 3], origin: &[i64; 3]) {
    let bits = q.offset_bits();
    for a in 0..3 {
        let cell = p[a].div_euclid(q.cell_steps);
        w.varbits(zigzag(cell - origin[a]), CELL_GROUP);
        w.write(p[a].rem_euclid(q.cell_steps) as u64, bits);
    }
}

fn read_position(
    r: &mut BitReader,
    q: &Quantization,
    origin: &[i64; 3],
) -> Result<[i64; 3], BadPacket> {
    let bits = q.offset_bits();
    let mut p = [0i64; 3];
    for a in 0..3 {
        let cell = unzigzag(r.varbits(CELL_GROUP)?) + origin[a];
        let offset = r.read(bits)? as i64;
        if offset >= q.cell_steps {
            return Err(BadPacket);
        }
        p[a] = cell * q.cell_steps + offset;
    }
    Ok(p)
}

fn write_velocity(w: &mut BitWriter, v: &[i32; 3]) {
    for x in v {
        let fits = write_classed(w, i64::from(*x), &VELOCITY_CLASSES);
        debug_assert!(fits, "velocities are clamped to 16 bits");
    }
}

fn read_velocity(r: &mut BitReader) -> Result<[i32; 3], BadPacket> {
    let mut v = [0i32; 3];
    for x in &mut v {
        *x = i32::try_from(read_classed(r, &VELOCITY_CLASSES)?).map_err(|_| BadPacket)?;
    }
    Ok(v)
}

/// Writes a full state.
pub fn write_full(w: &mut BitWriter, q: &Quantization, s: &QState, origin: &[i64; 3]) {
    write_position(w, q, &s.position, origin);
    w.write(u64::from(s.rotation), q.rotation_total_bits());
    write_velocity(w, &s.velocity);
    w.write(u64::from(s.archetype), 16);
    w.write(u64::from(s.animation), 16);
}

/// Reads a full state.
pub fn read_full(
    r: &mut BitReader,
    q: &Quantization,
    origin: &[i64; 3],
) -> Result<QState, BadPacket> {
    Ok(QState {
        position: read_position(r, q, origin)?,
        rotation: r.read(q.rotation_total_bits())? as u32,
        velocity: read_velocity(r)?,
        archetype: r.read(16)? as u16,
        animation: r.read(16)? as u16,
    })
}

/// Writes `s` as a delta against `base`.
pub fn write_delta(
    w: &mut BitWriter,
    q: &Quantization,
    s: &QState,
    base: &QState,
    origin: &[i64; 3],
) {
    let moved = s.position != base.position;
    let turned = s.rotation != base.rotation;
    let sped = s.velocity != base.velocity;
    let kind = s.archetype != base.archetype;
    let anim = s.animation != base.animation;
    for flag in [moved, turned, sped, kind, anim] {
        w.bool(flag);
    }
    if moved {
        let d = [0, 1, 2].map(|a| s.position[a].wrapping_sub(base.position[a]));
        let small = d.iter().all(|&x| fits_classed(x, &POSITION_CLASSES));
        w.bool(!small);
        if small {
            for x in d {
                write_classed(w, x, &POSITION_CLASSES);
            }
        } else {
            write_position(w, q, &s.position, origin);
        }
    }
    if turned {
        w.write(u64::from(s.rotation), q.rotation_total_bits());
    }
    if sped {
        for a in 0..3 {
            let fits = write_classed(
                w,
                i64::from(s.velocity[a]) - i64::from(base.velocity[a]),
                &VELOCITY_CLASSES_DELTA,
            );
            debug_assert!(fits, "a velocity's change fits 17 bits");
        }
    }
    if kind {
        w.write(u64::from(s.archetype), 16);
    }
    if anim {
        w.write(u64::from(s.animation), 16);
    }
}

/// A velocity's change spans twice a velocity's range: one more bit at the top.
const VELOCITY_CLASSES_DELTA: [u32; 4] = [0, 5, 9, 17];

/// Reads a delta against `base`.
pub fn read_delta(
    r: &mut BitReader,
    q: &Quantization,
    base: &QState,
    origin: &[i64; 3],
) -> Result<QState, BadPacket> {
    let mut s = *base;
    let (moved, turned, sped, kind, anim) = (r.bool()?, r.bool()?, r.bool()?, r.bool()?, r.bool()?);
    if moved {
        if r.bool()? {
            s.position = read_position(r, q, origin)?;
        } else {
            for a in 0..3 {
                s.position[a] = base.position[a].wrapping_add(read_classed(r, &POSITION_CLASSES)?);
            }
        }
    }
    if turned {
        s.rotation = r.read(q.rotation_total_bits())? as u32;
    }
    if sped {
        for a in 0..3 {
            let v = i64::from(base.velocity[a]) + read_classed(r, &VELOCITY_CLASSES_DELTA)?;
            s.velocity[a] = i32::try_from(v).map_err(|_| BadPacket)?;
        }
    }
    if kind {
        s.archetype = r.read(16)? as u16;
    }
    if anim {
        s.animation = r.read(16)? as u16;
    }
    Ok(s)
}

/// What the client acknowledges: the newest snapshot it took and, bit `k`, whether it took
/// the one `k + 1` before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ack {
    /// The newest snapshot taken.
    pub latest: Seq,
    /// The 32 before it.
    pub bits: u32,
}

impl Ack {
    /// Its 6 bytes.
    pub fn encode(&self) -> [u8; 6] {
        let mut b = [0u8; 6];
        b[..2].copy_from_slice(&self.latest.to_le_bytes());
        b[2..].copy_from_slice(&self.bits.to_le_bytes());
        b
    }

    /// Reads one.
    pub fn decode(b: &[u8]) -> Option<Self> {
        Some(Self {
            latest: u16::from_le_bytes(b.get(..2)?.try_into().ok()?),
            bits: u32::from_le_bytes(b.get(2..6)?.try_into().ok()?),
        })
    }

    /// The sequence numbers it acknowledges, newest first.
    pub fn sequences(&self) -> impl Iterator<Item = Seq> + '_ {
        std::iter::once(self.latest).chain(
            (0..32)
                .filter(|k| self.bits >> k & 1 == 1)
                .map(|k| self.latest.wrapping_sub(k + 1)),
        )
    }
}

/// A snapshot's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The packet's sequence number.
    pub seq: Seq,
    /// The server's tick.
    pub tick: u32,
    /// The cell full positions are written from.
    pub origin: [i64; 3],
}

/// Writes a header.
pub fn write_header(w: &mut BitWriter, h: &Header) {
    w.write(u64::from(h.seq), 16);
    w.write(u64::from(h.tick), 32);
    for c in h.origin {
        w.varbits(zigzag(c), 6);
    }
}

fn read_header(r: &mut BitReader) -> Result<Header, BadPacket> {
    let seq = r.read(16)? as u16;
    let tick = r.read(32)? as u32;
    let mut origin = [0i64; 3];
    for c in &mut origin {
        *c = unzigzag(r.varbits(6)?);
    }
    Ok(Header { seq, tick, origin })
}

/// Bits a gap costs in groups of [`ID_GROUP`].
pub fn gap_bits(gap: u32) -> usize {
    let mut groups = 1;
    let mut v = gap >> ID_GROUP;
    while v != 0 {
        groups += 1;
        v >>= ID_GROUP;
    }
    groups * (ID_GROUP as usize + 1)
}

/// Bits an entity id costs at the head of a full state.
pub fn entity_id_bits(id: EntityId) -> usize {
    let mut groups = 1;
    let mut v = id >> ENTITY_GROUP;
    while v != 0 {
        groups += 1;
        v >>= ENTITY_GROUP;
    }
    groups * (ENTITY_GROUP as usize + 1)
}

/// Bits of an entity id's groups.
const ENTITY_GROUP: u32 = 8;

/// Writes the entity id that heads a full state's body.
pub fn write_entity_id(w: &mut BitWriter, id: EntityId) {
    w.varbits(u64::from(id), ENTITY_GROUP);
}

/// A block ready to be written: the entity's slot, its baseline (0 for a full state) and where
/// its body lies among the bodies written (a full state's starting with [`write_entity_id`]).
#[derive(Clone, Copy, Debug)]
pub struct Block {
    /// The entity's slot for this client.
    pub slot: u32,
    /// Packets back to the baseline; 0 for a full state.
    pub baseline: u16,
    /// The body's first bit in the bodies.
    pub start: usize,
    /// The body's bits.
    pub len: usize,
}

/// Writes the blocks (sorted by slot), their bodies taken from `bodies`, and the removals
/// (sorted ids) after a header.
pub fn write_body(
    w: &mut BitWriter,
    bodies: &BitWriter,
    blocks: impl ExactSizeIterator<Item = Block>,
    removals: &[EntityId],
) {
    w.varbits(blocks.len() as u64, 6);
    let mut last: Option<u32> = None;
    for b in blocks {
        let gap = last.map_or(b.slot, |l| b.slot - l - 1);
        w.varbits(u64::from(gap), ID_GROUP);
        w.write(u64::from(b.baseline), 5);
        w.append_range(bodies, b.start, b.len);
        last = Some(b.slot);
    }
    w.varbits(removals.len() as u64, 4);
    let mut last: Option<EntityId> = None;
    for id in removals {
        let gap = last.map_or(*id, |l| id - l - 1);
        w.varbits(u64::from(gap), ENTITY_GROUP);
        last = Some(*id);
    }
}

/// Reads `n` gaps into increasing values.
fn read_gaps(r: &mut BitReader, n: usize, group: u32) -> Result<Vec<u32>, BadPacket> {
    let mut out = Vec::with_capacity(n);
    let mut last: Option<u32> = None;
    for _ in 0..n {
        let gap = u32::try_from(r.varbits(group)?).map_err(|_| BadPacket)?;
        let v = match last {
            None => gap,
            Some(l) => l
                .checked_add(gap)
                .and_then(|x| x.checked_add(1))
                .ok_or(BadPacket)?,
        };
        out.push(v);
        last = Some(v);
    }
    Ok(out)
}

/// What a received snapshot did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    /// It was applied.
    Applied {
        /// Its sequence number.
        seq: Seq,
        /// Entities it carried.
        blocks: usize,
        /// Of which full states.
        full: usize,
        /// Entities it removed.
        removed: usize,
    },
    /// It was older than the newest taken, and dropped.
    Stale,
}

/// A packet taken: its blocks as `(slot, entity, state)`, sorted by slot.
type HistoryEntry = (Seq, Vec<(u32, EntityId, QState)>);

/// The client's side of replication (see the module's documentation).
#[derive(Debug)]
pub struct SnapshotReceiver {
    quantization: Quantization,
    history: Vec<Option<HistoryEntry>>,
    /// Which entity each slot holds.
    slots: BTreeMap<u32, EntityId>,
    /// Which slot each entity is in.
    ids: BTreeMap<EntityId, u32>,
    view: BTreeMap<EntityId, QState>,
    latest: Option<Seq>,
    bits: u32,
    tick: u32,
}

impl SnapshotReceiver {
    /// A receiver that has taken nothing.
    pub fn new(quantization: Quantization) -> Self {
        Self {
            quantization,
            history: vec![None; HISTORY],
            slots: BTreeMap::new(),
            ids: BTreeMap::new(),
            view: BTreeMap::new(),
            latest: None,
            bits: 0,
            tick: 0,
        }
    }

    /// The entities as the newest snapshot left them, in integers.
    pub fn view(&self) -> &BTreeMap<EntityId, QState> {
        &self.view
    }

    /// The entities the newest snapshot taken carried.
    pub fn latest_entities(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.latest
            .and_then(|l| self.history[l as usize % HISTORY].as_ref())
            .into_iter()
            .flat_map(|(_, blocks)| blocks.iter().map(|b| b.1))
    }

    /// The server's tick of the newest snapshot taken.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// What to acknowledge; `None` before the first snapshot.
    pub fn ack(&self) -> Option<Ack> {
        self.latest.map(|latest| Ack {
            latest,
            bits: self.bits,
        })
    }

    fn baseline(&self, seq: Seq, slot: u32) -> Option<(EntityId, &QState)> {
        let (s, blocks) = self.history[seq as usize % HISTORY].as_ref()?;
        if *s != seq {
            return None;
        }
        let k = blocks.binary_search_by_key(&slot, |b| b.0).ok()?;
        Some((blocks[k].1, &blocks[k].2))
    }

    /// Takes a snapshot: applies it, or drops it if it is older than the newest taken. A
    /// packet that does not parse, or names a baseline the receiver does not have, changes
    /// nothing and is not acknowledged.
    pub fn receive(&mut self, packet: &[u8]) -> Result<Taken, BadPacket> {
        let q = self.quantization;
        let mut r = BitReader::new(packet);
        let h = read_header(&mut r)?;
        if self.latest.is_some_and(|l| !newer(h.seq, l)) {
            return Ok(Taken::Stale);
        }
        let n = r.varbits(6)? as usize;
        if n > packet.len() * 8 {
            return Err(BadPacket);
        }
        let mut blocks = Vec::with_capacity(n);
        let mut last: Option<u32> = None;
        let mut full = 0;
        for _ in 0..n {
            let slot = read_gaps(&mut r, 1, ID_GROUP)?[0];
            let slot = match last {
                None => slot,
                Some(l) => l
                    .checked_add(slot)
                    .and_then(|x| x.checked_add(1))
                    .ok_or(BadPacket)?,
            };
            let offset = r.read(5)? as u16;
            let (id, state) = if offset == 0 {
                full += 1;
                let id = u32::try_from(r.varbits(ENTITY_GROUP)?).map_err(|_| BadPacket)?;
                (id, read_full(&mut r, &q, &h.origin)?)
            } else {
                let (id, base) = self
                    .baseline(h.seq.wrapping_sub(offset), slot)
                    .ok_or(BadPacket)?;
                (id, read_delta(&mut r, &q, base, &h.origin)?)
            };
            blocks.push((slot, id, state));
            last = Some(slot);
        }
        let removals = r.varbits(4)? as usize;
        if removals > packet.len() * 8 {
            return Err(BadPacket);
        }
        let removed = read_gaps(&mut r, removals, ENTITY_GROUP)?;
        // Everything parsed: apply.
        for (slot, id, s) in &blocks {
            if let Some(old) = self.slots.insert(*slot, *id)
                && old != *id
                && self.ids.get(&old) == Some(slot)
            {
                // The slot went to another entity: the old one left the interest (its removal
                // may have been lost). Unless it came back under another slot meanwhile.
                self.ids.remove(&old);
                self.view.remove(&old);
            }
            if let Some(before) = self.ids.insert(*id, *slot)
                && before != *slot
                && self.slots.get(&before) == Some(id)
            {
                self.slots.remove(&before);
            }
            self.view.insert(*id, *s);
        }
        for id in &removed {
            if let Some(slot) = self.ids.remove(id)
                && self.slots.get(&slot) == Some(id)
            {
                self.slots.remove(&slot);
            }
            self.view.remove(id);
        }
        match self.latest {
            Some(l) => {
                let gap = u32::from(h.seq.wrapping_sub(l));
                self.bits = if gap >= 32 {
                    0
                } else {
                    (self.bits << gap) | (1 << (gap - 1))
                };
            }
            None => self.bits = 0,
        }
        self.latest = Some(h.seq);
        self.tick = h.tick;
        let count = blocks.len();
        self.history[h.seq as usize % HISTORY] = Some((h.seq, blocks));
        Ok(Taken::Applied {
            seq: h.seq,
            blocks: count,
            full,
            removed: removed.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::SplitMix64;

    fn random_state(r: &mut SplitMix64) -> QState {
        QState {
            position: [
                r.below(2_000_000) as i64 - 1_000_000,
                r.below(20_000) as i64,
                r.below(2_000_000) as i64 - 1_000_000,
            ],
            rotation: r.next_u32() & ((1 << 29) - 1),
            velocity: [0, 1, 2].map(|_| r.below(65_535) as i32 - 32_767),
            archetype: r.next_u32() as u16,
            animation: r.next_u32() as u16,
        }
    }

    fn nudge(r: &mut SplitMix64, s: &QState) -> QState {
        let mut t = *s;
        let scale = [0, 30, 3000, 2_000_000][r.below(4) as usize];
        for a in 0..3 {
            if scale > 0 && r.next_bool() {
                t.position[a] += r.below(2 * scale) as i64 - scale as i64;
            }
        }
        if r.below(4) == 0 {
            t.rotation = r.next_u32() & ((1 << 29) - 1);
        }
        if r.below(3) == 0 {
            t.velocity[r.below(3) as usize] = r.below(65_535) as i32 - 32_767;
        }
        if r.below(8) == 0 {
            t.animation = r.next_u32() as u16;
        }
        t
    }

    #[test]
    fn full_states_and_deltas_decode_to_the_same_integers() {
        let q = Quantization::default();
        let mut r = SplitMix64::new(6);
        for _ in 0..5000 {
            let origin = [r.below(100) as i64 - 50, 0, r.below(100) as i64 - 50];
            let base = random_state(&mut r);
            let s = nudge(&mut r, &base);
            let mut w = BitWriter::new();
            write_full(&mut w, &q, &s, &origin);
            write_delta(&mut w, &q, &s, &base, &origin);
            let bytes = w.finish();
            let mut rd = BitReader::new(&bytes);
            assert_eq!(read_full(&mut rd, &q, &origin), Ok(s));
            assert_eq!(read_delta(&mut rd, &q, &base, &origin), Ok(s));
        }
    }

    #[test]
    fn a_walker_costs_a_few_bytes_a_delta() {
        let q = Quantization::default();
        let base = QState {
            position: [123_456, 50, -98_765],
            velocity: [45, 0, 0],
            ..QState::default()
        };
        let mut s = base;
        s.position[0] += 14; // 1.4 m/s over 100 ms
        let mut w = BitWriter::new();
        write_delta(&mut w, &q, &s, &base, &[0, 0, 0]);
        assert_eq!(w.bits(), 5 + 1 + (2 + 6) + 2 + 2);
        let mut w = BitWriter::new();
        write_full(&mut w, &q, &s, &cell_of(&q, &s.position));
        assert_eq!(w.bits(), 3 * (5 + 13) + 29 + (2 + 9) + 2 + 2 + 32);
    }

    #[test]
    fn acks_name_the_packets_taken() {
        let ack = Ack {
            latest: 3,
            bits: 0b101,
        };
        assert_eq!(ack.sequences().collect::<Vec<_>>(), vec![3, 2, 0]);
        assert_eq!(Ack::decode(&ack.encode()), Some(ack));
        assert!(newer(1, 0xFFFF) && !newer(0xFFFF, 1) && !newer(5, 5));
    }
}
