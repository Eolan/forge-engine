# #220, 2026-10-10: the haze and faster new tiles

- `tone-question.png`: the tour's Èze stop (top) and Mont Blanc stop (bottom). On the left, the
  look as it is (AgX, EV 15, the haze at half the air's density); on the right, AgX's punchy look
  one stop brighter (`--tonemap agx-punchy --ev100 14`, or G in the demo). For the owner: AgX stays
  the default by D-045 unless they choose otherwise.
- `dag-template.patch`: a DAG template shared by every tile (`docs/demos/planet.md`, "Left for
  later"), measured and set aside: it cooked a tile's DAG in 23 ms against 0.16 s, but its errors
  were 7–10 times a tile's own. Kept for a later try at a template shown first, with the tile's own
  DAG cooked behind it.
