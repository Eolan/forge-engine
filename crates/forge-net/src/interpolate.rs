//! The client's picture of the others (#268, step 3): each entity drawn a fixed delay behind the
//! server's clock, between the two states it was sent at about that moment (Bernier 2001's entity
//! interpolation; Fiedler, "Snapshot Interpolation").
//!
//! - [`Interpolator::take`] records what a [`SnapshotReceiver`] just took: the states its newest
//!   packet carried, at the packet's server tick. An entity the receiver no longer holds is
//!   forgotten.
//! - [`Interpolator::at`] places every entity at a server tick with a fraction: between its two
//!   states about it, or at its first or last when the moment falls outside them (it is never
//!   carried past what was sent).
//! - [`PlayoutClock`] says which moment to draw: the server's tick as the freshest snapshots tell
//!   it, minus a fixed delay that covers the gaps between an entity's states.
//!
//! The moment drawn is what a shot carries ([`Moment`]): the authority tests the shot against its
//! hit shapes at that moment (`forge_sim::rewind`). The delay is the client's own and fixed. It is
//! never set from how the client's packets arrive: sizing hit volumes or placing what is shown
//! from the measured variance of a client's timing is what Amazon's US 10,940,388 B1 and US
//! 11,771,984 B2 claim (`docs/research/netcode.md` §9). The clock's offset follows the freshest
//! arrival (a lower bound on the link's delay), which is clock synchronisation, not a measure of
//! its variance. Everything is plain `f64` arithmetic in a fixed order (D-016).

use std::collections::{BTreeMap, VecDeque};

use glam::DVec3;

use crate::quantize::{QState, Quantization};
use crate::snapshot::{EntityId, SnapshotReceiver};

/// A moment of the server's clock: a tick and the share of the way to the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Moment {
    /// The tick.
    pub tick: u64,
    /// The share of the way to the next, 0 to 1.
    pub fraction: f64,
}

impl Moment {
    /// The moment at `ticks` (whole and a fraction; at least 0).
    pub fn at(ticks: f64) -> Self {
        let ticks = ticks.max(0.0);
        let tick = ticks.floor();
        Self {
            tick: tick as u64,
            fraction: ticks - tick,
        }
    }

    /// The moment as ticks, whole and a fraction.
    pub fn ticks(&self) -> f64 {
        self.tick as f64 + self.fraction
    }
}

/// Each entity's recent states by server tick (see the module's documentation).
#[derive(Clone, Debug)]
pub struct Interpolator {
    quantization: Quantization,
    tracks: BTreeMap<EntityId, VecDeque<(u32, QState)>>,
    /// Ticks a state is kept behind the newest snapshot's.
    keep: u32,
    newest: u32,
}

impl Interpolator {
    /// An interpolator over states in `quantization`'s integers, each kept `keep` ticks behind
    /// the newest snapshot (more than the playout delay and the gaps between an entity's states).
    pub fn new(quantization: Quantization, keep: u32) -> Self {
        Self {
            quantization,
            tracks: BTreeMap::new(),
            keep: keep.max(1),
            newest: 0,
        }
    }

    /// Records the newest packet `receiver` took: the states it carried, at its server tick.
    /// Call it after each packet the receiver applied; an entity it no longer holds is forgotten.
    pub fn take(&mut self, receiver: &SnapshotReceiver) {
        let tick = receiver.tick();
        let view = receiver.view();
        self.newest = self.newest.max(tick);
        for id in receiver.latest_entities() {
            let Some(state) = view.get(&id) else {
                continue;
            };
            let track = self.tracks.entry(id).or_default();
            // Ticks come in order (the receiver drops older packets); a state at the same tick
            // replaces the one before.
            match track.back() {
                Some(&(t, _)) if t == tick => {
                    track.pop_back();
                }
                Some(&(t, _)) if t > tick => continue,
                _ => {}
            }
            track.push_back((tick, *state));
        }
        self.tracks.retain(|id, _| view.contains_key(id));
        let oldest = self.newest.saturating_sub(self.keep);
        for track in self.tracks.values_mut() {
            // Keep one state at or before the oldest moment that may be drawn.
            while track.len() > 1 && track[1].0 <= oldest {
                track.pop_front();
            }
        }
    }

    /// The server tick of the newest snapshot taken.
    pub fn newest(&self) -> u32 {
        self.newest
    }

    /// How many entities it holds.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Entity `id`'s position at `moment`, metres: between its two states about it, linearly;
    /// at its first or last outside them. None for an entity it does not hold.
    pub fn position(&self, id: EntityId, moment: Moment) -> Option<DVec3> {
        let track = self.tracks.get(&id)?;
        let q = &self.quantization;
        let place = |s: &QState| q.dequantize(s).position;
        let at = moment.ticks();
        let after = track.iter().position(|&(t, _)| f64::from(t) > at);
        Some(match after {
            Some(0) => place(&track[0].1),
            None => place(&track.back()?.1),
            Some(k) => {
                let (t0, a) = &track[k - 1];
                let (t1, b) = &track[k];
                let share = (at - f64::from(*t0)) / f64::from(t1 - t0);
                place(a).lerp(place(b), share)
            }
        })
    }

    /// Every entity it holds at `moment`, by id.
    pub fn at(&self, moment: Moment) -> Vec<(EntityId, DVec3)> {
        self.tracks
            .keys()
            .filter_map(|&id| Some((id, self.position(id, moment)?)))
            .collect()
    }
}

