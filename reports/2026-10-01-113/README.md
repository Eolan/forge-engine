# #113: the water far up a valley from a low view (2026-10-01)

`up-valley.png`: the demo's logged view `up a steep river from low` (`--view
-1112,80.1,-3083,-97,4`, 2 m over a river falling about 20 %), frame 120, before (left, da572f7)
and with the fix (right). Before, the water showed only near the camera, the bed dry beyond; it
appeared as the camera rose (from 25 m the whole stream showed). With every page resident it was
the same, so not the streaming.

The cause: the fresh water's contact fade took its depth along the view ray under a level
surface (`path × v.y`). Looking up a steep river the water ahead stands over the camera, `v.y`
is negative there, so the depth was zero and the water discarded. The depth is now taken under
the surface as it lies, tilted down the river by its fall (`FreshWater::tilt` in
`shaders/water.slang`).

A/B at frame 60 against da572f7: this view ꟻLIP mean 0.015; down a lowland river 0.0004; the
largest mouth from 4 m 0.008 (a thin line at the water's edges); the lake and the first view
under 0.00002.
