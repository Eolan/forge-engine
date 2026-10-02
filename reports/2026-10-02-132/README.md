# #131 and #132: the stones' LOD and the rivers' cobbles (2026-10-02)

- **`lod-pops.png`** (#131): the glide past the granite's stones (`--view
  -1184,140,-3178,-59.4,-15 --dolly 5 --fixed-step --no-taa`), frame 150.
  - Left, the frame. Right, the pixels that change to frame 151 by more than 16 levels where the
    0.125 px run does not.
  - Top, the stones cooked by their geometry alone; bottom, their normals weighed 2 per metre.
  - Pops per frame pair over 80 pairs: 6 415 px at 0, 5 824 at 0.5, 4 371 at 1, 3 142 at 2.
- **`cobbles.png`** (#132):
  - top, a river's stones from 3 m above the water (`--view 672,200,-1957,-88.7,-25`), before
    and now (ꟻLIP mean 0.0093);
  - bottom, the `valley` shot (0.023).

**Checks:**
- **The batch:** it changes the island's images only. `mesh-ast-taa600` is at 302 px, #71's flake;
  no shader changed.
- **Harnesses:** the A/B harness and mesh against fallback are at 0 px.
- **Validation and tests:** `validate.sh` is clean; 261 tests pass.
- **Timings:** the island takes 1.501–1.503 ms against 1.491–1.493.
