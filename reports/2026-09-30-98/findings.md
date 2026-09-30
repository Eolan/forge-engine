# #98: the highlight's view vector, from each surface (2026-09-30)

**Change:** `sun_light` (`shaders/meshlet.slang`) now builds the sun highlight's half vector
from the direction from the surface to the camera (`s.view`, now set in space too).

**What it replaces:** the camera in the scene frame minus the object-space position.
- For an instance away from its object's origin, that is the direction from the scene's origin
  to the camera, so every rock and building took the same one.
- `legacy_camera_position` kept this after #93 so the ballad would not move; it is gone.
- No shader reads a camera position in world space any more.

## The batch (`tools/captures.sh` before and after, `tools/compare.sh`)

Every image moves, the same on the mesh path and the fallback:

| Image | Pixels that differ | ꟻLIP mean | Largest |
|---|---|---|---|
| ballad, TAA frame 600 | 72 933 | 0.0126 | 0.72 |
| ballad, frame 240 | 72 959 | 0.0184 | 0.69 |
| city south, frame 60 | 66 374 | 0.0126 | 0.43 |
| city orbit, frame 120 | 27 925 | 0.0080 | 0.34 |
| meshlets orbit, frame 120 | 63 352 | 0.0105 | 0.44 |
| island, frame 60 | 14 688 | 0.0023 | 0.30 |
| gallery, frame 60 | 586 | 0.0007 | 0.11 |

- **The A/B harness:** occlusion and cone culling off, `--show-culled`, and the mesh path
  against the fallback all stay at 0 px.
- **The mean brightness** does not move: the ballad 44.31 → 44.32 levels of 255, the city
  99.86 → 99.86, the orbit 109.21 → 109.29.
  - The highlights move to where the sun and the camera put them. They are not stronger or
    weaker on the whole, so the materials' `specular` rows are left as they were.
- **Where it shows:**
  - the ballad's large rocks: faces facing the sun between the camera and the sun catch the
    light, others lose it (`ballad-crop.png`);
  - the city's window panes on the upper floors (`city-crop.png`);
  - `sheet.png` is before, after and the ꟻLIP map for both.

## Far from the origin (`tools/origins.sh`)

Each offset against the origin's image:

| Offset | City | Ballad |
|---|---|---|
| 10⁴ m | 2 730 px, ꟻLIP mean 0.0017 | 1 519 px, 0.0005 |
| 10⁵ m | 2 514 px, 0.0016 | 1 519 px, 0.0005 |
| 10⁶ m | 2 539 px, 0.0016 | 1 518 px, 0.0005 |
| 10⁷ m | 2 486 px, 0.0015 | 1 519 px, 0.0005 |

With the old vector: the city 2 548–2 755 px, the ballad 1 665–1 666
(`docs/demos/city-blocks.md`). The rest is still the offset's rounding inside a cell.

**For the owner:** a look change to judge (`docs/PROCESS.md`, "Look changes"). If a highlight
now reads too strong or too weak, the lever is the material table's `specular` and
`specular_power` rows, not the vector.
