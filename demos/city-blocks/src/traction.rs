//! The materials' rows as physics reads them (#203, #206, D-007's one material record): the
//! materials lab's patches, the island's ground under the walker's boots, the sole, and the grip
//! a pair of them gives. The materials lab (`physics-lab --lab materials`) and the island's
//! walker both read them.

use forge_core::material::{MaterialTags, PhysicsLayer};

/// Gravity, m/s².
const GRAVITY: f32 = 9.81;

/// A material's row as physics reads it: its name (the prop drawn with it), its physical layer
/// and its tags.
pub struct Row {
    /// Its name: the prop drawn with it, or the ground's in the walker's title.
    pub name: &'static str,
    /// Its physical layer.
    pub physics: PhysicsLayer,
    /// Its tags.
    pub tags: MaterialTags,
}

/// The patches' rows, in the order the walker meets them. Friction is each material's against
/// itself: a pair combines by the geometric mean, restitution by the larger (Jolt's rules, the
/// table's `combine` for now). Jolt holds one coefficient a body, so the bodies take the dynamic
/// one; the static stays for the walker's first step and for later.
pub const ROWS: [Row; 5] = [
    Row {
        name: "lab-mat-brick",
        physics: PhysicsLayer {
            density: 1900.0,
            static_friction: 0.7,
            dynamic_friction: 0.6,
            restitution: 0.4,
        },
        tags: MaterialTags(0),
    },
    Row {
        name: "lab-mat-wood",
        physics: PhysicsLayer {
            density: 600.0,
            static_friction: 0.5,
            dynamic_friction: 0.4,
            restitution: 0.45,
        },
        tags: MaterialTags::FLAMMABLE,
    },
    Row {
        name: "lab-mat-sand",
        physics: PhysicsLayer {
            density: 1600.0,
            static_friction: 0.6,
            dynamic_friction: 0.5,
            restitution: 0.05,
        },
        tags: MaterialTags::DEFORMABLE,
    },
    Row {
        name: "lab-mat-snow",
        physics: PhysicsLayer {
            density: 400.0,
            static_friction: 0.3,
            dynamic_friction: 0.2,
            restitution: 0.1,
        },
        tags: MaterialTags(MaterialTags::DEFORMABLE.0 | MaterialTags::SLIPPERY.0),
    },
    Row {
        // Wet ice near melting, the slipperiest.
        name: "lab-mat-ice",
        physics: PhysicsLayer {
            density: 917.0,
            static_friction: 0.05,
            dynamic_friction: 0.02,
            restitution: 0.6,
        },
        tags: MaterialTags::SLIPPERY,
    },
];

/// The walker's boots: a rubber sole.
pub const SOLE: PhysicsLayer = PhysicsLayer {
    density: 1100.0,
    static_friction: 0.9,
    dynamic_friction: 0.8,
    restitution: 0.5,
};

/// A row of the island's ground (#206): its name as the walker's title shows it, then the
/// friction (static, dynamic, each against itself), the restitution, the density and the tags.
const fn ground(
    name: &'static str,
    friction: [f32; 2],
    restitution: f32,
    density: f32,
    tags: MaterialTags,
) -> Row {
    Row {
        name,
        physics: PhysicsLayer {
            density,
            static_friction: friction[0],
            dynamic_friction: friction[1],
            restitution,
        },
        tags,
    }
}

const DEFORMABLE_SLIPPERY: MaterialTags =
    MaterialTags(MaterialTags::DEFORMABLE.0 | MaterialTags::SLIPPERY.0);

/// The island's ground (#206): a row for each of its layers (`island_layer`, by its id), as the
/// walker's boots meet them. The beach's dry sand is the patches' sand row. Rough rock grips
/// best and wet or loose ground least: the streams' stones under a film of algae, and the
/// lakes' mud.
pub const ISLAND_ROWS: [Row; forge_terrain::island_layer::COUNT as usize] = [
    ground("grass", [0.5, 0.4], 0.2, 1300.0, MaterialTags(0)),
    ground(
        "dry sand",
        [
            ROWS[2].physics.static_friction,
            ROWS[2].physics.dynamic_friction,
        ],
        ROWS[2].physics.restitution,
        ROWS[2].physics.density,
        MaterialTags::DEFORMABLE,
    ),
    ground(
        "wet sand",
        [0.6, 0.5],
        0.05,
        2000.0,
        MaterialTags::DEFORMABLE,
    ),
    ground("granite", [0.8, 0.7], 0.5, 2700.0, MaterialTags(0)),
    ground(
        "wet stones",
        [0.35, 0.25],
        0.3,
        2000.0,
        MaterialTags::SLIPPERY,
    ),
    ground("dry grass", [0.55, 0.45], 0.2, 1300.0, MaterialTags(0)),
    ground("lush grass", [0.4, 0.3], 0.2, 1400.0, MaterialTags(0)),
    ground(
        "riverbank",
        [0.4, 0.3],
        0.1,
        1500.0,
        MaterialTags::DEFORMABLE,
    ),
    ground("mud", [0.3, 0.2], 0.05, 1500.0, DEFORMABLE_SLIPPERY),
    ground("gravel", [0.5, 0.4], 0.3, 1900.0, MaterialTags(0)),
    ground("scree", [0.5, 0.4], 0.3, 1800.0, MaterialTags(0)),
    ground("scrub", [0.5, 0.4], 0.2, 1300.0, MaterialTags(0)),
    ground(
        "silty sand",
        [0.65, 0.55],
        0.05,
        1900.0,
        MaterialTags::DEFORMABLE,
    ),
    ground("shingle", [0.45, 0.35], 0.3, 1800.0, MaterialTags(0)),
    ground("limestone", [0.6, 0.5], 0.5, 2600.0, MaterialTags(0)),
    ground("karst", [0.7, 0.6], 0.5, 2600.0, MaterialTags(0)),
    ground("grus", [0.55, 0.45], 0.1, 1700.0, MaterialTags::DEFORMABLE),
];

/// The pair friction of two rows' coefficients, as Jolt combines them.
pub fn pair(a: f32, b: f32) -> f32 {
    (a * b).sqrt()
}

/// How far the walker's feet change its speed in a tick of `dt` on the row `ground` (m/s): its
/// sole's pair friction with the ground times g, the most the ground can push it by.
pub fn traction(ground: &PhysicsLayer, dt: f32) -> f32 {
    pair(SOLE.dynamic_friction, ground.dynamic_friction) * GRAVITY * dt
}
