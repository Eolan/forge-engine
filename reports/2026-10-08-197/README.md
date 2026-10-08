# #197: the beach's sand as a window round the walker

`city-blocks --island 7 --walker --walk 0,-1.5 --view=0,25,5214,0,-50 --fixed-step`, frame 300.
- `trail.png`: the walker five seconds up the southern beach, its footprints behind it, left and
  right of its way.
- `prints.png`: the same, the trail enlarged three times.
- `untouched-diff.png`: the pixels that differ by more than 2 levels between the window untouched
  (the walker standing still) and `--no-sand-window`. They are 607 of 1.44 M, at most 10 levels,
  lone pixels with no outline; ꟻLIP mean 0.0047.

**What it is**
- A deformable layer (`forge_physics::deform`) 12 m square at 2 cm follows the walker in steps of
  the ground's 2 m cells, its prints kept where they lie (`Layer::move_to`).
- The walker's footfalls press it where the island's row draws sand: dry sand, 2.2 cm under a
  foot.
- It is drawn as the ground itself:
  - the tiles drop their fragments inside it (a ground window, through the cut-outs' raster, on
    both paths);
  - it is shaded by their layered row where it stands;
  - its field adds the ground's smooth slopes less its facets', so untouched it shades as the
    tiles do;
  - its mover has no motion of its own.
- The skin pass and its refit now run only on frames that bring joints or heights.

**Costs**
- About 0.10 ms a frame on the GPU: the window's triangles 0.07, `skin/blas` 0.022 on average,
  `movers/motion` 0.004.
- On the CPU, a step every 2 m costs 3.4–3.9 ms.
- A field costs 1.0–1.5 ms on each change.

**Checks**
- Tier 1 at ea2df6f plus the working tree (`captures/verify/20261008-200154-ea2df6f`): 429 tests,
  clippy, validation.
- Every image is 0 px apart from #71's flake and these:
  - The new `island-sand300` captures and their `-noocc` twins, on both paths. Mesh against
    fallback 0 px, occlusion off 0 px.
  - `fb-lab-yard240`, `600` and `1200`: the fallback path's yard images, which #194's Tier 0
    accept left old. Their ꟻLIP means, 0.0044, 0.023 and 0.018, are #194's mesh images' own. The
    run's comparison failed on them; taken again with them expected (`compare-expect.txt`), it
    passes.

**Not yet**
- A print casts no ray-traced shadow: the window's rays start as the terrain's do.
- Reflections show the ground's cut, not the window.
- Each change builds the field twice.
- The heights go up whole on each change, 4.3 MB.

**Found on the way:**
- #199, the owner's note on straight lines where areas meet.
- The grass near the camera looks soft with or without the window (#159).
