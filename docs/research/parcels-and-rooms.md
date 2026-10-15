# Parcels and rooms: lots that fit the houses, houses thought from their rooms

*2026-10-16, for #266 (the first mission map) and the city sandbox's tracker, item 24. The owner,
on the kit's faces map of a block: "sometimes architects have fun with the designs, but here we
have non-rectangle shaped buildings for no reason, or spaces between them that don't make
sense"; and the principle to design from: "a house or structure should be thought by thinking
about the rooms, the real space inside and the minimum required to make it a liveable space,
depending on how rich it is."*

## 1. What the city does today, and what the owner saw

- **Blocks** are the road graph's faces; **lots** are cut inside the building lines
  (`Block::buildable`, `geom::inset_by_collapsing`), each a strip from the street towards the
  block's middle, the slivers collapsed into their neighbours. The streets come from a tensor
  field, so the block edges curve and meet at odd angles, and the lots cut across them are
  trapezoids and wedges: a lot tapering to a point between two rows, a triangle of ground
  between a building's wall and the street, a notch where two cuts did not meet.
- **Masses** fill their lots to the lot lines on the party sides and to the building line on
  the street side, so a lot's odd shape is the building's: a non-rectangular house "for no
  reason". Where two lots' cuts leave a thin wedge, two blank walls face a slit.
- **Walls** get their faces from what lies outside them (`facades::edge_kinds`); the rooms come
  after, laid into whatever footprint the lot gave (`city::units`). The house is thought from
  the outside in.

The owner's later thought, that thick walls and real junctions between the structures (D-060)
would make the issue disappear, holds for the slits and wedges between adjoining buildings:
two structures sharing one party wall of real thickness have no gap. It does not hold for the
lots' shapes, which come from the subdivision upstream and stay whatever the walls are made of.
So the answer is both: the junctions from D-060, and a parcel rule.

## 2. Parcel subdivision

