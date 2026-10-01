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
