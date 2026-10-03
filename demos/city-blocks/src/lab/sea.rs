//! `physics-lab --lab sea` (issue #138, Phase 3's step 3): things that float, on the sea this
//! demo renders. The waves' heights come from the same spectrum as the GPU's cascades, the long
//! ones (`forge_procgen::SeaHeights`), every tick; each floating body's hull is cut at the
//! surface and pushed by the water (`forge_physics::buoyancy`). A jetty stands on pillars; a
//! boat modelled in Blender (`assets/blender/boat.py`, read through glTF) runs on an outboard
//! motor that pushes only while its propeller is in the water.

use std::sync::{Arc, OnceLock};

use anyhow::{Context as _, Result};
use forge_geom::city::{Block, Imported, Lathe, PropKind, PropSpec};
use forge_geom::model::{Model, load_glb};
use forge_physics::buoyancy::{Fluid, Hull, Water, push};
use forge_physics::{BodyId, Transform, World};
use forge_procgen::{Ocean, OceanParams, SeaHeights};
use forge_task::TaskPool;
use glam::{DVec3, Quat, Vec3};

/// The sea's level, metres, and its floor under the jetty.
pub(super) const LEVEL: f64 = 0.0;
pub(super) const FLOOR_Y: f32 = -12.0;
/// The cascades whose waves move what floats: the swell and the waves down to 4 m (the
/// ripples' centimetres are left to the eye).
const PHYSICS_CASCADES: usize = 2;
/// The jetty: its deck's top, its half sizes, the pillars' half side and how many a row.
pub(super) const DECK_TOP: f32 = 1.4;
pub(super) const DECK_HALF: [f32; 3] = [1.5, 0.15, 11.0];
pub(super) const PILLAR_HALF: f32 = 0.3;
pub(super) const PILLARS: u32 = 6;
/// The crates' half side and the logs' radius and length, metres.
pub(super) const CRATE_HALF: f32 = 0.35;
pub(super) const LOG_RADIUS: f32 = 0.2;
pub(super) const LOG_LENGTH: f32 = 3.0;
/// The boat: its mass, kg, how far under its hull's centre its weight sits, metres, the most
/// its motor pushes, N, and where the propeller turns in its frame.
const BOAT_MASS: f32 = 420.0;
const BOAT_WEIGHT_LOW: f32 = 0.3;
const BOAT_THRUST: f32 = 2600.0;
const PROPELLER: Vec3 = Vec3::new(0.0, -0.62, 2.64);
/// How far the outboard turns at full rudder: the thrust's sideways share.
const RUDDER_TURN: f32 = 0.55;

/// The sea's spectrum: the island's, from the same seed (D-038's cascades).
pub(super) fn oceans() -> Vec<Ocean> {
    let seed = forge_core::Seed::new(7).derive(0x5EA);
    OceanParams::cascades(seed).map(Ocean::new).into()
}

/// The boat's model, read once from `assets/models/boat.glb`.
pub(super) fn boat_model() -> &'static (Model, String) {
    static MODEL: OnceLock<(Model, String)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/boat.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the boat's model {}: {e}", path.display()));
        // The cache's key: the file and its bytes' digest, so a new export cooks again.
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        (model, format!("assets/models/boat.glb#{digest:016x}"))
    })
}

/// The sea's props: the crate, the log, the pillar, the deck, the boat.
pub(super) fn props() -> Vec<PropSpec> {
    let (model, key) = boat_model();
    let boat = model.mesh("boat").expect("the model's boat");
    vec![
        PropSpec {
            name: "lab-crate".to_owned(),
            kind: PropKind::Block(Block {
                half: [CRATE_HALF; 3],
                radius: 0.025,
                segments: 4,
            }),
        },
        PropSpec {
            name: "lab-log".to_owned(),
            kind: PropKind::Lathe(Lathe {
                profile: vec![
                    (0.0, 0.0),
                    (LOG_RADIUS - 0.03, 0.0),
                    (LOG_RADIUS, 0.03),
                    (LOG_RADIUS * 0.97, LOG_LENGTH * 0.5),
                    (LOG_RADIUS, LOG_LENGTH - 0.03),
                    (LOG_RADIUS - 0.03, LOG_LENGTH),
                    (0.0, LOG_LENGTH),
                ],
                around: 40,
                along: 48,
                flutes: 0,
                flute_depth: 0.0,
                flute_span: (0.0, 0.0),
            }),
        },
        PropSpec {
            name: "lab-pillar".to_owned(),
            kind: PropKind::Block(Block {
                half: [PILLAR_HALF, 0.5 * (DECK_TOP - FLOOR_Y), PILLAR_HALF],
                radius: 0.03,
                segments: 8,
            }),
        },
        PropSpec {
            name: "lab-deck".to_owned(),
            kind: PropKind::Block(Block {
                half: DECK_HALF,
                radius: 0.04,
                segments: 16,
            }),
        },
        PropSpec {
            name: "lab-boat".to_owned(),
            kind: PropKind::Imported(Imported {
                key: key.clone(),
                mesh: Arc::new(boat.mesh.clone()),
            }),
        },
    ]
}

/// The waves where the physics wants them: the long cascades at one instant.
pub(super) struct Sea {
    oceans: Vec<Ocean>,
}

impl Sea {
    pub(super) fn new() -> Self {
        let mut oceans = oceans();
        oceans.truncate(PHYSICS_CASCADES);
        Self { oceans }
    }

