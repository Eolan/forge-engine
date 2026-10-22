//! The player walking the plan's streets in first person (D-059's step 4), on the shared walker
//! (`city_blocks::walker`, #290). Enter puts it on the ground under the camera and the camera
//! at its eyes; WASD walks along the view and Shift runs (`city_blocks::wished_walk`, as on the
//! island); Enter again frees the camera. Its pieces are the buildings' walls round it (D-060:
//! each wall a slab of its thickness to its roof, open where a passage runs into a court) and
//! the insides of the houses that have them (floors, flights, partitions, furniture); the ground
//! is one slab at the streets' level. It crosses on its own what it walks into (#316, the owner
//! on 2026-10-19): a low garden wall is vaulted, an open window of a house with insides climbed
//! through.

use anyhow::Result;
use city_blocks::walker::{Ground, Pieces, WalkSettings};
use forge_nav::{CrossingSpec, Passage, PassageKind, PassageState, Passages};
use glam::Vec3;

pub use city_blocks::walker::EYES;

/// The streets' level, metres: what the walker stands on.
const LEVEL: f32 = 0.16;
/// The ground's thickness, metres.
const GROUND_THICKNESS: f32 = 2.0;

/// The pieces to walk against and the passages to cross.
pub struct Walls(Pieces, Passages);

impl Walls {
    /// The pieces (convex, their corners in the world's metres) and the passages.
    pub fn new(pieces: Vec<Vec<Vec3>>, passages: Vec<Passage>) -> Self {
        Self(
            Pieces::new(pieces, &WalkSettings::default()),
            Passages::new(passages),
        )
    }

    /// How many pieces stand between `a` and `b` (the world's metres): the walls and floors a
    /// sound passes through (#318).
    pub fn between(&self, a: Vec3, b: Vec3) -> usize {
        self.0.between(a, b)
    }
}

/// What the player crosses on its own (D-076's defaults): an open or broken window is climbed
/// through and a low wall vaulted, a person's way; a tall wall is a climber's, a door is walked.
fn crossing(p: &Passage) -> Option<CrossingSpec> {
    let open = matches!(p.state, PassageState::Open | PassageState::Broken);
    match p.kind {
        PassageKind::Window | PassageKind::LowFence if open => {
            Some(CrossingSpec::person(p.crossing()))
        }
        _ => None,
    }
}

/// The walker.
pub struct Walker(city_blocks::walker::Walker);

impl Walker {
    /// A walker on the ground under `at` (the world's x and z).
    pub fn new(walls: &Walls, at: Vec3) -> Result<Self> {
        city_blocks::walker::Walker::new(
            &walls.0,
            &Ground::slab(LEVEL, GROUND_THICKNESS),
            Vec3::new(at.x, LEVEL + 0.01, at.z),
            WalkSettings::default(),
        )
        .map(Self)
    }

    /// Its feet, the world's metres.
    pub fn feet(&self) -> Vec3 {
        self.0.feet()
    }

    /// Its feet as drawn, between its last two ticks (`Walker::drawn_feet`): where the camera
    /// follows it, at any refresh rate.
    pub fn drawn_feet(&self) -> Vec3 {
        self.0.drawn_feet()
    }

    /// Walks `walk` (m/s along the ground, the world's x and z) for the ticks `dt` seconds owe,
    /// crossing what it walks into.
    pub fn advance(&mut self, walls: &Walls, walk: [f32; 2], dt: f64) {
        self.0
            .advance_across(&walls.0, &walls.1, walk, dt, crossing);
    }
}
