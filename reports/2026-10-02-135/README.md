# #135: the granite's grus (2026-10-02)

The coarse sand granite rots into (D-042's last step for the rocks): on the granite's gentle
grass within 48 m of its bare rock, half of it, in patches drawn towards the rock. Before (left,
def84e5) and now (right), frame 60, `island` (seed 7):
- **`grus.png`**, by rows:
  - the crest's tors from 50 m (`--view -249,548,-84,-60.5,-18.4`), ꟻLIP mean 0.0215;
  - the tors from the grass (`--view -150,512,-140,-60.5,-12`), 0.0199;
  - the granite's hill from 150 m (`--view -1279,167,-3122,-59.4,-18.4`), 0.0099;
  - the `island` shot, 0.0103.
- **`close.png`:** the second row's grus at twice the size, and where the frame changes.

**Numbers:** 76 257 texels of grus (1.2 km²) beside 64 070 of bare granite; the karst unchanged.
About 11 ms more at start.

**Checks** (Tier 0):
- **The gate:** fmt, clippy, 261 tests; the credits are up to date.
- **The sentinels**, against #133's set: 0 px.
- **The island's captures**, recooked: the views by about 4 700 px (ꟻLIP mean 0.0024), the shots
  by 0 to 13 060 px. The A/B pairs and streamed against resident: 0 px.
- **Left for Tier 2:** the fallback path, validation and timings.

**Tuned on the way:** without the rock near, 6 % of the granite's dry grass put pale blobs over
the open grass, as the karst's first try did.