    /// The surface `time` seconds in.
    pub(super) fn at(&self, time: f64, pool: &TaskPool) -> SeaHeights {
        SeaHeights {
            cascades: self
                .oceans
                .iter()
                .map(|o| o.displacement(time, pool))
                .collect(),
            level: LEVEL,
        }
    }
}

/// [`SeaHeights`] as the buoyancy reads the water.
pub(super) struct Surface<'a>(pub &'a SeaHeights);

impl Water for Surface<'_> {
    fn height(&self, x: f64, z: f64) -> f64 {
        self.0.height(x, z)
    }
}

/// A body the water pushes, and its hull among the world's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Floater {
    pub body: BodyId,
    pub hull: usize,
}

/// Pushes every floater by `water` (of `fluid`) this tick: their states read in three calls, the
/// pushes worked out in parallel (each alone, so the same with any workers), applied in one call.
/// A body asleep wholly under the water (a rock on the floor) is left asleep.
pub(super) fn float(
    world: &mut World,
    floaters: &[Floater],
    hulls: &[Hull],
    water: &(impl Water + Sync),
    fluid: &Fluid,
    pool: &TaskPool,
) {
    let bodies: Vec<BodyId> = floaters.iter().map(|f| f.body).collect();
    let (mut transforms, mut velocities, mut centers, mut awake) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    world.transforms(&bodies, &mut transforms);
    world.velocities(&bodies, &mut velocities);
    world.centers_of_mass(&bodies, &mut centers);
    world.awake(&bodies, &mut awake);
    let mut pushes = vec![None; floaters.len()];
    let surface = water;
    pool.scope(|scope| {
        for (k, out) in pushes.chunks_mut(16).enumerate() {
            let (floaters, transforms, velocities, centers, awake, surface) = (
                floaters,
                &transforms,
                &velocities,
                &centers,
                &awake,
                surface,
            );
            scope.spawn(move |_| {
                for (i, slot) in out.iter_mut().enumerate() {
                    let n = k * 16 + i;
                    let p = push(
                        &hulls[floaters[n].hull],
                        transforms[n],
                        velocities[n],
                        centers[n],
                        surface,
                        fluid,
                    );
                    let sunk = p.buoyancy.y
                        >= hulls[floaters[n].hull].volume() * fluid.density * fluid.gravity * 0.999;
                    if awake[n] || !sunk {
                        *slot = Some((p.force, transforms[n].position, p.torque));
                    }
                }
            });
        }
    });
    let (bodies, pushes): (Vec<BodyId>, Vec<(Vec3, DVec3, Vec3)>) = bodies
        .iter()
        .zip(&pushes)
        .filter_map(|(&b, p)| p.map(|p| (b, p)))
        .unzip();
    world.push(&bodies, &pushes);
}

/// The boat: its body, and its motor's throttle (−1 astern to 1 ahead) and rudder (−1 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Boat {
    pub body: BodyId,
    pub throttle: f32,
    pub rudder: f32,
}

impl Boat {
    /// The motor's push this tick, while its propeller is under the surface: along the boat,
    /// turned by the rudder (an outboard turns its whole thrust), at the propeller.
    pub(super) fn drive(&self, world: &mut World, sea: &SeaHeights) {
        if self.throttle == 0.0 {
            return;
        }
        let mut t = Vec::new();
        world.transforms(&[self.body], &mut t);
        let at = t[0].position + (t[0].rotation * PROPELLER).as_dvec3();
        if sea.height(at.x, at.z) <= at.y {
            return;
        }
        // Forward is −z; the sideways share without trigonometry, the length normalised.
        let way = Vec3::new(-self.rudder * RUDDER_TURN, 0.0, -1.0).normalize();
        let force = t[0].rotation * way * (self.throttle * BOAT_THRUST);
        world.push(&[self.body], &[(force, at, Vec3::ZERO)]);
    }
}

/// The boat's shapes: its collision hull (the buoyancy shell's points, its weight lowered) and
/// its buoyancy hull.
pub(super) fn boat_shapes() -> Result<(forge_physics::Shape, Hull)> {
    let (model, _) = boat_model();
    let shell = model
        .mesh("boat-buoyancy")
        .context("the model's buoyancy shell")?;
    let points: Vec<Vec3> = shell
        .mesh
        .positions
        .iter()
        .map(|&p| Vec3::from(p))
        .collect();
    let triangles: Vec<[u32; 3]> = shell.mesh.indices.as_chunks::<3>().0.to_vec();
    let hull = Hull::new(&points, &triangles);
    let shape = forge_physics::Shape::convex_hull(&points, 0.02, 100.0)?
        .with_center_of_mass_offset(Vec3::new(0.0, -BOAT_WEIGHT_LOW, 0.0))?;
    Ok((shape, hull))
}

/// The boat's mass, kg.
pub(super) fn boat_mass() -> f32 {
    BOAT_MASS
}

/// Where the boat starts: off the jetty's end, turned a little across the waves.
pub(super) fn boat_start() -> Transform {
    Transform {
        position: DVec3::new(6.0, 0.2, -14.0),
        rotation: Quat::from_xyzw(0.0, 0.25, 0.0, 1.0).normalize(),
    }
}

/// The boat's motor and rudder at rest.
pub(super) fn boat_still(body: BodyId) -> Boat {
    Boat {
        body,
        throttle: 0.0,
        rudder: 0.0,
    }
}
