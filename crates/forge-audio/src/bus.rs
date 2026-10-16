//! The mix: a tree of buses with gains and sends.
//!
//! Every voice plays into a bus; every bus but the master feeds its parent, and may also send
//! a share of its output to other buses (an aux bus with a reverb, for example). The layout
//! is fixed when the mixer is made; gains and send levels change at run time
//! ([`crate::Control::BusGain`], [`crate::Control::BusSend`]).
//!
//! A bus's parent and its send targets must have been added before it. The mixer then runs
//! the buses from the last added to the first, so each bus has all its inputs before it
//! runs, and the master ([`BusId::MASTER`]) runs last. Sends are post-fader: they carry the
//! bus's output after its gain.

use crate::dsp::{BusFilter, MediumFilter, Reverb, ReverbParams};

/// A bus of the mixer's layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BusId(pub(crate) u16);

impl BusId {
    /// The master bus: the limiter's input.
    pub const MASTER: BusId = BusId(0);

    /// The bus's index in its layout.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One bus as laid out.
#[derive(Clone, Debug, PartialEq)]
pub struct BusSpec {
    /// A name for tools and logs.
    pub name: String,
    /// Where its output goes (`None` for the master).
    pub parent: Option<BusId>,
    /// Its fader, a linear gain.
    pub gain: f32,
    /// Its sends: a target and a linear level each.
    pub sends: Vec<(BusId, f32)>,
    /// A reverb inserted on it (an aux bus), if any.
    pub reverb: Option<ReverbParams>,
    /// A filter after the reverb, before the fader: the medium the bus is heard through
    /// (under water), if any.
    pub filter: Option<BusFilter>,
}

/// The buses of a mix, before the mixer is made.
#[derive(Clone, Debug, PartialEq)]
pub struct BusLayout {
    buses: Vec<BusSpec>,
}

impl Default for BusLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl BusLayout {
    /// A layout with the master bus alone.
    pub fn new() -> Self {
        Self {
            buses: vec![BusSpec {
                name: "master".to_owned(),
                parent: None,
                gain: 1.0,
                sends: Vec::new(),
                reverb: None,
                filter: None,
            }],
        }
    }

    /// Adds a bus feeding `parent`.
    pub fn add(&mut self, name: &str, parent: BusId) -> BusId {
        assert!(parent.index() < self.buses.len(), "unknown parent bus");
        assert!(self.buses.len() < u16::MAX as usize);
        self.buses.push(BusSpec {
            name: name.to_owned(),
            parent: Some(parent),
            gain: 1.0,
            sends: Vec::new(),
            reverb: None,
            filter: None,
        });
        BusId(self.buses.len() as u16 - 1)
    }

    /// Adds an aux bus feeding `parent` with a reverb on it: voices and buses send to it,
    /// and it returns the reverb's output only (100 % wet).
    pub fn add_reverb(&mut self, name: &str, parent: BusId, params: ReverbParams) -> BusId {
        let id = self.add(name, parent);
        self.buses[id.index()].reverb = Some(params);
        id
    }

    /// Sends `level` of `from`'s output to `to`, which must have been added before `from`.
    /// Returns the send's slot on `from` (for [`crate::Control::BusSend`]).
    pub fn send(&mut self, from: BusId, to: BusId, level: f32) -> usize {
        assert!(to < from, "a bus sends only to buses added before it");
        let sends = &mut self.buses[from.index()].sends;
        sends.push((to, level));
        sends.len() - 1
    }

    /// Gives a bus (not the master) a filter, set at run time by
    /// [`crate::Control::BusFilter`].
    pub fn set_filter(&mut self, bus: BusId, filter: BusFilter) {
        assert!(
            bus != BusId::MASTER,
            "the master has the limiter, not a filter"
        );
        self.buses[bus.index()].filter = Some(filter);
    }

    /// Sets a bus's fader.
    pub fn set_gain(&mut self, bus: BusId, gain: f32) {
        self.buses[bus.index()].gain = gain;
    }

    /// The bus named `name`.
    pub fn find(&self, name: &str) -> Option<BusId> {
        self.buses
            .iter()
            .position(|b| b.name == name)
            .map(|i| BusId(i as u16))
    }

    /// The buses in the order they were added (the master first).
    pub fn buses(&self) -> &[BusSpec] {
        &self.buses
    }

    /// How many buses.
    pub fn len(&self) -> usize {
        self.buses.len()
    }

    /// Never: the master is always there.
    pub fn is_empty(&self) -> bool {
        self.buses.is_empty()
    }
}

/// A bus in the running mixer.
pub(crate) struct Bus {
    pub parent: Option<u16>,
    pub gain: f32,
    pub gain_prev: f32,
    /// Target, level, level at the last block.
    pub sends: Vec<(u16, f32, f32)>,
    pub reverb: Option<Box<Reverb>>,
    pub filter: Option<Box<MediumFilter>>,
    pub left: Box<[f32]>,
    pub right: Box<[f32]>,
    /// Whether anything played into it this block (a silent bus is skipped).
    pub active: bool,
    /// Blocks its reverb still rings after its input stopped.
    pub tail: u32,
}

impl Bus {
    pub fn new(spec: &BusSpec, block: usize, rate: u32) -> Self {
        Self {
            parent: spec.parent.map(|p| p.0),
            gain: spec.gain,
            gain_prev: spec.gain,
            sends: spec
                .sends
                .iter()
                .map(|&(to, level)| (to.0, level, level))
                .collect(),
            reverb: spec.reverb.map(|p| Box::new(Reverb::new(p, rate))),
            filter: spec.filter.map(|f| Box::new(MediumFilter::new(f, rate))),
            left: vec![0.0; block].into_boxed_slice(),
            right: vec![0.0; block].into_boxed_slice(),
            active: false,
            tail: 0,
        }
    }

    pub fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.active = false;
    }
}

/// Adds `left`/`right` times a gain moving from `from` to `to` into `out_left`/`out_right`.
pub(crate) fn mix_into(
    left: &[f32],
    right: &[f32],
    from: f32,
    to: f32,
    out_left: &mut [f32],
    out_right: &mut [f32],
) {
    let n = left.len();
    if from == to {
        if to == 0.0 {
            return;
        }
        for i in 0..n {
            out_left[i] += left[i] * to;
            out_right[i] += right[i] * to;
        }
    } else {
        let step = (to - from) / n as f32;
        for i in 0..n {
            let g = from + step * (i + 1) as f32;
            out_left[i] += left[i] * g;
            out_right[i] += right[i] * g;
        }
    }
}

/// Multiplies two channels by a gain moving from `from` to `to`.
pub(crate) fn apply_gain(left: &mut [f32], right: &mut [f32], from: f32, to: f32) {
    let n = left.len();
    if from == to {
        if to != 1.0 {
            for i in 0..n {
                left[i] *= to;
                right[i] *= to;
            }
        }
    } else {
        let step = (to - from) / n as f32;
        for i in 0..n {
            let g = from + step * (i + 1) as f32;
            left[i] *= g;
            right[i] *= g;
        }
    }
}
