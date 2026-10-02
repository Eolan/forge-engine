# #96 step 3: the island demo of its own (2026-10-02)

`cargo run --release -p island`: Phase 2's demo, with the island (seed 7) as its scene and its
own window. It shares `city-blocks`' renderer, which is now a library. `city-blocks --island 7`
draws the same island, to the pixel.

**`shots.png`, the golden shots** (`island --shot NAME`), frame 60 on the mesh path. Each is
found in the island's features, so it holds for any seed:
- **`mouth`, dawn** (`--time-of-day 0.08`): the largest mouth with bars (#127), 140 m up the
  river from them and 60 m up, looking down the river into the sunrise over the sea.
- **`lake`, morning** (0.3): the highest of the three largest lakes, 30 m over its water past
  its south edge.
- **`island`, afternoon** (0.7): the whole island from 1.75 km off its southern beach, 300 m up.
- **`valley`, dusk** (0.92): 40 m below the steepest point of a river 5 m wide or more, 2 m up,
  looking up its steps and pools into the sunset.

**`tour.png`, the tour** (`island --tour`), at 5, 12, 20, 28, 36, 45, 55 and 66 s. It lasts 70 s:
- out of the steep valley;
- over the hills to the lake;
- across the plain and its rivers to the mouth with bars;
- out over the sea, then round to the island.

It rests 2.5 s at each shot. In between it keeps 30 m over the highest ground within 60 m, the
lift smoothed so it rises before a hill. The path is precomputed, so a frame depends only on
its time.

**What changed:**
- **`demos/city-blocks`:** a library and a thin binary. `city_blocks::main_island` runs the same
  demo with `--island 7` by default, titled "forge island".
- **`--time-of-day T`:** the sun held where `--day` has it, the exposure metered from the
  scene.
- **`--shot NAME`:** a shot's view and time, unless `--view` or `--time-of-day` is given. The log
  lists the shots (`the island's golden shots (--shot)`).
- **`--tour`:** the flight above (`demos/city-blocks/src/island_demo.rs`).

**Numbers:**

| The tour, whole (4 200 frames at a fixed step) | GPU per frame | Frame p50 / p99 |
|---|---|---|
| 1600 × 900 | 1.43 ms | 1.43 / 2.10 ms |
| 2560 × 1440 | 2.72–2.74 ms | 2.68 / 3.67 ms |

The worst frame, 49 ms, is a single one: no second's p99 passes 3.93 ms. The island's still
views have one like it (41 ms).

**Checks:**
- The capture batch has 58 images, the four shots on both paths being new. The 50 before are
  at 0 px against the batch before, and the A/B pairs at 0 px. The new shots are 0 px between
  the mesh path and the fallback.
- `validate.sh` is clean, with the tour's first 10 s at a metered time of day.
- `timings.sh` is within noise (the island 1.591–1.633 ms against 1.586–1.643).
- 255 tests pass, one new: the tour rests at its shots, looking their way, and clears a 200 m
  ridge between them by its clearance. Clippy and fmt pass.

**Seen in the shots, left for later:**
- **The dawn mouth:** a dark blue band runs along the right bank. It is the bank's shadow on the
  water:
  - the river stands about 1 m under the coastal plain there, and its bank climbs that metre
    over 2–4 m;
  - with the sun 6° up, that metre throws a shadow about 9.5 m long across the water.

  A first note blamed a beach's berm (#127); measured across the river, there is none. Lower
  or gentler banks on the coastal plain would soften it, if wanted.
- **The tour's plainer moments:** climbing out of the valley (5 s), and the sea alone as it
  turns at 55 s.
- **The planet variant** (orbit-to-ground): the demo's second step, not started.
