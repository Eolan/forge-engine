# #129: the rocks by the island's geology (2026-10-02)

The owner's pick (D-042): a granite core and a limestone coast. Before (left, `--no-rock-types`)
and now (right), frame 60, `island` (seed 7):
- **`rocks.png`**, by rows:
  - the granite of the hills, from 150 m down its slope (`--view -1279,167,-3122,-59.4,-18.4`),
    ꟻLIP mean 0.076;
  - the limestone of a low hill, with granite in its valley's V (`--view
    -1272,93,-3541,-113.0,-18.4`), 0.075;
  - the karst's pavements on the limestone's dry slopes (`--view -4573,93,-2754,-140.6,-18.4`),
    0.020;
  - the `valley` shot, 0.0055.
- **`island.png`**:
  - the `island` shot, 0.0079;
  - the stretch that was black sand in #128's first version (`--view -5304,18,-343,-56.3,-14.4`),
    left with it and right as it is now, pale sand.

**The rule** (`forge_procgen::paint_geology`, last over the layer map):
- **Rock below the limestone's height** (45 m, give or take 25 m over patches 1.5 km across,
  ragged by 4 m) turns to limestone. Above it, the rock stays the hills' granite.
- **Karst** covers a fifth of the limestone's dry grass on slopes of 0.12 to 0.35, in patches
  25 m across.
- **Three textures** of their own: `textures::granite` (grey-pink, specked with feldspar, quartz
  and mica, slabs, sheet joints, lichen), `textures::limestone` (pale cream-grey, pitted, blotched)
  and `textures::karst` (blocks and mossy fissures).

**Numbers:** 64 070 texels of granite, 1 510 of limestone rock and 17 948 of karst; about
130 ms at start.

**Checks:**
- **The batch:** the island's images change; the A/B harness and the paths are at 0 px. The
  exception is `fb-ast-taa600`, 52 px with an ꟻLIP of at most 0.058: #71's flake, not this
  change.
- **Validation and tests:** `validate.sh` is clean; 257 tests pass.
- **Timings:** `timings.sh` is within noise: the island 1.609–1.619 ms against 1.608–1.611, its
  layered shading 0.312 ms against 0.313.

**Tuned on the way** (seen in the first captures):
- **Granite:** at a tint of 1 it came out nearly white, like snow.
- **Karst:** on the lush plain in blobs 60 m across, it read as snow patches. It now lies on
  drier slopes, grey.
- **The views:** each now looks back up the slope at its target. Standing to the south put
  one inside a hill.
