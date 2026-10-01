# #118: the steep valleys' look: rocky beds, scree, plants on the walls (2026-10-01)

Before (left, d6699a7) and now (right), frame 60 with the fixed step:
- `look-steep.png`:
  - the logged slot from 3 m (`--view -3083,48.3,-376,123.7,-10`) and from 40 m (`-45°`);
  - up the steep river (`--view -1112,80.1,-3083,-97,4`);
  - the hills from 250 m (`--view -3083,250,-376,123.7,-25`).
- `look-wide.png`: a bench in the highlands from 40 m (`--view -613,327,2510,144,-45`), the
  island from 2.5 km.
- `look-zoom.png`: the slot's sunlit wall from 40 m, cropped: bare rock before, scrub now.

**What changed** (`docs/demos/island.md`, "The steep valleys' look"):
- **Rocky beds:** cobbles and gravel under and beside the water of the reaches falling 2.5–4 %
  or more, and 2 525 boulders on their banks.
- **Scree:** broken rock on the walls' foot up to about 5 m over the water, with 1 212 rubble
  piles scaled down on it.
- **Plants on the walls:** scrub on the valleys' rock up to about 30 m over the water, and on the
  wetter rock of the whole island; dry spurs and cliffs stay bare.

All three textures are procedural (`forge_render::textures`), 210 ms at start.

| View | ꟻLIP mean |
|---|---|
| slot, 3 m | 0.076 |
| slot, 40 m | 0.136 |
| up the steep river | 0.048 |
| the hills, 250 m | 0.037 |
| the bench, 40 m | 0.053 |
| the island, 2.5 km | 0.0041 |
| the first view | 0.0073 |

**Cost:** `shading/layered` +0.05 ms in the slot and +0.08 ms up the steep river (frame
1.960 → 2.057 ms). More pixels meet three layers there: #111's third-layer cost. Elsewhere the
cost is under 0.03 ms.

**Checks:** the capture batch changes the island's two views only (`island60` 0.0062, `water60`
0.0073). The A/B harness and mesh-against-fallback stay at 0 px, and `validate.sh` is clean.

**Next:** the scrub is a texture. It reads as shrubs from a few metres up, but up close against
the walls it is flat. Shrubs as props belong with Phase 8's vegetation (D-013).
