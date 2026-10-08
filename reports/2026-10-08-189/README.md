# #189: the car's mud berms

`physics-lab --lab yard --fixed-step`. After #187 the berms beside the car's ruts through its deep
mud stood as walls 12 cm over the mud, ridged across their flanks a step apart (2.4 mm a point,
striped under the yard's low sun). Softening them first stalled the car in the mud, so the
crossing was made robust before the berms were touched.

`berms.png`: tick 600, along the car's mud ruts: rounded berms 2 to 3 cm over the mud.

- **The stall:** a wheel's press followed its own round, so a wheel that slowed to a crawl rested
  in a trough of its own shape. It touched the trough's front wall, 18 cm ahead with its normal
  tilted 35° back, and the suspension pushed the car back from it. The press now reaches at
  least 4 cm ahead and its round is 30 % wider than the wheel's. The car crosses at the same
  throttle (at most 0.6) and grip, slowing to 1.3 m/s.
- **The berms:**
  - **Water squeezed out:** the deep mud loses half of what a tyre pushes out as water.
  - **Wider rim:** it spreads three half widths beside the rut.
  - **Blended steps:** each step's rim fades past its stretch's ends, so the steps blend.
  - **Smoothing:** a wheel's press smooths its box (three passes moving an eighth of each
    difference between neighbours, the print itself left as pressed) before the slump. Smoothed
    after it, the ground beside the print stood steeper than the material holds. The dogs' prints
    are not smoothed: their rims are of one press.
- **The numbers:**
  - **Crests:** 2 to 3 cm over the mud, stepping 0.85 mm a point.
  - **Flanks:** 0.3 to 0.65 mm a point (2.4 mm before).
  - **Cost:** a tick 0.271 ms over 1 200 (0.236 before), p99 0.95.

**Tests:** the yard's walk asserts the berms low (under 5 cm over the mud) and even (crests under
2 mm a point, flanks under 1 mm), and that the car crosses; the lab's save, reset and replay.

Left: the treads (#188), on top of these berms.

**Tier 0** (`captures/verify/20261008-140315-48249b3`, 421 tests): the yard's three images changed
as meant (FLIP mean 0.07 to 0.09), everything else 0 px but the #71 flake.
