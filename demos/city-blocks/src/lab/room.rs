//! `physics-lab --lab room` (issue #159): a plain room to find where the image loses its
//! sharpness, after the owner's report of 2026-10-03 ("I feel like the image is always a bit
//! blurry of fuzzy, never clear and neat as it should ... we can start with a very simple
//! environment, simple geometry. in a room with a simple light source").
//!
//! White matte walls on three sides, open to the sky above, and a floor of black and white
//! squares (12.5 cm). The sun alone lights it, with no sky light; the background is plain. On the
//! back wall, 8 m from the camera's start, stand three black squares turned 5° off the pixel grid.
//! A white board 2.5 m from the camera holds a fourth. Their edges are what `tools/sharpness`
//! measures (the slanted-edge method). The squares are flat colour, so their edges' softness is
//! the pipeline's own: the raster, TAA, bloom, the tone curve. The floor seen at a slant is the
//! textures' filtering. Nothing moves.

use glam::{Mat4, Quat, Vec3};

use forge_geom::city::{Block, PropKind, PropSpec};

/// The room's inside: half its width (x) and depth (z), and its height, metres. Its floor's top
/// is at 0, the back wall's face at z = -BACK.
const HALF_WIDTH: f32 = 4.0;
const BACK: f32 = 5.0;
const FRONT: f32 = 5.0;
const HEIGHT: f32 = 3.0;
/// The walls' and floor's half thickness.
const SLAB: f32 = 0.05;
/// The targets: half their side, how far they turn off the grid (degrees), their half thickness
/// (thin, so their shadow on the wall stays under a pixel).
const TARGET_HALF: f32 = 0.3;
const TURN: f32 = 5.0;
const THIN: f32 = 0.001;
/// The board near the camera: half its side and thickness, where its face stands (z), its centre's
/// height (low, so it hides none of the wall's targets from the camera's start) and its target's
/// half side.
const BOARD_HALF: f32 = 0.6;
const BOARD_THICK: f32 = 0.01;
const BOARD_Z: f32 = 0.5;
const BOARD_Y: f32 = 0.75;
const BOARD_TARGET_HALF: f32 = 0.25;
/// The targets' centres' height over the floor.
const TARGET_Y: f32 = 1.5;

/// The room's sun: from the front left, high, so the back wall and the board face it.
pub(crate) const SUN: Vec3 = Vec3::new(-0.35, 0.8, 0.5);

/// The props, in this order: the floor, the back wall, a side wall, a target, the board.
pub(super) fn props() -> Vec<PropSpec> {
    let block = |name: &str, half: [f32; 3], radius: f32| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half,
            radius,
            segments: 4,
        }),
    };
    vec![
        block(
            "lab-room-floor",
            [HALF_WIDTH, SLAB, 0.5 * (BACK + FRONT)],
            0.002,
        ),
        block(
            "lab-room-wall",
            [HALF_WIDTH + 2.0 * SLAB, 0.5 * HEIGHT, SLAB],
            0.002,
        ),
        block(
            "lab-room-side",
            [SLAB, 0.5 * HEIGHT, 0.5 * (BACK + FRONT)],
            0.002,
        ),
        // The least rounding a block takes (a millimetre), so an edge is a step in the light.
        block("lab-room-target", [TARGET_HALF, TARGET_HALF, THIN], 0.001),
        block(
            "lab-room-board",
            [BOARD_HALF, BOARD_HALF, 0.5 * BOARD_THICK],
            0.001,
        ),
        block(
            "lab-room-board-target",
            [BOARD_TARGET_HALF, BOARD_TARGET_HALF, THIN],
            0.001,
        ),
    ]
}

/// Where the room's props stand (`first`: the floor's prop, the others after it as in
/// [`props`]). Nothing in the room moves, so the world holds none of it.
pub(super) fn build(first: usize) -> Vec<(usize, Mat4)> {
    let turn = Quat::from_rotation_z(TURN.to_radians());
    let mid_z = 0.5 * (FRONT - BACK);
    let mut statics = vec![
        (first, Mat4::from_translation(Vec3::new(0.0, -SLAB, mid_z))),
        (
            first + 1,
            Mat4::from_translation(Vec3::new(0.0, 0.5 * HEIGHT, -BACK - SLAB)),
        ),
    ];
    for x in [-1.0, 1.0] {
        statics.push((
            first + 2,
            Mat4::from_translation(Vec3::new(x * (HALF_WIDTH + SLAB), 0.5 * HEIGHT, mid_z)),
        ));
    }
    // Three targets on the back wall, a little proud of it.
    for x in [-2.0, 0.0, 2.0] {
        statics.push((
            first + 3,
            Mat4::from_rotation_translation(turn, Vec3::new(x, TARGET_Y, -BACK + THIN)),
        ));
    }
    // The board, its face at BOARD_Z, and its target on that face.
    statics.push((
        first + 4,
        Mat4::from_translation(Vec3::new(0.0, BOARD_Y, BOARD_Z - 0.5 * BOARD_THICK)),
    ));
    statics.push((
        first + 5,
        Mat4::from_rotation_translation(turn, Vec3::new(0.0, BOARD_Y, BOARD_Z + THIN)),
    ));
    statics
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_targets_stand_proud_of_what_holds_them() {
        let statics = build(0);
        let z_of = |prop: usize| -> Vec<f32> {
            statics
                .iter()
                .filter(|(p, _)| *p == prop)
                .map(|(_, m)| m.w_axis.z)
                .collect()
        };
        // The back wall's face is at -BACK; the targets' backs touch it.
        for z in z_of(3) {
            assert!((z - THIN - -BACK).abs() < 1e-6);
        }
        // The board's face at BOARD_Z; its target's back on it.
        let board = z_of(4)[0] + 0.5 * BOARD_THICK;
        assert!((board - BOARD_Z).abs() < 1e-6);
        assert!((z_of(5)[0] - THIN - BOARD_Z).abs() < 1e-6);
    }
}
