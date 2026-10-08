# #196: a walking player on the island

`city-blocks --island 7 --walker --walk 0,-1.5 --fixed-step`, frame 300 (`beach.png`): the
walker five seconds after it was put on the southern beach at (0, 5064), walking inland at
1.5 m/s on the sand, the camera 5 m behind its head.

- **Enter** on the island puts the lab's walking character (#139) on the ground under the camera
  and follows it: WASD, Shift, Space, the right mouse button turning the view. Enter again flies.
  `--walker X,Z` puts it somewhere from the first frame; a bare `--walker` puts it on that beach.
- **Its ground** is the drawn ground, refined cells included (`DrawnGround::surface_at`). Jolt
  holds it in 63 m tiles of samples a metre apart, at most four within 24 m of it, each laid a
  quarter turn about +y so Jolt's quads split along the drawn ground's diagonal. A test's rays
  meet the drawn ground within 1 cm (6 cm unturned). On the beach its feet stand within 0.1 cm
  of it.
- **Costs:** a tick 0.014 ms; the first two tiles cut in 0.57 ms. The capsule and visor are
  movers, parked under the island while nobody walks, and the movers' passes cost the island
  0.06 ms a frame even so (2.28–2.36 ms against 2.26–2.30). #198 is to skip them while the movers
  stand still.

**Checks:** Tier 0. Every image is 0 px apart from #71's flake, with the walker parked. 427 tests,
the walker's among them. Timings: `FORGE_SETS=island tools/timings.sh` against 4103a5f.

Found on the way: with `--no-rock-sites --movers N`, the barrel and the log were drawn as rocks
(the boulders' range left out one prop of three); fixed.

Next: #197, the beach's sand as a deformable window round the walker.
