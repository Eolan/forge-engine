# Fauna: the village's animals as behaviour

The research behind `crates/forge-fauna` (#269 point 3, a parallel session of D-062 off the GPU):
how a flock, a herd and a school move together, how close each species lets people, vehicles,
dogs and cats come, what the village's animals (#266) do by the hour, and the patents met on the
way. Companion to [game-ai-ecosystems.md](game-ai-ecosystems.md) §4 (animals as needs, senses and
herds; Reynolds' steering and Couzin's zones are read there and not repeated) and
[flora.md](flora.md) (the trees and shrubs the animals rest in and browse). Labels follow the
house convention: **[paper] [web] [data] [patent]** and **foundational / still-current /
recent**. Each source was read on a reachable page on 2026-10-16 unless it is marked
*search-recorded* (seen in search results and abstracts only); see
[Verification notes](#verification-notes).

> **In five sentences.** A flock holds together and turns as one when each bird follows a fixed
> number of its nearest (six or seven, Ballerini et al. 2008) in three zones, away from the
> closest, along with the next, toward the rest (Couzin et al. 2002), at a nearly constant speed
> with a turn it cannot exceed (Hemelrijk and Hildenbrandt 2011), looking ahead for what it would
> hit (Reynolds 1999). How close an animal lets a threat come is its flight initiation distance:
> across Europe 13.3 m for rural birds and 7.1 m for urban ones (Díaz et al. 2013), 10 m for house
> sparrows over 26 Catalan villages and towns, shorter the larger the settlement (García-Arroyo et
> al. 2023); birds flee cars by the road's limit, not the car's speed (Legagneux and Ducatez
> 2013). Pet cats keep to about 3.6 ha (Kays et al. 2020) and are out about five hours a day
> (Jensen et al. 2022); goats on Mediterranean ranges walk 4–7 km a day, half of it browsing;
> swifts fly at about 10 m/s and twice that in their evening screaming parties (Henningsson et al.
> 2010). No patent was found on flocking or on animals fleeing at species distances; the fauna
> runs region by region like the flora (D-063), never in a window around the player.

**Contents**

1. [Moving together](#1-moving-together)
2. [Fleeing](#2-fleeing)
3. [The village's species](#3-the-villages-species)
4. [Patents](#patents)
5. [Recommendation for Forge](#recommendation-for-forge)
6. [Checked and left out](#checked-and-left-out)
7. [Verification notes](#verification-notes)

---

## 1. Moving together

**Michele Ballerini, Nicola Cabibbo, Raphael Candelier, Andrea Cavagna, Evaristo Cisbani, Irene
Giardina, Vivien Lecomte, Alberto Orlandi, Giorgio Parisi, Andrea Procaccini, Massimiliano
Viale, Vladimir Zdravkovic. "Interaction ruling animal collective behavior depends on
topological rather than metric distance: Evidence from a field study." *PNAS* 105(4), 2008,
1232–1237.** [paper] [foundational] *search-recorded*
<https://arxiv.org/abs/0709.1916>

Starling flocks of a few thousand reconstructed in three dimensions: each bird interacts with a
fixed number of neighbours, six to seven on average, not with all within a distance. In their
simulations the topological rule keeps a flock cohesive through density changes (a predator's
attack) where a metric one splits it.
*Bearing:* each bird follows its `neighbours` nearest groupmates (7 for sparrows, pigeons and
swifts; 4 for the looser gulls; 7 for the mullet), found by brute force within its group (at most
50), in a fixed order.

**Iain D. Couzin, Jens Krause, Richard James, Graeme D. Ruxton, Nigel R. Franks. "Collective
Memory and Spatial Sorting in Animal Groups." *Journal of Theoretical Biology* 218(1), 2002,
1–11.** [paper] [foundational] (read in [game-ai-ecosystems.md](game-ai-ecosystems.md) §4)

Three zones about each individual: repulsion, orientation, attraction; when any neighbour is in
the zone of repulsion it is the only rule that acts. Swarms, tori and polarised groups come from
the same rules with different zone widths.
*Bearing:* a neighbour closer than the species' `separation` turns the bird away and the others
are ignored that step; otherwise it steers along its neighbours' mean heading and toward their
middle, as much as it steers to its goal.

**Charlotte K. Hemelrijk, Hanno Hildenbrandt. "Some Causes of the Variable Shape of Flocks of
Birds." *PLoS ONE*, 2011 (StarDisplay).** [paper] [still-current]
*search-recorded* <https://pmc.ncbi.nlm.nih.gov/articles/PMC3150374>

In StarDisplay each bird flies at a nearly constant cruise speed, banks to turn and interacts with
seven neighbours; flocks change shape through their turns because the birds hardly vary their
speed (the university's press summary of the paper). The talks of the same group give a cruise of
about 10 m/s for starlings.
*Bearing:* a bird relaxes toward its species' cruise speed, cannot turn faster than its `turn`
(degrees a second) or climb faster than its `climb`; the banking and the lift are left out.

**Craig W. Reynolds. "Steering Behaviors For Autonomous Characters." GDC 1999.** [paper]
[foundational] (read in [game-ai-ecosystems.md](game-ai-ecosystems.md) §3)

Obstacle avoidance by looking ahead along the velocity and steering to the side that is clear.
*Bearing:* Forge's flyers test their next 0.8 s against the buildings and walls (with a margin of
0.6 m, 0.15 m near their perch) and, when the way is not clear, take the first flyable direction
among up, level and down, turned by 0° to 135° either way, at full speed or else at half; the
step they can actually take within their turn is checked again, and a step that would still end
in a wall becomes a climb on the spot and counts as a contact (none over the example's day).

## 2. Fleeing

A flight initiation distance (FID) is how close an animal lets a threat come before it flees,
measured by walking straight at it at a normal pace.

**Mario Díaz and co-authors (Díaz et al.). "The Geography of Fear: A
Latitudinal Gradient in Anti-Predator Escape Distances of Birds across Europe." *PLoS ONE* 8(5),
2013, e64634.** [paper] [still-current] <https://pmc.ncbi.nlm.nih.gov/articles/PMC3665823>
(read through Europe PMC's full text)

714 populations of birds in nine pairs of rural and urban sites from southern Spain to northern
Finland, 2009–2010, approached from at least 30 m at a normal walking speed: a mean FID of
11.46 m; 13.34 m rural, 7.12 m urban. The urban–rural difference shrinks northward with the
raptors' abundance.
*Bearing:* a plan marked urban scales its birds' distances by 7.12 / 13.34 = 0.53 (`plan::URBAN`).

**García-Arroyo and co-authors. "House Sparrow (*Passer
domesticus*) escape behavior is triggered faster in smaller settlements." *Scientific Reports*,
2023.** [paper] [recent] <https://pmc.ncbi.nlm.nih.gov/articles/PMC9925442/>

26 villages and towns in north-eastern Spain: a mean FID of 10.0 m (± 0.6 SE) per settlement,
from 18.0 m in the smallest village to 4.2 m in a town's old centre; settlement size alone
explains 70 % of it (sparrows are bolder in larger settlements, not in denser ones).
*Bearing:* the village's sparrows flee a person at 10 m.

**Pierre Legagneux, Simon Ducatez. "European birds adjust their flight initiation distance to
road speed limits." *Biology Letters* 9, 2013.** [paper] [still-current] *search-recorded*
<https://pmc.ncbi.nlm.nih.gov/articles/PMC3971718>

134 approaches by car of 21 species (more than half carrion crows, house sparrows and blackbirds)
on roads limited from 20 to 110 km/h: the FID grows with the limit while the car's own speed has
no effect; smaller birds let cars closer. (A correction: FIDs are shorter in spring and summer
than in autumn and winter.)
*Bearing:* a species' `flee_vehicle` is its distance at a 50 km/h limit, scaled by the street's
limit; the distances themselves are estimates.

**Urban pigeons, gulls and approaches that pass by.** *search-recorded*
- Feral pigeons in New York (519 birds): FID falls with pedestrian traffic and urban cover
  (<https://zenodo.org/records/3936120>).
- Rock doves in Melbourne let people closer than the native doves and mostly walk away rather than
  fly (Gendall et al., *Avian Research* 2015,
  <https://avianres.biomedcentral.com/articles/10.1186/s40657-015-0018-x>).
- Herring gulls in towns can be approached closer than rural ones (Goumas et al., *Animal
  Behaviour* 2020, <https://ore.exeter.ac.uk/repository/handle/10871/121871>).
- FIDs are measured on direct approaches; a threat passing by is tolerated closer. Forge's
  estimate: 0.6 of the distance when the threat is not coming at the animal (`sim::PASSING`).

*Bearing:* pigeons start to walk away at 1.5 times their distance and fly at 3 m; gulls walk away
at 1.3 times theirs. A bird perched 2.5 m or higher ignores threats on the ground, a cat on a wall
top (1.5 m or higher) ignores dogs. A group flees together: the flock, the herd, the school go
with the one that saw the threat.

## 3. The village's species

Sizes, speeds and distances as `data/fauna.toml` holds them; *est.* marks Forge's estimates.

| Species | Group | Speeds (m/s) | Flees a person at | Day | Sources |
|---|---|---|---|---|---|
| House sparrow | 6–14 | walk 0.4, fly 8, flee 11 | 10 m (car 6 m at 50 km/h, dog 12, cat 8, *est.* but the person's) | feeds 2–8 min in gardens, the square, the streets' edges; rests 2–8 min (10–25 at midday) in a tree, on a wall or a roof; roosts in its colony's tree from 10 min after sunset to 15 min before sunrise | García-Arroyo et al. 2023; flock sizes and times *est.* |
| Feral pigeon | 12–30 | walk 0.6, fly 16, flee 18 | 3 m, walks away from 4.5 m (*est.*) | on the square and the streets 8–20 min, on the roofs 5–15 min, circles over the square now and then (1.5–3 min at 16–24 m); on its ledge from half an hour before sunset to half an hour after sunrise | Alerstam et al. 2007 (pigeons 15–20 m/s); Gendall et al. 2015 |
| Yellow-legged gull | 3–8 | walk 0.9, fly 11, flee 13 | 9 m (*est.*) | soars over the harbour (40–70 m circles at 25–55 m), stands on the quay and the roofs, feeds on the quay (more in the morning), floats off the quay and there at night | Alerstam et al. 2007 (gulls at the slow end of 8–23 m/s); Goumas et al. 2020 |
| Common swift | 10–20 | fly 10, party 20.9, climb 4 | never (aloft or nesting) | aloft all day at 40–90 m; from 75 min before sunset screaming parties at 7–13 m along the streets; in its nest under the eaves from 20 min after sunset to 30 min before sunrise | Henningsson et al. 2010; Hedenström et al. 2016; screaming parties of 10–20 (Wikipedia) |
| Domestic cat | 1 | walk 0.8, flee 4 | 2.5 m; a dog at 12 m (*est.*) | out by the hour (0.8 at dawn and dusk, 0.35 at night, 0.3 by day): legs and rests of 2–45 min in its own and the neighbouring gardens, on the walls, along its front; home by a cat flap or the door | Kays et al. 2020; Jensen et al. 2022 |
| Domestic dog | 1 with its owner | owner 1.0–1.4, dog up to 2 | never (on a 1.8 m leash) | walked at about 7:00 (20–40 min), at noon one day in two (10–15 min), at about 19:30 (25–50 min), out and back along the streets, the owner stopping at some corners for the dog | a UK owners' survey: 34 min a walk (*search-recorded*); times *est.* |
| Goat | 12–24 | browse 0.25–0.5, walk 1, flee 3 | 4 m; a dog at 20 m (*est.*) | the fold until about 8:30, browsing shrub to shrub (25–90 s each) about a goal that moves 25–60 m every 8–15 min, the shade of the pasture's widest crown 12:30–15:30, the fold again at about 19:15 | Chebli et al. 2022; the Filabres study |
| Thicklip grey mullet | 20–50 | swim 0.4 (0.24 at night), flee 2.5 | 3 m from the quay's edge (*est.*) | schools along the quay, near the surface by day (a quarter of the time grazing the wall), 2–3 m down at night; a new goal every 40–120 s | photographic records of shoals grazing marina walls (*search-recorded*); speeds *est.* |

**Henningsson, Johansson, Hedenström. "How swift are swifts *Apus apus*?" *Journal of Avian
Biology* 41, 2010, 94–98.** [paper] *search-recorded*
<https://portal.research.lu.se/sv/publications/how-swift-are-swifts-apus-apus/>
Stereo high-speed filming of screaming parties: 20.9 ± 5.1 m/s horizontally, at most 31.1 m/s,
climbing at 4.0 ± 2.8 m/s, about twice their speeds in migration and roosting flights. Hedenström
et al. 2016 (*Current Biology*) tracked swifts aloft over 99 % of their ten months away from the
nests.

**Thomas Alerstam, Mikael Rosén, Johan Bäckman, Per G. P. Ericson, Olof Hellgren. "Flight speeds
among bird species: allometric and phylogenetic effects." *PLoS Biology* 5(8), 2007, e197.**
[paper] [foundational] <https://www.ebi.ac.uk/europepmc/webservices/rest/PMC1914071/fullTextXML>
Radar airspeeds of 138 species: means of 8–23 m/s; pigeons among the fast flyers at 15–20 m/s,
gulls and swifts in the lower part. The species' values are in its Protocol S1 (not read).

**Roland Kays and co-authors. "The small home ranges and large local ecological impacts of pet
cats." *Animal Conservation*, 2020.** [paper] [recent] *search-recorded* (Movebank data packages)
925 pet cats with GPS collars in six countries: home ranges of 3.6 ± 5.6 ha, only three over
1 km²; 75 % used mostly disturbed (built) habitat.

**Jensen and co-authors. "Movement Patterns of Roaming Companion Cats in Denmark — A Study
Based on GPS Tracking." *Animals* 12(14), 2022.** [paper] [recent] *search-recorded*
<https://www.ncbi.nlm.nih.gov/pmc/articles/PMC9311815/>
A median of 5 h a day away from home (2.5–8.8 h) and 2.4 km a day (1.3–3.7 km); less on rainy
days and for older cats. Other studies describe pet cats as crepuscular, more nocturnal in summer.

**Youssef Chebli, Samira El Otmani, Jean-Luc Hornick, Jérôme Bindelle, Jean-François Cabaraux,
Mouad Chentouf. "Estimation of Grazing Activity of Dairy Goats Using Accelerometers and Global
Positioning System." *Sensors* 22, 2022, 5629.** [paper] [recent] *search-recorded*
(doi 10.3390/s22155629)
Goats on a Mediterranean forest rangeland in northern Morocco walked about 4.5 km a day in spring,
6.5 in summer and 7.4 in autumn; grazing took 48 % of the spring daytime. The same group's work
on that rangeland: grazing 36–59 % of the time, resting 22–30 %, walking without grazing 10–24 %.
Semi-extensive goats in the Sierra de los Filabres (Almería) walked 4,295 m a day on average,
3,482 m in autumn to 5,763 m in summer, grazing 51.7 % and walking 42.0 % of their time on the
range (a *Journal of Range Management* study, *search-recorded*,
<https://repository.arizona.edu/handle/10150/644162>).
*Bearing:* the test holds a goat's day between 3 and 8 km; the example's herd walks 5.4 km.

## Patents

Searches on 2026-10-16 (web search; Google Patents' pages need a browser and were not queried
directly): flocking, boids, separation–alignment–cohesion and swarm animation in games; animals
or ambient creatures fleeing a player at a distance; creatures spawned in a region about the
player.

- **Flocking itself:** none found. Reynolds' boids (1987) and steering (1999) are publications;
  Couzin's zones and Ballerini's topological neighbours are science.
- **US 10,860,115 B1**, "Air transportation systems and methods", Ha Tran (inventor and
  assignee), United States, granted 2020-12-08, active, anticipated expiry 2039-09-19 (Google
  Patents' estimate). Its description mentions flocking for aerial vehicles; it concerns vehicles,
  not simulated animals, and its claims were not read. Not relevant to Forge's animals; no
  alternative needed.
- **Flight from a threat at a species' distance:** none found.
- **Fauna around the avatar:** Sony Online's US 8,115,765 B2 (now Daybreak, United States, active
  to about 2028-01-07; D-063) claims flora generated by rules in an area about an avatar and
  updated as it moves; it was met for the flora. The fauna is simulated over a whole region (a
  map, a world cell) from its plan and its seed, as the flora is placed; a game that spawns
  animals only in a window that follows the player should wait for the owner's word, as D-063
  says for plants.

## Recommendation for Forge

What `forge-fauna` builds (`docs/demos/fauna.md` has the numbers):
1. **Species as data**: one TOML record each, with its sources in comments; the behaviour (the
   day's routine) is code per kind, the numbers are data.
2. **A plan the animals read**: buildings with their eaves, walls, gardens, squares, the street
   graph with its limits, the water and its quay, a pasture and its fold, `forge-flora`'s trees
   and shrubs; the resting places (eaves, crowns, wall tops, bollards, nest cavities under free
   eaves, the sea) and feeding places (gardens, squares, streets' edges, the quay) derived from
   it. The village and the city fill it from their plans.
3. **Groups decide, members move**, at a fixed 10 Hz, as columns: a flock chooses among the five
   nearest quiet places by the hour; birds fly in Couzin's zones over seven neighbours, at a
   cruise with a turn limit, looking ahead with their own climb and turn; herds browse inside
   their fence; cats keep to their yard; dogs walk on the leash; schools keep to the quay.
4. **Flight distances by species and threat**, approaching or passing, scaled for towns and by
   the street's limit; the group flees together, to a refuge further from the threat.
5. **Deterministic**: every choice a hash of the group's seed and a counter, IEEE arithmetic and
   `forge_core::dmath`, a fixed order; a digest per run.

## Checked and left out

- **Predation and hunting:** cats never chase birds, nothing is caught or harmed (the owner's
  rule). Birds still keep their distance from cats.
- **StarDisplay's banking and lift:** a turn limit and a constant cruise give the turning flock;
  banking would matter for the bodies' animation, which is the renderer's.
- **A 3D navigation volume for birds** (Brewer's sparse voxel octrees, game-ai-ecosystems.md §3):
  a village's flyers look ahead against the plan's buildings and walls instead; a city's towers
  may need it.
- **ORCA for walkers:** the village's walkers rarely meet; goats keep a body's length by a push.
- **Swifts sleeping aloft:** the village's swifts are breeders and spend the night in their nests;
  the non-breeders' night ascents (to 3 000 m) are left out.
- **Seasons and weather:** a June day; swifts leave in August, gulls and pigeons stay.
- **Children:** none (the owner's rule).

## Verification notes

- Read on reachable pages or through Europe PMC's API: Díaz et al. 2013 (full text), García-Arroyo
  et al. 2023 (full text), Alerstam et al. 2007 (full text, not its species table), Chebli et al.
  2022 (its Crossref record), US 10,860,115 B1 (Google Patents' page, description only).
- Search-recorded: Ballerini et al. 2008, Hemelrijk and Hildenbrandt 2011, Legagneux and Ducatez
  2013, the New York pigeons, Gendall et al. 2015, Goumas et al. 2020, Henningsson et al. 2010,
  Hedenström et al. 2016, Kays et al. 2020, Jensen et al. 2022, the Filabres goats, the dog
  walking surveys, the mullet's photographic records.
- Estimates, marked in `data/fauna.toml`: every flight distance but the sparrow's from a person,
  the walk-off shares, the group sizes but the swifts', the cats', dogs', goats' and mullet's
  speeds, the dogs' walking times, the goats' times, the passing share.
