# Morning report, 2026-10-25

Tracking issue for your walk: Eolan/forge-engine#354. Pictures in `reports/2026-10-25-354/`.

## Your list, item by item

**Done and pushed (each view shot before and after):**
- **Small things on the floor:** a ground-floor pendant's brass canopy came through the floor above. It was placed from the raised floor but sized under the storey's ceiling. The upper floors are also triangulated without T-junctions round the stair opening.
- **Bike racks, walled-off corners, paths through private gardens:** the village's own lots were being taken for "gaps between buildings". Fixed at the source.
- **Path intersections:** bends are mitred so they no longer narrow, and paths end round where they join.
- **Walls higher than their neighbours:** one street height round each block; party walls are no higher.
- **Walls and hedges not fitting the gardens (your red marks):**
  - leftover ground between two streets' strips, and lots too small for a house, go to the neighbouring garden;
  - so do small squares closed all round (the "small closed square for nothing").
- **The gate by the house corner:** a gate now stands at least 1.5 m from the house, past the planting.
- **Light leaking on the roof:** the house body's lighting region reached over the roof's edge, where lamps are unshadowed. Fixed, and the walls' hollow tops are closed.
- **The pattern after going through a window:** DLSS wasn't told about the exposure change at night (its pre-exposure was clamped). It now follows the change, and the pattern is gone.
- **Horizontal lines in the round crowns:** the crowns' leaf lumps sat in grid layers. They are now gathered round seeds.
- **The circled white specks on the walls at night:**
  - They were thin faces set into the wall: an open shutter's edge, a balcony bracket.
  - Their lighting lookups landed inside the house's body, so they took a lit room's light and every lamp of the frame.
  - They now take the side they are seen from. The four specks went from brightness 150–230 to the wall's own 25–35.
- **Floating crown without a trunk:** the trunk is always drawn now. That young pine was also one of the few poorly grown individuals (16 leaf clusters, a few tufts on a stick). Such a plant is now grown again from another seed (2953135a), and it is a full young crown.
  - Of the village's 90 tree models, only that one changed.
- **Gulls floating:** their legs are under the water now.
- **Decisions you gave:** animals on by default; #285 and #287 closed; D-079 accepted with your parallax note.

**Done and pushed in the evening (the gate, the full capture batch, the map check and CI green on each push):**
- **Crowns "as if the generation stopped too early":**
  - Every broad crown grew into the same smooth ball, because the grower filled one smooth envelope evenly.
  - Each tree's envelope is now a cluster of rounded domes, the sub-crowns of its main limbs: holm oaks many small ones, planes a few heavy ones, the cypress barely rippled.
  - The domes deepen with age.
  - Seen from above, crowns went from 0.92–0.95 round to 0.64–0.85.
  - A planted tree's whole crown now grows above its clear stem. Before, a young umbrella pine's cone was cut in half, leaving a spire on a stick.
- **Shimmering on the water / the "trails" behind the gulls:**
  - The water reflected clouds from a sky table about one texel per 18 pixels, which gave blocky columns that slid as you moved.
  - It now uses the screen's own cloud image wherever the sky is visible there; elsewhere it keeps the table as before.
  - Side by side the cloud reflections are soft clouds instead of streaks, and steady from frame to frame.
- **Room insides changing as you approach:**
  - Rooms within about 20 m have their own probe grid. Farther rooms were lit by coarser probes, darker, so rooms jumped from dark to lit as you came close.
  - Far rooms now take a share of the sky light, measured against the grid-lit rooms.
  - It matches within 0.02 EV in the afternoon and under a low sun. The perfect match changes with the hour, filed as #356.
- **The light spots at the foot of the landing's guard wall (4058390a):**
  - A row of the hall's lighting probes stood inside the 10 cm guard. The engine moved each one out on the nearer side, the stairwell's, and from there it lit the landing floor: one spot per metre.
  - Grid probes now stay where they are placed. One that ends up inside a solid is switched off, and its neighbours light around it.
  - At your view the spots are gone. Elsewhere the insides barely change (the hall a little darker) and the outside views not at all.

**Partly done, your call:**
- **Window lights like spotlights:** the night bloom is now a quarter of the day's. The deeper cause, from the lighting research (`docs/research/home-lighting.md`, 57 sources):
  - The night is exposed for moonlight, about EV −2. That makes ordinary house lights glare.
  - D-080 proposes exposing the night for the lit places. It's built and off by default (`r.exposure.night_floor`).
  - `reports/2026-10-25-354/d080-night-key.png` compares today (left) with floor 0 (middle) and floor 1 (right) at three night views. With the floor, you see the rooms behind the windows instead of white.
  - Say if you want floor 0 as the default.
