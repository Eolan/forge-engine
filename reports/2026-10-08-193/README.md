# #193: road-tyre grooves for the car's treads

`physics-lab --lab yard --fixed-step`, tick 720. `mud.png` (`--view=3.6,1.3,1.9,0,-50`): down the
car's mud ruts; `sand.png` (`--view=3.6,1.6,-1.4,0,-55`): down its sand ruts. The tractor's chevron
lugs of #188 are gone; lines run down each rut where its tyres' grooves were.

- `deform::Tread` is an enum: `Lugs` (#188's chevrons, kept) and `Grooves`: `count` grooves
  `width` wide evenly across the tread, its ribs pressing `depth`, smooth sides so the beds' 2 cm
  grid takes them without a step.
- The car: three grooves 2.5 cm wide across its 20 cm, their middles 5 cm apart (two and a half of
  the beds' points: coarse enough not to shimmer), its ribs pressing 5 mm.
- Grooves run along the travel, so a spinning tyre leaves them too: #191's smear is gone.

**Checks** (the owner asked for no full run): fmt, clippy on forge-physics and city-blocks, their
tests (46 and 43); a new test: a road tyre leaves its grooves as lines down its rut, the same all
along it. The yard's images change; the next verify names them (`--expect '*lab-yard*'`).