**The reference method** is Vanegas, Kelly, Weber, Halatsch, Aliaga and Müller, *Procedural
Generation of Parcels in Urban Modeling*, Computer Graphics Forum 31(2), Eurographics 2012
(DOI 10.1111/j.1467-8659.2012.03047.x; the PDF is at Purdue's CGV lab). Two ways to cut a
block, both with attributes a user sets or the method reads off real cities:

- **OBB subdivision:** cut the block (then each piece) across the shorter side of its oriented
  bounding box, recursively, until the pieces reach the wanted area, with a check that every
  parcel keeps a street side (a piece without one is merged back or cut the other way). It
  gives rectangular lots perpendicular to the street on straight edges and keeps them nearly
  rectangular on curved ones, since every cut runs across the piece, not along the block's
  outline.
- **Skeleton subdivision:** the straight skeleton of the block gives a strip of constant depth
  along each street side, cut perpendicular to the street at the wanted widths; the corners
  are resolved by the skeleton's bisectors. Tom Kelly's thesis (*Unwritten procedural
  modeling with the straight skeleton*, Glasgow, 2013) is the fuller treatment and the first
  use of the skeleton for parcels. The strips are what a European perimeter block looks like
  in a cadastre: lots of one depth, party walls perpendicular to the street, the court behind.

Both methods are published, with no patent found on them (§6). The paper reports parcels that
"closely resemble those found in real-world cities of a large variety of styles" and
"functionally and geometrically plausible for subsequent building modeling", which is the
owner's complaint in one sentence.

**What a real cadastre does** that the tensor-field lots do not: the party walls stand
perpendicular to the street (so adjoining houses share one straight wall); a lot's width comes
from its type (a village house 4–7 m, a town house 6–10 m, a perimeter-block building 12–25 m,
a villa's lot 15–30 m; estimates, see §6); a lot's depth from the block (a strip of 12–20 m in a
perimeter block, the whole half-block in a village); a lot too narrow for its type is merged
with its neighbour, never built as a sliver; corners get the corner lot, wider, which takes the
odd angle so the lots along the sides stay rectangular. Curved streets are met by fanning the
party walls (each perpendicular to the local street tangent), so the lots are gentle
trapezoids, the buildings on them rectangular to the street, and the fan absorbed in the
court or the party-wall thickness.

## 3. Joined buildings, the owner's definitions

- **Semi-detached house:** one building sharing one common wall with another (a *maison
  mitoyenne*).
- **Terraced house (UK), townhouse (US):** a row of identical or mirror-image houses joined
  face to face in a continuous line.
- **Row house (North America):** the same at medium density.
- **Attached or adjoining buildings:** the general terms for any structures sharing a common
  wall.

For the generator, the type decides the lots and the walls together:

| type | lot width | shared walls | the party wall | where |
|---|---|---|---|---|
| detached (villa, farm) | 15–30 m | none | none; setbacks all round | garden districts, the edges |
| semi-detached | 8–12 m, in pairs | one | one straight wall on the pair's lot line, the houses mirrored | suburbs, new quarters |
| terraced, row | 4–8 m | two (one at a row's end) | every lot line, the row's roofs on one ridge | old towns, villages, workers' rows |
| attached (perimeter block) | 12–25 m | two | the lot lines, each building its own height and roof | the centre's blocks |

A shared wall is one structural wall, D-060's party wall at its real thickness (an old rubble
wall 45–80 cm, a modern one 20–25 cm, `research/building-structure.md`), owned by the pair and
built once, with the junctions of the floors on both sides into it. That is what removes the
slits: not two walls a hand apart, but one wall two houses lean on.

## 4. Rooms first

**The owner's principle** reverses the pipeline: the household and its means give the
programme (which rooms, how big), the programme gives the plan, the plan gives the footprint and
the storeys, and the lot is cut to fit the house. The literature has exactly this pipeline:

- Merrell, Schkufza and Koltun, *Computer-Generated Residential Building Layouts*, ACM
  Transactions on Graphics 29(6), SIGGRAPH Asia 2010: from high-level requirements (the number
  of bedrooms, the floor area, the number of storeys), a Bayesian network trained on 120 real
  architectural programmes gives the rooms, their sizes and adjacencies; stochastic
  optimisation (a Metropolis walk over the plan) lays them out as floor plans "with varying
  dimensions", that is, the outline follows the rooms; the plans become a complete 3D house
  with its internal structure. The authors say the approach follows the layout process
  architects use.
- The village house is the same thing by hand: one or two rooms a storey on a plot a room
  wide, the stair in the depth, the kitchen and the workshop or stable at the ground, the
  living room above, the bedrooms above that; the DRAC PACA's typology sheet classes the
  pre-1914 town and village house by its urban quality, "gabarit" and continuity, modest
  mouldings, and the inertia its party walls give.

**The minimums** (France, from the decency decree 2002-120 of 30 January 2002 and the housing
aid rules):

| | |
|---|---|
| the main room | 9 m² and 2.20 m under the ceiling, or 20 m³ |
| a room let on its own in a shared flat | 9 m² and 20 m³ |
| two people (for housing aid) | 16 m² habitable, then 9 m² each more person |
| the average dwelling (INSEE, 2013) | 90.9 m²: 112.2 m² in a house, 63.0 m² in a flat; 92.5 m² in 2024 |
| rooms a person (INSEE, 2013, already in the city) | 1.5 in flats, about 1.9 a flat |

**By wealth**, the programme grows in rooms before it grows in room size, then in both:

| means | the programme | room sizes |
|---|---|---|
| poor | one or two rooms a household, the kitchen in the living room, a shared or tiny bathroom | at the minimums: 9–12 m² |
| modest | a living room, a kitchen, one bedroom a couple and one a child or two | 10–14 m² bedrooms, a 16–20 m² living room |
| comfortable | a bedroom a person, a dining room or a kitchen one eats in, a study, two bathrooms | 12–16 m², a 25–35 m² living room |
| rich | reception rooms, suites with their own bathrooms, service rooms, a courtyard or a terrace, a garage or a stable | 20 m² and up, a 40–60 m² salon |

The sandbox already has the households with their sizes (forge-sim's population, D-060's
units) and the districts' kinds; wealth is one more number a district or a lot gives a
household.

## 5. Recommendation for Forge

1. **Cut the lots with the straight skeleton** along each street side, at widths drawn from
   the district's building type (§3), party walls perpendicular to the local street tangent,
   corner lots wider, any strip narrower than the type's minimum merged with its neighbour, and
   the block's middle left as the court. OBB subdivision for the free-standing types
   (villas, farms, the industrial lots). Keep `Block::buildable` for the building line. A check
   map draws every lot, its width and its angle to the street; a test asserts no lot narrower
   than its type's minimum and no interior angle under 60°.
2. **Houses from their rooms:** a programme per household (its size and means give the room
   list and sizes from §4's tables), a plan solver inside the lot's width (rooms as rectangles
   along a spine, the stair and the wet rooms in the depth, a Metropolis pass to fit the
   lot's depth and the storeys), the footprint and the storeys from the plan, the facade from
   the rooms (a window a room, the entrance where the hall is). D-060's structures take the
   plan as their units instead of cutting units into a given footprint; `city::units` stays
   for the blocks' flats until the solver covers them.
3. **The joined types as rules** per district in the style bible: which type, the widths, the
   shared walls, mirrored or repeated; the row's roofs on one ridge (the city's `pitches`
   already does this where the neighbour is as tall).
4. **One party wall for two houses** (D-060's junctions): built once at its thickness, both
   structures' floors bearing on it; the slits and wedges go with it.
5. **The first map** (#266) lays out its village this way from the start: a few streets, the
   skeleton lots, 20–40 houses from their rooms, the rich house on the square; the city takes
   the rule back block by block.

## 6. Checked and left out

- **Patents.** A search for an Esri or Procedural Inc. patent on parcel subdivision or CGA
  shape grammars found none; the Vanegas paper and Kelly's thesis are published methods. Not
  checked on Google Patents directly: do so before a sold product leans on OBB subdivision
  with the paper's road-side rule, and on Merrell's pipeline (Stanford, 2010; no patent known).
- **The village house's dimensions** (4–7 m wide, 8–14 m deep, two or three storeys, one or two
  rooms a storey) are the author's estimates from the type; the sources found describe the
  type (narrow, tall, continuous, built in height for want of ground) without numbers. The
  regional inventory (Inventaire général, PACA) would give measured plans.
- **INSEE's areas by number of rooms** for 2013 sit in the tables of *Le confort des
  résidences principales en 2013* (CONF_T3, by rooms, area and occupants); the figures were not
  retrieved here. The averages above are from the 2017 and 2024 editions.
- **The 1.80 m ceiling** some pages give as a 2023 amendment to the decency decree was found on
  one site only and is left out.

## Sources

- Vanegas, Kelly, Weber, Halatsch, Aliaga, Müller, *Procedural Generation of Parcels in Urban
  Modeling*, CGF 31(2), 2012: https://www.cs.purdue.edu/cgvlab/papers/aliaga/eg2012.pdf and
  https://diglib.eg.org/handle/10.1111/v31i2pp681-690
- Kelly, *Unwritten procedural modeling with the straight skeleton*, thesis, Glasgow:
  https://theses.gla.ac.uk/4975
- Merrell, Schkufza, Koltun, *Computer-Generated Residential Building Layouts*, SIGGRAPH Asia
  2010: https://www.cs.princeton.edu/courses/archive/spring11/cos598A/pdfs/Merrell10a.pdf
- The decency minimums: https://www.justice.fr/fiche/hauteur-surface-minimum-logement-louer and
  https://edito.seloger.com/conseils-d-experts/reglementations/qu-qu-un-logement-decent-article-20487.html
- INSEE, *Les conditions de logement en France*, 2017 edition (2013 data), and the 2024 figures:
  https://www.insee.fr/fr/statistiques/1912705 and https://www.insee.fr/fr/statistiques/8727513
- DRAC PACA, *Les typologies architecturales en PACA*:
  https://www.culture.gouv.fr/Media/Regions/Drac-Paca/Files/Politique-culturelle/Transition-ecologique/Performance-thermique/Les-typologies-architecturales-en-PACA
- The owner's definitions of joined buildings, 2026-10-15.
