# Village houses: footprints, terraces, rooms and windows

The research behind the first map's houses (#266, D-062): what a Mediterranean village's
houses and flats are made of, as rules the generator can follow and the anomaly checker can
test. The owner's walk of 2026-10-16 asked for it ("I think you lack a bit of data about house /
apartment layouts"): footprints stretched out, windows that matched no room, partitions on a
window's edge, houses side by side each under its own hip, slivers and leftover yards walled
round. Desk research of the same evening (a remote research agent). Labels follow the house
convention: **[paper] [web] [docs] [standard] [dataset]**; sources marked **R** were read,
**S** seen in search results only. Several French and Italian PDFs could not be read as text
and are S. Statements marked *(synthesis)* are design judgement built on the cited rules, not
sourced facts. Local plans (PLU, regolamento edilizio, ordenanzas) override the national
defaults quoted here; none of this is legal advice.

> **In five sentences.** Village row houses are 4–7 m wide and 8–15 m deep on lots 15–30 m
> deep, because floor joists span from party wall to party wall (one bay ≤ about 6 m): houses
> grow in bays, not by stretching. Daylight limits depth: a room lit from one side works to
> about 2–2.5 times its window head (5–6 m), so a house is one room deep or two rooms deep with
> windows front and back (10–12 m). A row has one roof per run of equal height, its ridge along
> the row, party walls never carry windows, and two buildings on neighbouring lots touch or
> stand 3 m apart (Italy's Civil Code), so walled slivers do not exist. Every bedroom, living
> room and (in Italy and Spain) kitchen must have an openable window (Italy ≥ 1/8 of the floor,
> Spain ≥ 1/20 openable, France ≥ 1/6 of the habitable area for a new house); bathrooms and WCs
> may be windowless with extraction. Generate the facade's bays first and put the partitions on
> the piers between them, not the other way round.

**Contents**

1. [Footprints](#1-footprints)
2. [Terraces, party walls and roofs](#2-terraces-party-walls-and-roofs)
3. [Rooms and windows](#3-rooms-and-windows)
4. [Room sizes and layouts](#4-room-sizes-and-layouts)
5. [Gardens and boundaries](#5-gardens-and-boundaries)
6. [Data and published methods](#6-data-and-published-methods)
7. [Patents](#7-patents)
8. [Recommendation for Forge: rules for the generator](#8-recommendation-for-forge-rules-for-the-generator)
9. [Sources](#sources)

---

## 1. Footprints

### 1.1 Terraced village houses

- **France.** The Provençal *maison de village*: a rectangular plan, a street facade, one or
  more party walls, on a small plot it "generally occupies entirely"; some stand 1–5 m back
  behind a small front space [1]. Old cores' PLU zones (UA) build on the street line in
  continuous order [12]; one PLU allows building on the side boundaries only within 15 m of the
  street line [13].
- **Italy.** Medieval lots narrow and deep, square to the street, the unbuilt back a kitchen
  garden (*orto*) (Cagliari, Castello) [2]. Teaching material classes row-house fronts as narrow
  (3.00–5.40 m), medium (5.40–7.20) and wide (> 7.20), sides about 1:2 [3].
- **Spain** (houses *entre medianeras*): Girona 4.5 m wide and over 30 m deep [4]; Pamplona a
  4.6 m bay and 18 m deep [5]; Sant Cugat under 5 m [6]; Seville's bays about 3 m, the timbers'
  length [7]. Ordinances: a 6 m least front in a *casco antiguo* [8]; Murcia's closed block 15 m
  deep at most, lots of 150 m² and 7 m front at least to split [9]; Alcúdia 30 m deep on the
  ground floor and 20 m above [10]; Sanxenxo caps merged fronts at 12 m to keep the street's
  rhythm [11].
- **Why narrow.** Joists span between the party walls: BS 8103-3 assumes timber spans of 6.0 m
  at most [14], span tables about 5.15 m for 220 × 75 mm joists [15]. Wider than about 6 m, a
  house needs a second bay and a spine wall or beam *(synthesis)*.

| Type *(synthesis from [1]–[15])* | Front | Built depth | Lot depth | Width : depth |
|---|---|---|---|---|
| Mid-terrace, one bay | 3.5–6.0 m | 8–12 m | 12–30 m | 1:1.5–1:3 |
| Mid-terrace, two bays | 7–12 m | 9–15 m | 15–30 m | 1:1–1:2 |
| End of terrace | as mid, or 1–2 m more | as mid | as mid | as mid |
| Corner house | 5–9 m on each street | ≤ 12 m | a small yard or none | about 1:1, or an L |

### 1.2 Depth from daylight

- Side windows light usefully to about 2–2.5 times their head's height [16][17]; windows on
  opposite walls about double it [18].
- BRE's limiting depth (from BS 8206-2, withdrawn but still quoted): a room lit from one wall is
  gloomy at the back unless L/W + L/H ≤ 2/(1 − Rb), with L its depth, W its width, H the window
  head's height and Rb the back half's reflectance [19]. W = 4.0 m, H = 2.3 m, Rb = 0.5 give
  L ≤ 5.8 m; Rb = 0.4, L ≤ 4.9 m.
- The UK annex to BS EN 17037: bedrooms 100 lx, living rooms 150, kitchens 200, over half the
  reference plane for half the daylight hours [20].
- *(Synthesis)* a single-aspect habitable room ≤ about 6 m deep; a dual-aspect house two rooms
  deep, 10–12 m, the stair, bathroom and storage in the dark middle band; deeper than 12–15 m
  needs a court or a light well (the 15 m caps of [9][13]).

### 1.3 Detached and semi-detached

- The Provençal *mas* faces south, no openings to the north (the mistral's side), few openings
  for its walls [21][22]; it grows by additions, a U round a yard for one [23].
- *(Synthesis)* a detached house one bay deep (5–6 m) or two (9–12 m), 1.5–3 times as long for a
  *mas*, 1:1–1:1.6 for a modern villa; a semi-detached pair one symmetrical volume of 2 × 6–8 m
  by 9–11 m, the party wall on its axis under one roof.

## 2. Terraces, party walls and roofs

- **Roofs.** The Var: one or two symmetric slopes at 27–33 % (15–18°) in round clay tiles, the
  eaves' *génoise* rows growing with the facade's height, the verges matching the neighbours'
  [24]. Le Pouget's UA ≤ 30 % [25]; Iglesias 30–33 % [26]; Piedmont 30–40 %, every slope alike
  [27]. Ridges parallel or square to the street in one plan [30]; along gutter-fronted rows,
  parallel *(synthesis)*.
- **Party walls.** French Civil Code art. 675: no window or opening of any kind, "even with
  fixed glass", in a party wall without the neighbour's consent [28]. Parapets rising over the
  roofline are London's fire practice (1707, 1774) [29], not Provence's or Italy's, where the
  tiles run across the party walls *(synthesis)*.
- **Gaps.** Italian Civil Code art. 873: buildings on neighbouring lots "united or adjoining",
  or 3 m apart at least [31]: no gap or a real one, never a 0.5–2 m sliver.
- **Height steps.** Murcia ties storeys to the street's width: under 4 m two storeys (7 m),
  4–8 m three (10 m), 8 m or more four (13 m) [9]. Where neighbours differ, each keeps its own
  roof, the step exactly on the party wall, the taller's wall above the lower roof blank (art.
  675) under a tiled verge; eaves differing by 10–40 cm read as errors: equal, or 0.8 m apart at
  least *(synthesis)*.
- **Ends and corners** *(synthesis)*. An end-of-terrace house has a gable on its free side,
  windows in it where it faces a street or its own garden. Hips only on detached houses and on
  some corners. At a corner one row's roof runs through and the other meets it in a valley, or
  the corner house turns the roof, a hip outside and a valley inside; the corner house has two
  street fronts and little or no yard.
- **Row length.** Murcia: new blocks with three fronts of 50 m or more round a common court [9].
  Village runs of 3–12 houses (20–60 m), broken by alleys or passages *(synthesis)*.
- **Roof terraces.** The Provençal *tropézienne*, cut into the slope "in very dense blocks, for
  narrow and deep buildings", keeping two thirds of the slope and 2.5 m of tiles over the eaves,
  served by a habitable room [32].

## 3. Rooms and windows

### 3.1 The codes

| Country | Rule |
|---|---|
| France | Main rooms need "sufficient natural light" and an opening onto open air; the WC apart from the kitchen and the room where meals are taken; one main room of 9 m² and 2.20 m at least, or 20 m³ [37]. Kitchens, washrooms, WCs and corridors are service rooms [81]. |
| France | Ventilation (1982 order): air in through the main rooms, out through kitchens, bathrooms and WCs, so wet rooms may be windowless [40]. |
| France | New houses (RT2012 art. 20, reportedly kept in RE2020): openings ≥ 1/6 of the habitable area, measured in the wall's opening; entrance doors and openings under 0.5 m² do not count [38][39]. |
| France | No clear view within 1.90 m straight or 0.60 m oblique of a neighbour's land (art. 678–679) [42]; fixed obscured *jours* allowed with a sill ≥ 2.6 m on the ground floor, ≥ 1.9 m above (art. 677) [41]. |
| Italy | Habitable rooms 2.70 m high, service rooms 2.40 [35]. Every room but bathrooms, corridors, stairs and stores direct daylight, a mean daylight factor ≥ 2 %, an openable window ≥ 1/8 of the floor; bedrooms, living room and kitchen an openable window [35]. A bathroom an outside opening or mechanical extraction [35]. The 1/8 survived 2024's "Salva Casa" [36]. |
| Italy | Clear views 1.5 m from the boundary (art. 905); light-only *luci* with a fixed grating and a sill ≥ 2.5 m on the ground floor, ≥ 2 m above (art. 901) [43]. |
| Spain (CTE DB-HS3) | Kitchens, dining rooms, bedrooms and living rooms an openable exterior window or door ≥ 1/20 of the floor; air in through the dry rooms, out through the wet ones; bathrooms not on the list [44]. |
| Spain (regions) | Valencia: natural ventilation but for circulation, storage and hygiene rooms [45]. Catalonia: 1/8 of the floor (a secondary source), 2.50 m high, 2.20 for wet rooms and circulation [46]. Clear views 2 m straight, 0.60 oblique (Civil Code art. 582) [47]. |
| UK | Purge ventilation: openable ≥ 1/20 of the floor (windows opening over 30°), 1/10 for 15–30° [48]. Approved Document O caps glazing near 11–18 % of the floor per facade, 26–30 % in the most glazed room; sources disagree on the table [49]. |

### 3.2 By room *(synthesis; to calibrate on Swiss Dwellings, §6)*

| Room | P(window): house / flat | Count | Typical opening (w × h) | Notes |
|---|---|---|---|---|
| Living room | 1.0 / 1.0 | 2 (1–3) | 0.9–1.2 × 1.35–1.6; French window 0.9–1.4 × 2.15 | to the garden or the best side; luxury: one or two more French windows, a terrace |
| Kitchen (closed) | 1.0 / 0.9 | 1 | 0.8–1.2 × 1.0–1.35, sill 1.05–1.2 m | over the sink; an open kitchen takes the living room's |
| Bedroom | 1.0 / 1.0 | 1; master 1–2 | 0.8–1.0 × 1.2–1.5 | luxury master: a French window onto a balcony |
| Bathroom | 0.6 / 0.3 | 0–1 | 0.4–0.6 × 0.6–0.9, sill ≥ 1.4 m, obscured | else an extractor |
| WC | 0.4 / 0.15 | 0–1 | 0.3–0.5 × 0.4–0.6, sill ≥ 1.6 m, obscured | often under the stair, windowless |
| Hall, corridor | 0.15 / 0.05 | 0–1 | a fanlight over the door, or small | |
| Stair (house) | 0.5 | 0–1 a half-landing | 0.5–0.8 × 0.8–1.2 | its sill half a storey off the floors' |
| Store, cellar | 0.2 | a vent | 0.3 × 0.3 | |

Openings ≥ 1/8 of each habitable room's floor: a 12 m² bedroom needs 1.5 m² (one 1.0 × 1.5
window), a 20 m² living room 2.5 m² (two 0.9 × 1.4); a new 90 m² house 15 m² in all (France's
1/6).

### 3.3 The facade

- Old windows taller than wide, 1.5:1 to 2:1 (90 × 135, 100 × 150 cm), stacked on vertical axes,
  lintels and sills aligned a floor, often shorter a floor up [33]. Regular composition is
  mainly 17th–18th century; earlier facades are irregular [34], so a few irregular ones fit.
- The *trumeau*, the pier between two openings [52], is where a partition belongs.
- *(Synthesis)* modest houses: the windows the code asks for, one or two bays a floor, wet rooms
  windowless or with a small high window. Luxury: French windows and a terrace to the garden, a
  balcony off the master bedroom, a *tropézienne* [32], a loggia, a bay or two more; no room over
  about 30 % glazing to floor [49]; a detached house's north side 0–2 small windows [21].

## 4. Room sizes and layouts

| Room | Code minimum | Generator range *(synthesis)* | Least width | Long : short at most |
|---|---|---|---|---|
| Single bedroom | IT 9 m² [35]; UK 7.5 m², 2.15 m wide [50] | 9–12 m² | 2.4 m | 1.6 |
| Double bedroom | IT 14 m² [35]; UK 11.5 m², the first 2.75 m wide, others 2.55 [50] | 12–18 m² | 2.75 m | 1.6 |
| Living room | IT 14 m² [35]; London: living, kitchen and dining 27 m² for 4, 29 for 5, a sitting area 2.8 m wide for 2–3, 3.2 for 4+ [51] | 16–30 m² | 3.2 m | 2.0 |
| Kitchen (apart) | — | 6–12 m² | 1.8 m (galley) | 2.5 |
| Bathroom | IT one at least with WC, bidet, bath or shower, basin [35] | 4–7 m² | 1.6 m | 2.0 |
| WC | FR apart from the kitchen and dining room [37] | 1.2–2.0 m² | 0.85 m | 2.2 |
| Corridor | — | — | 1.0–1.2 m; 1.2 with 1.5 m turning circles wheelchair-first | — |

- **Dwellings.** Italy: 14 m² a person for the first four, 10 for each more; a studio 28 m² for
  one, 38 for two [35] (Salva Casa's 20 and 28 m² for existing buildings only [36]). UK gross
  areas: 1 bedroom for 2, 50 m²; 2 bedrooms for 4, 70 m² on one storey, 79 on two; 3 bedrooms for
  5, 86 / 93 / 99 m² on one / two / three; 2.3 m ceilings over 75 % [50]. Catalonia ≥ 36 m², a
  bedroom ≥ 10 m² when there are two or more [46].
- **A terraced house 5 × 10 m on 2–3 storeys** *(synthesis)*: a straight flight along a party
  wall at mid-depth (about 0.9 × 4.3–4.6 m for a 2.9 m storey, 17 risers, 27 cm treads) splits
  the plan into a front and a back room. Ground floor: the street door, the front room a kitchen,
  workshop or store, the back a living room onto the garden, the WC under the stair. Upstairs:
  bedrooms front and back, the bathroom in the middle band by the stair. Top: a bedroom, an attic
  or a roof terrace. Provençal variant: the ground floor a store, the living rooms upstairs.
  Two bays (8–12 m): a central hall and stair, rooms each side.
- **A small flat, 55–70 m²** *(synthesis)*: living and kitchen on the best side, bedrooms
  opposite (dual aspect) or on the same side (single aspect, ≤ 6 m deep); bathroom and WC inside
  by the stair; corridors ≤ 5 m.
- **Windows and partitions** *(synthesis)*: the facade's bays first, then the partitions on the
  piers (WallPlan, 2022, also makes windows before walls [73]); a room with one window has it
  centred on its stretch of facade (± 0.2 m), or spans whole bays; with hinged shutters each leaf
  folds back onto the wall, so a pier is one window wide at least, an end pier half a window.

## 5. Gardens and boundaries

- **France.** With no local rule, a dividing wall a neighbour can require is 2.60 m high at least
  (towns under 50 000) or 3.20 m (larger), no national maximum; 2 m or more needs a declaration
  [53]. PLUs often cap fences at 1.80 m, a 0.60 m low wall under railings on the street [55].
  Plants over 2 m stand 2 m from the line, lower ones 0.5 m; trained on a shared wall, no higher
  than its top [54]. No general duty to fence a plot was found [53].
- **Italy.** Hedges up to 2.5 m: 0.5 m from the boundary; small trees 1.5 m, tall ones 3 m; no
  distance behind a dividing wall for plants below its top [56]. Fences on public space up to
  2.0 m, at most half solid (Bogogno), or open above 0.5 m (Lozzolo) [57].
- **Spain.** Ordinances make owners fence empty urban plots on the street line: Ávila ≥ 2 m of
  block, Aldea del Rey ≥ 2.80 m of brick or block, Rute until built [58].
- **Leftover space** *(synthesis)*: village cores are built solid or keep a small back court or
  garden [1][2]; large Spanish blocks a common court [9]; the 3 m rule [31] removes slivers. So
  every open area is one lot's (a garden with a door from its house), a shared court (doors from
  two houses at least), or a public alley or square; nothing walled with no door onto it.

## 6. Data and published methods

| Source | Content | Licence | Commercial use |
|---|---|---|---|
| Swiss Dwellings [59] (R) | 42 207 flats in 3 093 buildings: rooms, walls, about 550 000 doors and windows, room types | CC BY 4.0 | yes, credited; the best to calibrate windows by room |
| MSD [60] (R) | 5 300 plans, 18 900 flats (from the Swiss data) | CC BY 4.0 (linked) | yes |
| CubiCasa5K [61] (R) | 5 000 plans, 80+ object classes | CC BY-NC-SA 4.0 | no |
| RPLAN [62] (S) | 80 788 Asian flat plans | research only, no redistribution | no |
| LIFULL HOME'S [63] (R) | 5.15 M Japanese plan images | universities and public institutes, non-profit research | no |
| ResPlan [64] (R) | 17 000 South Asian plans | data CC BY 4.0, code MIT | allowed, but the plans come from listing sites |
| IGN BD TOPO [65] (S) | French footprints | Etalab 2.0 | yes, credited: real Provençal villages' footprints |
| Catastro INSPIRE [66] (S) | Spanish footprints | CC BY 4.0 per the catalogue; another page disagrees | check first |
| OpenStreetMap [67] (R) | footprints | ODbL | share-alike for derived data |

Nothing is downloaded without the owner's go.

| Method | What it gets right | Weakness |
|---|---|---|
| Merrell, Schkufza, Koltun 2010 [68] | a Bayesian network (reportedly from 120 real programmes) for the room list, sizes and adjacencies; optimised multi-storey plans | "idealized": no site, no facade |
| Marson & Musse 2010 [69] | squarified treemaps keep rooms near square in a fixed outline, in real time | corridors added after waste space [70] |
| Mirahmadi & Shami 2012 [70] | windows on outer walls except bathrooms; forbidden links (bedroom–bedroom, bedroom–kitchen) | |
| Lopes et al. 2010 [71] | constrained growth on a grid, designer control, reachability guaranteed | |
| Rodrigues et al. 2008 [72] | legal rules with an L-system | |
| WallPlan 2022 [73] | windows first, then walls | needs training data |
| CGA, Müller et al. 2006 [74][82] | facade split grammars, the bay grid's home | |

## 7. Patents

Status and expiry are Google Patents' estimates, not legal conclusions.

| Patent | Holder | Status | Relevance |
|---|---|---|---|
| US 12,220,640 B2 [75] (R) | Electronic Arts | active, to about 2043-10-29 | claim 1: a player's boundary and a budget in game currency, a floor-plan model and a graph-neural-network furniture layout; a rule-based generator is outside it as summarised. Avoid GNN furniture placement under a budget. |
| US 10,427,046 B2 [76] (R) | Take-Two | active, to about 2037-12-22 | procedural interiors in the description, claim 1 on asset metadata and shop items: low |
| US 9,129,432 B2 [77] (R) | HKUST | active, to about 2031-10-07 | image-based remodelling of buildings: not relevant |
| US 9,355,470 B2 [78] (R) | Stanford (Merrell et al.) | expired (fees unpaid) | interactive furniture layout by MCMC: free to use |
| US 2017/0076016 A1 [79] (R) | an individual | abandoned | room sizing and corridors |
| US 2018/0189422 A1 [80] (R) | Aditazz | abandoned 2025-12 | tile-based interior test fit |

No patent from Esri/Procedural on CGA was found, which proves nothing. Rule-based bays and
subdivision rest on the pre-2011 academic prior art [68]–[72][82].

## 8. Recommendation for Forge: rules for the generator

Ordered by how visible a breach is to a walker; each is a check for the anomaly checker the owner
asked for the same evening ("we need a way to detect anomalies").

1. **No slivers.** Two buildings on neighbouring lots touch (a shared party wall) or stand 3.0 m
   apart at least; a gap of 3 m or more is an alley (2.5 m clear, public), a shared court or one
   lot's garden.
2. **One roof a run.** Row houses of one eave height share one roof: one or two slopes at
   27–33 % (15–18°), the ridge along the row; no hip, valley or parapet at a party wall. Hips only
   on detached houses and outside corners.
3. **Heights snap.** Neighbours' eaves equal or 0.8 m apart at least; the step exactly on the
   party wall, the wall above the lower roof blank.
4. **Storeys by street.** A street under 4 m: two storeys (≤ 7 m to the eaves); 4–8 m: three
   (10 m); 8 m or more: four (13 m).
5. **Fronts in bays.** Mid-terrace fronts 3.5–7 m (one bay ≤ 6 m clear); two-bay houses 7–12 m
   with a spine wall; no house over 12 m of front in the core.
6. **Bays stacked.** Windows on vertical axes, one width up a bay, heads and sills aligned a
   floor, 1.5–2 times as tall as wide, upper ones no taller; 1–3 bays a house a floor; about
   10 % of houses irregular.
7. **Shutters fold.** A pier between windows one window wide at least; an end pier half a window
   and 0.5 m from the party wall.
8. **Blank walls.** No opening in a party wall; no clear window onto a neighbour's lot closer
   than 1.9 m (France), 1.5 (Italy), 2.0 (Spain): only small fixed obscured lights, sills ≥ 2.6 m
   on the ground floor, ≥ 1.9 m above.
9. **Every open space someone's.** No walled area without a door from a house, a shared court or
   an alley; boundary walls only on lot lines.
10. **Boundary heights.** Between gardens solid 1.8–2.0 m; on the street a 0.6–1.0 m low wall
    under railings, 1.8 m in all; hedges ≤ 2.0 m, 0.5 m off the line; trees over 2 m 2 m off it
    (3 m for tall trees in Italy).
11. **Empty lots.** France and Italy: no wall, or one low wall or fence on the street, plus the
    neighbours' garden walls; Spain: one blank wall 2.0–2.8 m on the street line. Never four new
    high walls round one.
12. **Setbacks.** In the core the facade on the street line; elsewhere 1–5 m behind a low front
    wall.
13. **No partition in a window.** No partition, slab or stair meets the facade within a window
    and 0.25 m each side: bays first, partitions on the piers.
14. **Every habitable room lit.** Each bedroom, living room and closed kitchen an openable window,
    ≥ 1/8 of its floor; a new or luxury house's openings ≥ 1/6 of its habitable area.
15. **One window, centred.** A room with one window has it centred on its stretch of facade
    (± 0.2 m).
16. **Depth.** A single-aspect habitable room ≤ 2.5 window heads deep (about 6 m) and within
    L/W + L/H ≤ 2/(1 − Rb) at Rb = 0.5; a dual-aspect house ≤ 12 m deep; over 15 m a court.
17. **Wet rooms.** A bathroom window in 60 % of houses and 30 % of flats, ≤ 0.5 m², sill ≥ 1.4 m,
    obscured; a WC's in 40 % and 15 %, sill ≥ 1.6 m; else an extractor.
18. **Stair windows** at half-landing height, half a storey off the floors' sills.
19. **Room sizes:** single bedroom ≥ 9 m² and 2.4 m wide; double ≥ 14 m² and 2.75 m; living room
    ≥ 14 m² (16–30) and 3.2 m; bathroom 4–7 m²; WC 1.2–2.0 m², 0.85 m; corridor ≥ 1.0 m (1.2
    wheelchair-first); bedrooms no longer than 1.6 times their width, living rooms 2.0.
20. **Ceilings:** habitable rooms ≥ 2.70 m (Italy; 2.50 acceptable), service rooms ≥ 2.40 m.
21. **The stair** along a party wall at mid-depth in a mid-terrace house; day rooms on the ground
    floor towards the garden, night rooms upstairs.
22. **Flats** ≥ 28 m² for one, ≥ 38 m² for two; two bedrooms about 60–70 m².
23. **Detached houses** open to the south, their north side 0–2 small windows; one bay (5–6 m) or
    two (9–12 m) deep, up to three times as long.
24. **Luxury:** French windows to the garden (≥ 2.15 m tall), a balcony off the master bedroom, a
    roof terrace keeping two thirds of the slope and 2.5 m of tiles over the eaves, a loggia; no
    room glazed over 30 % of its floor.
25. **Every bedroom its own door on circulation** (the owner, 2026-10-29: "two bedroom should not
    be linked together except if there is another door to a corridor for each"): no bedroom is
    entered only through another room; where rooms stand side by side, a landing (≥ 1.2 m) runs
    across the house between the front and the back rooms, each opening on to it.

**For the owner:** the wheelchair-first rule (no steps in homes, one walking level) and
multi-storey row houses meet at the stair: lifts, or the walked level kept to the ground floor.

## Sources

R = read; S = seen in search results only.

1. CRAUP Meyreuil, *maison de village* [docs] S — https://www.ville-meyreuil.fr/upload/PLU/URBA/craup-meyreuil_maison-de-village_03-hq.pdf
2. Comune di Cagliari, Castello's lots [docs] S — https://www.comune.cagliari.it/portale/protected/8918I/0/def/ref/DOC102413I/
3. Italian teaching and exam material on row houses [docs] S (which figure from which file uncertain) — https://www.uniroma1.it/sites/default/files/A10306PEVO1.pdf ; https://oldsaad.unicam.it/sites/saad.unicam.it/files/Temi%20%20-%20Giugno%202012.pdf ; https://www.skuola.net/universita/appunti/architettura-tecnica-1
4. Ara.cat, a Girona house [web] S — https://es.ara.cat/domingo/ruina-hogar_130_5647248.html
5. Metalocus, Pamplona (Ruiz Esquíroz) [web] S — https://www.metalocus.es/es/noticias/tejer-con-ladrillo-sustitucion-de-4-viviendas-por-josean-ruiz-esquiroz
6. Metalocus, Sant Cugat (Ferrando) [web] S — https://www.metalocus.es/es/noticias/vivienda-entre-medianeras-por-josep-ferrando
7. Cosentino, Seville's bays [web] S — https://www.cosentino.com/?p=77595
8. Barbastro NNSS Título V [standard] S (attribution uncertain) — https://v4.barbastro.org/attachments/article/63/documentos_05_normas_subsidiarias_titulo_V.pdf
9. PG Murcia, Título 5 [standard] R — https://normatia.com/es/normativa/pg-murcia/2005/titulo-5-regimen-del-suelo-urbano/
10. BOIB 2014, Alcúdia [standard] S — https://intranet.caib.es/eboibfront/pdf/es/2014/165/898634
11. Sanxenxo, Ordenanzas [standard] S — https://sanxenxo.es/attachments/article/2559/Ordenanzas%20Urbanismo.pdf
12. Salon-de-Provence PLU, UA [standard] S — https://www.salondeprovence.fr/wp-content/uploads/ancien_site/pdf/logement-urbanisme/PLU_2018-1/ZONAGE-2018/UA.pdf
13. A forum quoting a PLU's 15 m band [web] S — https://www.forumconstruire.com/construire/topic-154377.php
14. BS 8103-3, product page [standard] S — https://shop-checkout.bsigroup.com/products/structural-design-of-low-rise-buildings-code-of-practice-for-timber-floors-and-roofs-for-housing-1
15. Lancaster CC, timber floors [docs] S — https://www.lancaster.gov.uk/assets/attach/1346/Leaflet-7-Domestic-Timber-Floors-Timber-sizes-and-construction-details.pdf
16. ASEE, daylight rules of thumb [paper] S — https://peer.asee.org/daylight-rules-of-thumb-experimentally-examined.pdf
17. CIB, rules of thumb [paper] S — https://www.irbnet.de/daten/iconda/CIB_DC23487.pdf
18. Architect, "Mastering Sidelight" [web] S — https://www.architectmagazine.com/technology/lighting/mastering-sidelight_o
19. Camden daylight report, BS 8206-2's formula [docs] S — https://camdocs.camden.gov.uk/CMWebDrawer/Record/6310387/file/document
20. Haringey report, BS EN 17037's UK annex [docs] S — https://publicregister.haringey.gov.uk/sfc/servlet.shepherd/document/download/069Tu00000AbGgAIAV
21. AQC/ProRéno, typology MI2a [docs] S — https://proreno.fr/storage/media/shares/pdf/00079/FicheTypologie-MI2a.pdf
22. Wikipedia, Winds of Provence [web] S — https://en.wikipedia.org/wiki/Winds_of_Provence
23. A Fontvieille *mas* listing [web] S — https://www.french-property.com/fr/bien-a-vendre/3772-Alby6b1g8xijprz3
24. STAP du Var, round-tile roofs [docs] S — https://www.culture.gouv.fr/Media/Regions/Drac-Paca/Files/Aides-demarches/Demarches-et-conseils-architecture-patrimoine/Fiches-conseil/Stap-du-Var/05-Les-toitures-en-tuiles-rondes
25. Le Pouget PLU, UA [standard] S — https://le-pouget.com/wp-content/uploads/2015/08/ua-033.pdf
26. Iglesias, historic centre manual [docs] S — https://www.comune.iglesias.ca.it/.galleries/doc-tematiche/Manuale_per_il_recupero_del_Centro_Storico.pdf
27. Rosignano Monferrato regulation [standard] S — https://www.regione.piemonte.it/governo/bollettino/abbonati/2020/16/attach/aa_aa_comune%20di%20rosignano%20monferrato_2020-04-06_72218.pdf
28. Code civil art. 675 [standard] S — https://www.legifrance.gouv.fr/codes/id/LEGIARTI000006430206/2012-02-16/
29. Wikipedia, Party wall [web] S — https://en.wikipedia.org/wiki/Party_wall
30. Sauze-du-Lac OAP [docs] S — https://www.paca.developpement-durable.gouv.fr/IMG/pdf/f09321p0040_oap.pdf
31. Brocardi, Italian Civil Code art. 873 [standard] R — https://www.brocardi.it/codice-civile/libro-terzo/titolo-ii/capo-ii/sezione-vi/art873.html
32. UDAP 13, roof changes [docs] S — https://www.culture.gouv.fr/Media/Regions/Drac-Paca/Files/Aides-demarches/Demarches-et-conseils-architecture-patrimoine/Fiches-conseil/udap-des-bouches-du-rhone/les-modifications-de-toiture
33. PNR Verdon, renovation sheet [docs] S — https://www.parcduverdon.fr/sites/default/files/pnrverdon/pdf/2013_archietsavoirfaire_fiche9_rehab.pdf
34. Guérande AVAP, openings [docs] S — https://www.ville-guerande.fr/fileadmin/user_upload/AVAP/4-percements-.pdf
35. DM 5 luglio 1975 [standard] R — https://www.bosettiegatti.eu/info/norme/statali/1975_dm_05_07.htm
36. Edilizia.com, Salva Casa [web] R — https://www.edilizia.com/guide/requisiti-minimi-per-labitabilita-le-modifiche-del-decreto-salva-casa
37. Décret 2002-120 [standard] R — https://www.legifrance.gouv.fr/loda/id/JORFTEXT000000217471
38. Construires.fr, the 1/6 rule [web] R — https://www.construires.fr/surface-de-plancher/surface-vitree/
39. A forum quoting RT2012 art. 20 [web] S — https://www.forumconstruire.com/construire/topic-203460.php
40. Assemblée nationale, on the 1982 ventilation order [docs] S — https://questions.assemblee-nationale.fr/dyn/14/questions/QANR5L14QE55323.pdf
41. Criminocorpus, Code civil art. 677 [standard] S — https://criminocorpus.org/fr/legislation/droit/versions/779/677/pdf/
42. Construires.fr, art. 678–679 [web] S — https://www.construires.fr/vues-droites-et-obliques-art-678-679/
43. Studio Cataldi, *luci e vedute* [web] S — https://www.studiocataldi.it/articoli/24784-luci-e-vedute.asp
44. CTE DB-HS3 2022 [standard] R — https://normatia.com/es/normativa/cte-db-hs/2022/seccion-hs-3-calidad-del-aire-interior
45. Valencia, Decreto 151/2009 [standard] R — https://normatia.com/es/normativa/diseno-calidad-vivienda-comunidad-valenciana
46. Catalonia, Decret 141/2012: vlex [standard] R (scope only) — https://vlex.es/vid/minimas-habitabilidad-404943842 ; Certicalia [web] S — https://www.certicalia.com/preguntas/p/para-conseguir-cedula-habitabilidad-medidas-ventanas ; La Marea [web] S — https://www.lamarea.com/2023/05/26/infraviviendas-legales-cuales-son-las-condiciones-minimas-de-un-piso
47. Vozpópuli, Spanish Civil Code art. 582 [web] S — https://www.vozpopuli.com/economia/inmobiliario/aviso-a-los-duenos-de-chalets-el-motivo-legal-por-el-que-no-puedes-abrir-una-ventana-mirando-al-vecino.html
48. Planning Portal, ventilation [docs] S — https://www.planningportal.co.uk/permission/common-projects/extensions/building-regulations-ventilation
49. Pilkington, Approved Document O [web] S — https://www.pilkington.com/en-gb/uk/architects/standards-and-regs/building-regulations/england/conservation-of-fuel-and-power/overheating-in-new-dwellings ; Everest (another table) [web] S — https://everest.co.uk/planning-permission/building-regulations-part-o
50. Nationally Described Space Standard [standard] R — https://www.gov.uk/government/publications/technical-housing-standards-nationally-described-space-standard/technical-housing-standards-nationally-described-space-standard
51. Camden exhibition boards (London Housing Design Guide) [docs] S — https://contentauthortraining.camden.gov.uk/documents/20142/0/Agar+Grove+estate+redevelopment+-+exhibition+boards%2C+July+2014+%281%29.pdf/1484ed0b-1c1d-e53b-4625-e8e580d55d2e ; widths from a flashcard summary [web] S (low quality) — https://vocapp.com/dwelling-space-standards-summary-flashcards-128430
52. Futura, *trumeau* [web] S — https://www.futura-sciences.com/maison/definitions/maison-trumeau-10958/
53. Service-public F914, fences [docs] R — https://www.service-public.gouv.fr/particuliers/vosdroits/F914
54. Service-public F614, plantings [docs] R — https://www.service-public.gouv.fr/particuliers/vosdroits/F614
55. Wintzenheim PLU [standard] S — https://www.ville-wintzenheim.fr/wp-content/uploads/2021/05/Modification-simplifiee-n°6-du-PLU-Reglement.pdf
56. Ediltecnico, Italian Civil Code art. 892 [web] S — https://ediltecnico.it/distanza-tra-piante-e-confini-come-regolarsi/
57. Bogogno [standard] S — https://www.regione.piemonte.it/governo/bollettino/abbonati/2017/46/attach/aa_aa_comune%20di%20bogogno_2017-11-02_60934.pdf ; Lozzolo [standard] S — https://www.regione.piemonte.it/governo/bollettino/abbonati/2014/46/attach/aa_aa_comune%20di%20lozzolo_2014-10-30_48785.pdf
58. Ávila BOP [standard] S — https://www.diputacionavila.es/bops/2021/09-04-2021/09-04-2021_082121.pdf ; Aldea del Rey [standard] S — https://aldeadelrey.es/documentos/bando_vallado_solares.pdf ; Rute, BOP Córdoba [standard] S — https://bop.dipucordoba.es/visor-pdf/16-05-2012/BOP-A-2012-3092.pdf
59. Swiss Dwellings [dataset] R — https://zenodo.org/records/7070952
60. MSD [paper, dataset] R — https://arxiv.org/abs/2407.10121
61. CubiCasa5K [dataset] R — https://zenodo.org/records/2613548
62. RPLAN's terms, as a 2026 paper describes them [paper] S — https://arxiv.org/pdf/2605.14117
63. LIFULL HOME'S terms [docs] R — https://www.nii.ac.jp/dsc/idr/en/lifull/documents/lifull_homes-policy.html
64. ResPlan [dataset] R — https://github.com/m-agour/ResPlan ; its statistics [paper] S — https://arxiv.org/pdf/2508.14006
65. IGN open data under Etalab 2.0 [web] S — https://www.solutions-numeriques.com/?p=75628
66. Catastro Buildings, the national catalogue's record [dataset] S — https://www.idee.es/csw-codsi-idee/srv/api/records/ES_SDGC_BU
67. OpenStreetMap copyright [docs] R — https://www.openstreetmap.org/copyright
68. Merrell et al. 2010, abstract [paper] R — https://www.cs.princeton.edu/courses/archive/spring11/cos598A/abstracts/Merrell10a.html ; the 120 programmes [paper] S — https://arxiv.org/pdf/2005.08106
69. Marson & Musse 2010 [paper] S — https://onlinelibrary.wiley.com/doi/10.1155/2010/624817
70. Mirahmadi & Shami 2012 [paper] R — https://ar5iv.labs.arxiv.org/html/1211.5842
71. Lopes et al. 2010 [paper] R (abstract) — https://publications.graphics.tudelft.nl/papers/584
72. Rodrigues et al. 2008 [paper] R — https://iconline.ulo.pt/entities/publication/a21fc16f-e3e4-4835-8226-5f687358c9a1/full
73. WallPlan 2022 [paper] S — https://history.siggraph.org/?p=117866
74. A layout-generation review [paper] R — https://arxiv.org/html/2504.09694
75. US 12,220,640 B2 [docs] R — https://patents.google.com/patent/US12220640
76. US 10,427,046 B2 [docs] R — https://patents.google.com/patent/US10427046B2/en
77. US 9,129,432 B2 [docs] R — https://patents.google.com/patent/US9129432B2/en
78. US 9,355,470 B2 [docs] R — https://patents.google.com/patent/US9355470B2/en
79. US 2017/0076016 A1 [docs] R — https://patents.google.com/patent/US20170076016A1/en
80. US 2018/0189422 A1 [docs] R — https://patents.google.com/patent/US20180189422A1/en
81. French construction code, main and service rooms [standard] S — https://www.legifrance.gouv.fr/codes/id/LEGIARTI000043818263/2026-08-05
82. Müller et al. 2006, CGA [paper] S — https://www.doi.org/10.1145/1141911.1141931
