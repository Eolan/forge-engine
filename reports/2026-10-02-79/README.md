# #79: moving geometry, the first step (2026-10-02)

Barrels drifting down the island's four largest rivers (`city-blocks --island 7 --movers N`,
`docs/demos/island.md`, "Moving geometry"). Frame 60 with the fixed step, 1600 × 900, 1 000
barrels.

- `barrels.png`: the barrels on the largest river by its mouth (`4384,4.0,-2840,-88.9,-20`), and
  the first barrel from 4 m, in the lake at that river's head
  (`-238.2,318.14,-1843.9,135.2,-18.1`, the log's `barrels on the rivers` view).
- `taa.png`, the barrel cropped:
  - without TAA (`--no-taa`);
  - with TAA and the movers' motion vectors;
  - with TAA and the camera's alone (`--no-mover-motion`). The hoops blur and the rim ghosts:
    2 394 pixels differ from the second, ꟻLIP max 0.27.

**Cost** (2560 × 1440, two rounds):
- From the barrel's view, 3.62 ms with no movers, 3.64 with 1 000, 3.71 with 10 000.
- `movers/upload` 0.002–0.006 ms, `movers/cell bounds` 0.003 ms, `movers/motion` 0.032 ms.

**Checks:**
- Without movers the capture batch is at 0 px (42 images), and so are the A/B harness, streamed
  against resident and mesh against fallback.
- `validate.sh` is clean, with 1 000 movers on both paths, synchronisation validation included.
- The tests pass (217); clippy and fmt are clean.

**The second step: their acceleration structure** (rebuilt every frame, traced after the static
one; `docs/demos/island.md`, "Their acceleration structure"):
- `rays.png`: the nearest barrel at the mouth from 2.5 m (`4404,2.5,-2840.4,-88.9,-25`, cropped
  and enlarged), without the rays (`--no-shadows`), with them, and the pixels that changed: the
  water now mirrors the barrel.
- Cost at 2560 × 1440: about 0.3 ms with 1 000 movers (the build 0.14 ms, a second traversal for
  every ray); 10 000 add little more.
- Without movers the batch is unchanged; `validate.sh` is clean with them on both paths.