/// Which moment of the server's clock the client draws (see the module's documentation).
#[derive(Clone, Debug)]
pub struct PlayoutClock {
    /// Ticks drawn behind the server's clock.
    delay: f64,
    /// Recent arrivals: the local time (ticks) each snapshot came at, and its server tick.
    arrivals: VecDeque<(f64, u32)>,
    /// Arrivals the offset is taken over.
    window: usize,
    /// The last moment drawn, in ticks: the clock never goes back before it.
    drawn: Option<f64>,
}

impl PlayoutClock {
    /// A clock drawing `delay` ticks behind the server's (at least two snapshot intervals and
    /// the link's jitter), its offset taken over the last `window` snapshots.
    pub fn new(delay: f64, window: usize) -> Self {
        Self {
            delay,
            arrivals: VecDeque::new(),
            window: window.max(1),
            drawn: None,
        }
    }

    /// A snapshot of server tick `tick` arrived at local time `now` (ticks of the client's own
    /// clock).
    pub fn arrived(&mut self, now: f64, tick: u32) {
        if self.arrivals.len() == self.window {
            self.arrivals.pop_front();
        }
        self.arrivals.push_back((now, tick));
    }

    /// The moment to draw at local time `now`: the server's tick the freshest recent arrival
    /// implies, minus the delay, never before the last one drawn. When the freshest arrival leaves
    /// the window the offset can drop by the link's jitter: the clock holds until time catches up,
    /// rather than drawing (and shooting at) a moment it already passed. None before the first
    /// snapshot.
    pub fn moment(&mut self, now: f64) -> Option<Moment> {
        let offset = self
            .arrivals
            .iter()
            .map(|&(at, tick)| f64::from(tick) - at)
            .reduce(f64::max)?;
        let ticks = self.drawn.map_or(now + offset - self.delay, |last| {
            (now + offset - self.delay).max(last)
        });
        self.drawn = Some(ticks);
        Some(Moment::at(ticks))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quantize::EntityState;
    use crate::replicate::{ReplicationConfig, Replicator};

    /// Two snapshots four ticks apart: an entity is drawn between its two states, and at its
    /// last past it; one the second snapshot removed is forgotten.
    #[test]
    fn an_entity_is_drawn_between_the_two_states_about_the_moment() {
        let config = ReplicationConfig::default();
        let q = config.quantization;
        let mut replicator = Replicator::new(config);
        replicator.add_client(1, DVec3::ZERO);
        let mut receiver = SnapshotReceiver::new(q);
        let mut interpolator = Interpolator::new(q, 60);
        let walker = |x: f64| EntityState {
            position: DVec3::new(x, 0.0, 10.0),
            ..EntityState::default()
        };
        for (tick, x) in [(10_u32, 0.0), (14, 2.0)] {
            let (w, other) = (walker(x), walker(-5.0));
            let mut world = vec![(7, &w, 1.0)];
            if tick == 10 {
                world.push((9, &other, 1.0));
            }
            replicator.set_world(world.into_iter());
            let (bytes, _) = replicator.snapshot(1, tick).expect("a client");
            receiver.receive(&bytes).expect("a snapshot");
            interpolator.take(&receiver);
            let ack = receiver.ack().expect("an ack");
            replicator.acknowledge(1, ack);
        }
        let middle = interpolator
            .position(7, Moment::at(12.5))
            .expect("the walker");
        assert!((middle.x - 1.25).abs() < 0.01, "{middle}");
        let past = interpolator.position(7, Moment::at(20.0)).expect("held");
        assert!((past.x - 2.0).abs() < 0.01, "{past}");
        let before = interpolator.position(7, Moment::at(3.0)).expect("held");
        assert!(before.x.abs() < 0.01, "{before}");
        assert_eq!(interpolator.position(9, Moment::at(12.0)), None);
        assert_eq!(interpolator.len(), 1);
    }

    /// The clock follows the freshest arrival: snapshots 2 ticks apart arriving 3 ticks late
    /// give the server's tick 3 behind, and the moment drawn the delay behind that.
    #[test]
    fn the_playout_clock_draws_the_delay_behind_the_freshest_arrival() {
        let mut clock = PlayoutClock::new(6.0, 30);
        assert_eq!(clock.moment(0.0), None);
        for k in 0..10_u32 {
            // One arrival in three comes a tick later still.
            let late = if k % 3 == 0 { 1.0 } else { 0.0 };
            clock.arrived(f64::from(2 * k) + 3.0 + late, 2 * k);
        }
        let m = clock.moment(30.0).expect("a moment");
        assert!((m.ticks() - 21.0).abs() < 1e-9, "{m:?}");
        // A window of two: the freshest arrival leaves it and the offset drops by a tick; the
        // moment holds there rather than going back, then goes on as time passes.
        let mut short = PlayoutClock::new(6.0, 2);
        short.arrived(10.0, 10);
        short.arrived(13.0, 12);
        assert!((short.moment(13.0).unwrap().ticks() - 7.0).abs() < 1e-9);
        short.arrived(16.0, 14);
        assert!(
            (short.moment(13.5).unwrap().ticks() - 7.0).abs() < 1e-9,
            "held"
        );
        assert!((short.moment(16.0).unwrap().ticks() - 9.0).abs() < 1e-9);
        assert_eq!(
            Moment::at(21.25),
            Moment {
                tick: 21,
                fraction: 0.25
            }
        );
    }
}