- **#339 downloads (CC0, ambientCG), the links you asked for:**
  - https://ambientcg.com/get?file=SurfaceImperfections003_1K-PNG.zip (12.9 MB)
  - https://ambientcg.com/get?file=SurfaceImperfections013_1K-PNG.zip (14.2 MB)
  - https://ambientcg.com/get?file=SurfaceImperfections015_1K-PNG.zip (18.3 MB, dust)
  - https://ambientcg.com/get?file=Smear005_1K-PNG.zip (4.2 MB)
  - https://ambientcg.com/get?file=Smear006_1K-PNG.zip (13.7 MB)
  - Listing: https://ambientcg.com/list?category=SurfaceImperfections

## Found on the way

- **#355, a fragile shader:**
  - Two small additions to the lighting lookups, code that never runs in the materials lab, made the lab's moving creatures lose their shadows.
  - It's not a race (the same with full barriers), and validation is clean. It looks like slangc or the NVIDIA driver once the shading shader passes some size.
  - Worked round by making the shader smaller than before. Filed with the reproduction steps. A report upstream would need your go.
- **A review agent read the day's 12 commits.** Its findings, checked one by one:
  - **Fixed:**
    - ground-floor sconces placed 0.32 m too low;
    - shrubs' millimetre stems drawn by the thousand (from my trunk fix);
    - a wall end turned along itself at some joints;
    - a leftover lot that could vanish;
    - narrow planned crowns widened into the facades: trees in tight spots are now drawn smaller, not squeezed.
  - **Tested and not reproduced:** DLSS's pre-exposure going very low in sunlight. At 1/32 and 1/64 with the sun in frame, the image stays clean.
- **A second review read the evening's six commits** and found two real bugs and one consequence. All fixed in 8943bc06:
  - **Far-room rule:** it also applied in scenes without room grids (for example `--no-probe-rooms`).
  - **Planted crowns:** the lift was measured against the full-grown height, so young planted crowns were still cut at their widest. Now no planted tree is widest at its lift, and a test holds that.
  - **Street trees and birds:** where the street's room narrowed a crown, the tree kept its full height and was drawn shorter, with birds perching in mid-air above it. The plan now keeps a street tree at most 4 times its crown radius tall.
  - **Also:** a shelf in shrubs' crowns; impostors sized from the planned crown instead of the drawn one.

## Left, with what I found

- **Trees with `--tree-ladder --impostors`:**
  - **Measured at the calibration views:** cards, shells and impostors still match within about 5 % in brightness and hue. Impostors are more contrasty, and 6 % brighter on the green.
  - **Forced onto nearer trees:** impostors are clearly lighter and yellower (+18 to +27 %), which matches "changing colour".
  - **The second cause:** each species and age gets one impostor while the trees use three different individuals, so a tree can change shape as it crosses. The new lobed crowns make individuals differ more.
  - **Your call:** baking one impostor per individual would roughly triple their memory against D-075's budget. A short flight with you to see which pops you mean would help.
- **Seen on the way:** inside or right next to a crown, the dark cores of the leaf clumps show as dark faceted stones through the gaps. Say if you notice them; they can be made smaller or leafier.

## Pictures (`reports/2026-10-25-354/`)

- `d080-night-key.png`: the night exposed for the moon today (left), floor 0 (middle), floor 1 (right), at the key, window and street views. This is D-080's choice.
- `crowns-before.png`, `crowns-after.png`: the species sheet from the side and from above.
- `canopy-before-after.png`: your canopy view.
- `water-clouds-before-after.png`: the river's clouds from the sky table, then from the frame.
- `water-clouds-motion.png`: three frames of the dolly, old above, new below.
- `ladder-cards-shells-impostors.png`: every tree forced to cards, shells and impostors.
- `wall-foot-spots-before-after.png`: your landing view, before and after.
- `young-pine-before-after.png`: your floating-crown view, before and after the regrowth.

## Commits since yesterday's report (all #354 unless noted)

67f51e6e, 78826d94, e5c58d01, f6996974, 7e443aa1, be7e4a4f, ed4b577c, 7684c05e, 44bd4dd4 (research), ba270e18, 5c6a95cb, d19e01c6, 4911a98c, da9248f1 (#355), 6777994c (review fixes), 97facb82 (crowns), 2159d862 (clouds), c1dea1bf (far rooms), 4058390a (the wall-foot spots), 2953135a (the young pine), 8943bc06 (second review fixes). New issues: #355, #356.
