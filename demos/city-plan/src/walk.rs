//! The player walking the plan's streets in first person (D-059's step 4), on the shared walker
//! (`city_blocks::walker`, #290). Enter puts it on the ground under the camera and the camera
//! at its eyes; WASD walks along the view and Shift runs (`city_blocks::wished_walk`, as on the
//! island); Enter again frees the camera. Its pieces are the buildings' walls round it (D-060:
//! each wall a slab of its thickness to its roof, open where a passage runs into a court) and
//! the insides of the houses that have them (floors, flights, partitions, furniture); the ground
//! is one slab at the streets' level.

use anyhow::Result;
use city_blocks::walker::{Ground, Pieces, WalkSettings};
use glam::Vec3;

pub use city_blocks::walker::EYES;

/// The streets' level, metres: what the walker stands on.
const LEVEL: f32 = 0.16;
/// The ground's thickness, metres.
const GROUND_THICKNESS: f32 = 2.0;

/// The pieces to walk against.
pub struct Walls(Pieces);

impl Walls {
    /// The pieces: convex, their corners in the world's metres.
    pub fn new(pieces: Vec<Vec<Vec3>>) -> Self {
        Self(Pieces::new(pieces, &WalkSettings::default()))
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

    /// Walks `walk` (m/s along the ground, the world's x and z) for the ticks `dt` seconds owe.
    pub fn advance(&mut self, walls: &Walls, walk: [f32; 2], dt: f64) {
        self.0.advance(&walls.0, walk, dt);
    }
}
