# #106: the ground in tiles (2026-10-01)

The island's ground cooked as 8 × 8 tiles of 2 km instead of one mesh, the first step towards
drawing it at 2 m (`docs/demos/island.md`, "The ground in tiles, towards 2 m").

- **Exact:** a tile is a window of the whole mesh, its vertices and normals the whole mesh's to
  the bit (`tiles_of_a_refined_heightfield_are_the_whole_mesh_to_the_bit`); one tile over the
  whole field draws the batch's island to the pixel.
- **Two faults the tiles showed, fixed:**
  - a seam at every border, from the layered ground's layers taking each instance's tint and
    texture place (`standard_surface` hashes the instance): a layered ground now shades as the
    first instance;
  - a dark blot in a far valley, a coarse level drawn under the rays' cut and shadowing itself:
    a terrain's shadow rays now also start twice the drawn cluster's error off.
    `tiles-blot.png`, a crop of the batch's island view: the previous commit, the tiles before
    the fix, the tiles now.
- **Cost:** the frame 0.09–0.18 ms shorter at 1440p (the cluster cull spread over 64 DAGs);
  `docs/PROFILE.md`, "The ground in tiles".
- **Checks:** the batch changes the island's views (the sea's, stones' and rocks' tints hash
  instance ids that moved by 63, the levels at the borders, the rays' cut; ꟻLIP mean 0.024 with
  the stand-in sea, 0.0055 with the water) and 71 pixels of the city's orbit (0.0003, the shadow
  rays' start); the A/B harness and mesh against fallback at 0 px; validation clean.

## Drawn at 2 m (`--island-drawn 2`)

The ground at 2 m on the field's cubic, carved by the channels, with the amplification's detail
faded out near the water (0.26 m root mean square where it is whole). 143 M triangles, cooked in
45 s on the first start, 4.2 GB of pages; the frame within 0.05 ms of the 8 m tiles' at 1440p.
`drawn-2m.png`, 8 m left and 2 m right: a hillside at a river's head, a steep valley's wall from
its water, the largest valley from 200 m. The look hardly changes. The default since the owner's
look the same day (`--island-drawn 8` for the 8 m tiles).
