//! `physics-lab` — Phase 3's first demo (issue #136, `docs/demos/physics-lab.md`): small scenes
//! of rigid bodies on a flat floor through `forge-physics` (Jolt 5.6, D-009), each a test with
//! its numbers and its determinism hash. It shares `city-blocks`' renderer and flags
//! (`city_blocks::main_lab`); `--lab` picks the scene (`drop` by default). Space throws a ball
//! from the camera, Enter starts the scene over.

#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    city_blocks::main_lab()
}
