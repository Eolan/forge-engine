# #130: the boulders where rocks gather (2026-10-02)

The owner, on #129's result: the boulders by the rocks, "more varied shapes and size", "and
places that make sense. There's quite a lot, maybe too much". Before (left, `--no-rock-sites`:
300 000 of the city's boulders and rubble) and now (right: 60 000 stones of the island's granite
and limestone), frame 60, `island` (seed 7):
- **`rocks.png`**, by rows:
  - the granite's hill from 150 m (`--view -1279,167,-3122,-59.4,-18.4`), ꟻLIP mean 0.119:
    rounded corestones, tors on the crest, a few on the face;
  - the foot of a granite face (`--view -1272,93,-3541,-113.0,-18.4`), 0.080: the talus in the
    granite it fell from, though it lies on the limestone;
  - the karst (`--view -4573,93,-2754,-140.6,-18.4`), 0.084: loose blocks on the pavements;
  - the granite's slope from 40 m (`--view -1184,140,-3178,-59.4,-15`), 0.174: the shapes and
    the sizes.
- **`shots.png`**, by rows:
  - the talus from close (`--view -1189,62,-3505,-113.0,-8`), 0.103;
  - the `valley` shot, 0.171: the dark rubble gone from its crests (the whole frame moves with
    the probes' light and the metered exposure, mean 75.7 → 75.0 levels);
  - the `island` shot, 0.0089;
  - the first view, `water60` of the batch, 0.0105.

**Where** (`forge_procgen::rock_sites`, 8 m cells): talus below the steep ground (1), the scree
(1), the granite's crests (0.6), the karst (0.35), the steep faces (0.06), a scatter on slopes
from 0.05 (0.004); grouped in patches 60 m across. None on the beaches, in the rivers or the
lakes. The GPU draws each rock's cell from its rock's cumulative weights
(`placement::RockRule::Sites`).

**Shapes** (`forge_geom::stone`): granite's corestones, slabs and a tor; limestone's blocks
(bedding, joints, solution pits) and flags. Sizes 0.3 to 2.2 times the mesh, mostly small. They
lean with the ground.

**Numbers:**
- **The rocks:** 49 494 granite and 10 506 limestone. By site: 28 511 talus, 9 579 on crests,
  8 217 on the karst, 8 493 scattered, 4 914 on faces and 286 on the scree.
- **The costs:** the map takes about 570 ms at start. The placed triangles fall from 196 G to
  8.7 G.

**Checks:**
- **The batch:** only the island's images change. The exception is `mesh-ast-taa600`, at
  302 px: #71's flake. Two runs each of the old and the new build give the same frame, 0 px.
- **The A/B harness and mesh against fallback:** 0 px.
- **The city's placement:** unchanged, checksum `4e10743a3499dc0e`. The island's placement
  matches its CPU mirror.
- **Validation and tests:** `validate.sh` is clean; 261 tests pass.
- **Timings:** `timings.sh`, 1600 × 900. The island takes 1.493–1.507 ms against 1.583–1.620,
  the tour 1.315–1.327 against 1.409–1.437. Among the zones, the probe rays take 0.274 ms
  against 0.336 and the software raster 0.024 against 0.164. The rest is within noise.

**Tuned on the way:**
- **The scatter:** at 0.02, on any ground, it put pale blocks evenly over the plain.
- **The faces:** at 0.25 they took 27 % of the rocks.
- **The talus:** it took the ground's rock at first, which put limestone under granite faces.
