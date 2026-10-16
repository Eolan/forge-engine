//! Reverb zones: the room the listener is in sets the reverb.
//!
//! A zone is a box or a sphere in the world with the reverb of what it encloses (a hall, a
//! cave, a room) and a fade: the distance outside its shape over which its weight falls from
//! 1 to 0. Where the listener stands, the zone of the highest weight wins (a zone added later
//! wins a tie, so a room inside a hall is added after the hall), and its parameters are blended
//! with the outdoor ones by its weight ([`ReverbParams::lerp`]: decay times in the log domain).
//! [`ReverbZones::follow`] sends the result to a reverb bus when it has changed, from the game
//! thread; the mixer only sees [`crate::Control::Reverb`].
//!
//! This is the listener-driven environment of Wwise's and FMOD's reverb zones and of D-011's
//! "one FDN per acoustic cell"; sources in another room than the listener's (heard through a
//! door) wait for the rooms and portals of the world model.

use glam::DVec3;

use crate::dsp::ReverbParams;
use crate::{AudioEngine, BusId, Control};

/// A zone's extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneShape {
    /// An axis-aligned box.
    Box {
        /// The lowest corner.
        min: DVec3,
        /// The highest corner.
        max: DVec3,
    },
    /// A sphere.
    Sphere {
        /// Its centre.
        centre: DVec3,
        /// Metres.
        radius: f64,
    },
}

impl ZoneShape {
    /// Metres from `point` to the shape (0 inside).
    pub fn distance(&self, point: DVec3) -> f64 {
        match *self {
            ZoneShape::Box { min, max } => {
                let outside = (min - point).max(point - max).max(DVec3::ZERO);
                outside.length()
            }
            ZoneShape::Sphere { centre, radius } => (point.distance(centre) - radius).max(0.0),
        }
    }
}

/// A space with its own reverb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbZone {
    /// Where.
    pub shape: ZoneShape,
    /// Metres outside the shape over which the zone fades out.
    pub fade: f64,
    /// Its reverb.
    pub params: ReverbParams,
}

impl ReverbZone {
    /// The zone's weight at `point`: 1 inside, falling linearly to 0 at `fade` metres out.
    pub fn weight(&self, point: DVec3) -> f64 {
        let d = self.shape.distance(point);
        if d <= 0.0 {
            1.0
        } else if self.fade <= 0.0 {
            0.0
        } else {
            (1.0 - d / self.fade).max(0.0)
        }
    }
}

/// The zones of a place and the reverb outside them.
#[derive(Clone, Debug, PartialEq)]
pub struct ReverbZones {
    /// The reverb where no zone reaches (the open: [`ReverbParams::OPEN`]).
    pub outside: ReverbParams,
    /// The zones; a later one wins a tie.
    pub zones: Vec<ReverbZone>,
    last: Option<ReverbParams>,
}

impl ReverbZones {
    /// No zones yet, `outside` everywhere.
    pub fn new(outside: ReverbParams) -> Self {
        Self {
            outside,
            zones: Vec::new(),
            last: None,
        }
    }

    /// Adds a zone; returns its index.
    pub fn add(&mut self, zone: ReverbZone) -> usize {
        self.zones.push(zone);
        self.zones.len() - 1
    }

    /// The zone that wins at `point` and its weight, if any reaches it.
    pub fn winner(&self, point: DVec3) -> Option<(usize, f64)> {
        let mut best: Option<(usize, f64)> = None;
        for (i, zone) in self.zones.iter().enumerate() {
            let w = zone.weight(point);
            if w > 0.0 && best.is_none_or(|(_, b)| w >= b) {
                best = Some((i, w));
            }
        }
        best
    }

    /// The reverb at `point`.
    pub fn params_at(&self, point: DVec3) -> ReverbParams {
        match self.winner(point) {
            Some((i, w)) => self.outside.lerp(&self.zones[i].params, w as f32),
            None => self.outside,
        }
    }

    /// Sends the reverb at `point` to `bus` when it differs from the last sent (by more than
    /// 1 % in any parameter), and returns it.
    pub fn follow(&mut self, engine: &mut AudioEngine, bus: BusId, point: DVec3) -> ReverbParams {
        let params = self.params_at(point);
        let changed = match self.last {
            None => true,
            Some(last) => {
                let near = |a: f32, b: f32| (a - b).abs() <= 0.01 * a.abs().max(b.abs()).max(1e-3);
                !(near(last.rt60, params.rt60)
                    && near(last.hf_ratio, params.hf_ratio)
                    && near(last.size, params.size)
                    && near(last.predelay, params.predelay)
                    && near(last.wet, params.wet))
            }
        };
        if changed {
            engine.control(Control::Reverb { bus, params });
            self.last = Some(params);
        }
        params
    }
}
