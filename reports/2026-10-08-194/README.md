# #194: the dogs' beds as ground

`physics-lab --lab yard --fixed-step --view=2.0,0.6,2.7,45,-20`. `snow.png`: tick 620, a dog
walking its snow, its paws sunk into the prints it leaves, a trail of them behind it.
`stepping.png`: tick 560, a dog stepping from the floor onto the snow's untouched top.

- **On the beds' top:** the dogs' IK plants each paw on the bed under it (`yard::top`: its
  thickness), and their balance holds the torso over the paws' mean, so a dog steps onto the
  untouched material and sinks into the print its footfall presses (to the floor in the snow).
- **Footfalls on it:** a paw comes down within 2 cm of that top, and is up again once 4 cm over
  the floor under it. Judged up by the top too, a paw standing in its print, below the top beside
  it, stayed down across a bed and came down at the beds' edges alone. The dogs now make 38, 51
  and 70 footfalls in 26 s in their sand, mud and snow (16, 16 and 18 on the floor before, which
  missed most steps).
- **Not Jolt's:** the beds as Jolt height fields felt by the paws, at the point every centimetre a
  3 by 4 cm pad needs, made a tick 1.08 ms (0.27). And they broke replays: Jolt keeps a field's
  heights quantized in blocks and re-encodes the blocks beside a region it sets from what they
  held, so a field set whole on a restore was not the one the run set region by region, and the
  dogs standing on it went their own way. Setting every change whole and notifying every field
  each tick made it exact, at 0.2 ms more. A field whose blocks were not a power of two each way
  also had blocks padding Jolt's tree of ranges that a paw beside the field met (an assert).
  The car's grounds stay Jolt's (#187): its ray wheels have no contacts to keep.
- **A reset** puts the dogs back where they start, in their snow, so its own tick prints there; the
  lab's test checks the car's beds untouched and the dogs' holding a tenth of their prints at most.

**Costs:** a yard tick 0.273–0.279 ms against 0.267–0.269 for f89951d (the extra footfalls).

**Tests:** the yard's walk (the dogs print every bed, a dozen footfalls each and more), the lab's
save mid-spin, reset and replay; the crate's 43.

Left: the beds felt by the physics (a dog let go falls through to the floor). Found on the way:
#195, after a reset the car no longer stops in its sand.
