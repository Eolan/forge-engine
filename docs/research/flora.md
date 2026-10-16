# Flora: growing the village's plants and placing them on the hills

The research behind `crates/forge-flora` (#269, a parallel session of D-062): how a tree or a shrub
is grown as a branch graph from a few parameters, what the Mediterranean village's seven species
(#266) measure in published descriptions, where they grow on a hillside, and the patents met on the
way. Companion to [vegetation-materials.md](vegetation-materials.md) (the authoring pipelines and
the rendering ladder, D-013), [procedural.md](procedural.md) §4 (plant ecosystems) and
[planet-environment.md](planet-environment.md) §2–§3 (biomes, ecotones, succession); their entries
are referenced, not repeated. Labels follow the house convention: **[paper] [book] [web] [docs]
[patent]** and **foundational / still-current / recent**. Each source was read on a reachable page
on 2026-10-16 unless it is marked *search-recorded* (seen in search results only); see
[Verification notes](#verification-notes).

> **In five sentences.** A tree's skeleton grows best from the competition for space: markers
> scattered in the crown's envelope are claimed by the nearest bud that sees them (Runions et al.
> 2007), the light they stand for is summed down to the base and the vigour handed back up by
> Borchert and Honda's rule (Pałubicki et al. 2009), and the radii follow the pipe model
> (Shinozaki et al. 1964). A species is then a dozen numbers: its published height and spread,
> its crown's shape young and old, an internode, a branching angle, the apical control and a
> tropism; a seed is an individual. The village's plants measure 15–25 m (Aleppo pine), 20–28 m
> (holm oak), 5–10 m (olive), 12–30 m by 1.5–4 m (Italian cypress) and 0.5–2.5 m (rosemary,
> cistus, kermes oak) in nurseries' and floras' descriptions. On a Mediterranean limestone hill
> the garrigue holds the dry, shallow, sunny ground, Aleppo pine colonises it as a pioneer, and the
> holm oak, shade-tolerant, takes the deeper soils of the valleys and grows up under the pines.
> No patent was found on space colonisation or self-organising trees; the one live patent met is
> Sony Online's on flora generated in an area around an avatar (US 8,115,765 B2, to 2028), which
> a region-by-region placement does not need.

**Contents**

1. [Growing a tree from a few parameters](#1-growing-a-tree-from-a-few-parameters)
2. [The village's species](#2-the-villages-species)
3. [Where they grow: a Mediterranean hillside](#3-where-they-grow-a-mediterranean-hillside)
4. [Patents](#patents)
5. [Recommendation for Forge](#recommendation-for-forge)
6. [Checked and left out](#checked-and-left-out)
7. [Verification notes](#verification-notes)

---

## 1. Growing a tree from a few parameters

**Adam Runions, Brendan Lane, Przemyslaw Prusinkiewicz. "Modeling Trees with a Space Colonization
Algorithm." *Eurographics Workshop on Natural Phenomena*, 2007.** [paper] [foundational]
<https://algorithmicbotany.org/papers/colonization.egwnp2007.html>

Already in `vegetation-materials.md` §1. What `forge-flora` takes from it: attraction markers
scattered in the crown's envelope; each marker goes to the nearest bud within a perception radius
and cone; buds grow towards the mean direction of their markers; markers within the occupancy
(kill) radius of new wood are removed. The envelope is the species' crown shape, so the crown's
silhouette is controlled and its inside emerges.
*Bearing:* the free-space signal each bud receives (`grow`, step 1).

**Wojciech Pałubicki, Kipp Horel, Steven Longay, Adam Runions, Brendan Lane, Radomír Měch, Przemyslaw
Prusinkiewicz. "Self-organizing tree models for image synthesis." *ACM Transactions on Graphics*
28(3), 58:1–10, 2009.** [paper] [still-current]
<https://algorithmicbotany.org/papers/selforg.sig2009.html> (DOI 10.1145/1531326.1531364)

Already in `procedural.md` §4. The paper's abstract was read; its PDF (1.5 MB web version) could not
be read here (no PDF text extraction on this machine), so the algorithm below is the published
model as `vegetation-materials.md` and `procedural.md` summarise it, with Forge's own numbers. Each
growth cycle: the environment (light, or free space by colonisation) is computed per bud; a
basipetal pass sums it from the buds to the base; an acropetal pass hands the vigour from the base
up, split at each branching between the main axis and the laterals; each bud grows a shoot of
⌊vigour⌋ metamers along a direction weighted between its own, the free space's and a tropism;
branches that gather too little are shed; pruning and bending act on the same simulation.
*Bearing:* the loop of `grow::Grower::cycle`, steps 1 to 5.

**Rolf Borchert, Hisao Honda. "Control of Development in the Bifurcating Branch System of *Tabebuia
rosea*: A Computer Simulation." *Botanical Gazette* 145(2), 1984, 184–195.** [paper] [foundational]
<https://www.journals.uchicago.edu/doi/10.1086/337445> (search-recorded)

The resource-allocation rule Pałubicki et al. adopt: at a branching, the main axis receives
`v·λQm / (λQm + (1−λ)Ql)` of the vigour `v`, the lateral the rest, where `Qm` and `Ql` are the light
the two sides gathered and λ the apical control. λ above one half gives a leader (an excurrent pine
or cypress), below it a spreading crown (an olive). The abstract also notes that branch increase
slows because a crown's surface grows with the square of its size while branches multiply
exponentially.
*Bearing:* `Species::apical_control`, the λ of step 3; and why the branch count's growth with
age slows as a crown fills (the test grows plants at 20 %, 45 % and 100 % of maturity).

**K. Shinozaki, K. Yoda, K. Hozumi, T. Kira. "A quantitative analysis of plant form — the pipe model
theory. I. Basic analyses." *Japanese Journal of Ecology* 14, 1964, 97–105; "II. Further evidence of
the theory and its application in forest ecology", 133–139.** [paper] [foundational]
(search-recorded through the reviews below)
**Rodrigo Lehnebach et al. "The pipe model theory half a century on: a review." *Annals of Botany*
121(5), 2018, 773–795.** [paper] [still-current]
<https://hal-centralesupelec.archives-ouvertes.fr/hal-01823925>

The stem's conducting cross-section at a height is proportional to the leaf area above it: each
leaf unit is joined to the base by a unit pipe. Lehnebach et al. find most of its properties do not
hold as general rules (heartwood, the branchless part of the stem), which matters for wood science,
not for a picture. Graphics uses the generalised form `rᵖⁿ = Σ rᶜⁿ`, `n` between 2 (Leonardo's
rule) and 3.
*Bearing:* `pipe_radii`: every twig tip of the graph carries one leaf cluster and has the species'
`tip_radius`; a parent's radius to the `n` is its children's summed, so a child is never thicker
than its parent by construction. The tip radius stands for the bundle of real shoots behind one
cluster and is set so the trunk comes out at the published diameters (an Aleppo pine of 19 m:
0.34 m against "up to 60 cm" on Wikipedia).

**F. J. Richards. "A Flexible Growth Function for Empirical Use." *Journal of Experimental Botany*
10(2), 1959, 290–300.** [paper] [foundational] (search-recorded; DOI 10.1093/jxb/10.2.290)

The sigmoid family behind the Chapman–Richards curve foresters fit height growth with:
`H(t) = A (1 − e^(−kt))^c`.
*Bearing:* `species::growth_share`, with `c = 2` and `k` set so a plant reaches 95 % of its height
at the species' `maturity`; the Aleppo pine's curve gives 2.7 m at 10 years where the Conifer
Society gives 1.5–3 m.

**Christer Ericson. *Real-Time Collision Detection*. Morgan Kaufmann, 2005, §5.1.9.** [book]
[foundational]

The closest points of two segments, which the growth's clearance and the crossing check use.

---

## 2. The village's species

The published ranges are the tests' bounds (`tests::grown_plants_have_their_published_size`). Where
two sources differ, the range spans both: a nursery's figure is a grown street or garden tree's, a
flora's a wild plant's. The growth parameters (internode, angles, λ, tropism, the crown's shape)
are estimates fitted to the descriptions quoted, not measurements; `data/mediterranean.toml` holds
them with a comment per species.

| Species | Height, m | Spread, m | Form described | Sources |
|---|---|---|---|---|
| Aleppo pine, *Pinus halepensis* | 15–25 | 7.6–15 | "rounded to flat-topped crown", "slender, irregular horizontal, upturned branches", 1.5–3 m after 10 years; trunk up to 60 cm across, exceptionally 1 m | Wikipedia; Conifer Society; Sacramento Tree Foundation (30–60 ft by 25–60 ft); Van den Berk (15–20 m by 10–15 m, round crown, light-loving) |
| Holm oak, *Quercus ilex* | 20–28 | 12–28 | "broad ovoid, dark, dense crown", "low branches that stand at an angle of 45°", a short trunk; in the open "a huge head … as much across"; shade-tolerant | Van den Berk (20–25 m by 12–15 m); Wikipedia (21–28 m in favourable places); EUFORGEN |
| Olive, *Olea europaea* | 5–10 | 8–12 | "round to fan-shaped … crown", "tangled, dense branching", a gnarled and twisted trunk; warmth-loving, dry sunny sites, well-drained calcareous soils | Van den Berk; Wikipedia ("some 10 m") |
| Italian cypress, *Cupressus sempervirens* 'Stricta' | 12–30 | 1.5–4 | columnar, fastigiate, the "exclamation mark"; branches growing upwards | RHS Stricta Group (over 12 m by 2.5–4 m); Van den Berk (1.5–2 m wide; 20–30 m in its text); Wikipedia (to 35 m for the species) |
| Rosemary, *Salvia rosmarinus* | 1.2–2.5 | 1.5–2.5 | upright to trailing, bushy, evergreen; 5–10 years to its size; south or west facing, well-drained chalk, loam or sand | Wikipedia (upright forms 1.2–1.8 m); RHS (1.5–2.5 m by 1.5–2.5 m) |
| Grey-leaved cistus, *Cistus albidus* | 0.5–1 | 0.5–1.5 | compact, bushy; 5–10 years to its size; south or west facing, well-drained | Wikipedia (up to 1 m); RHS (0.5–1 m by 0.5–1 m); Beth Chatto (1 m by 1.5 m) |
| Kermes oak, *Quercus coccifera* | 1–2 | 1–3 (estimate) | "usually a shrub less than 2 metres", rarely a small tree (1–6 m); dense thickets in the garrigue, regrowing fast after fire | Wikipedia; the Calanques national park (search-recorded). No published spread was found: the range is an estimate |

**Van den Berk Nurseries. Tree pages for *Pinus halepensis*, *Quercus ilex*, *Olea europaea*,
*Cupressus sempervirens* 'Stricta'.** [web] [still-current]
<https://www.vdberk.co.uk/trees/pinus-halepensis/> — <https://www.vdberk.co.uk/trees/quercus-ilex/> —
<https://www.vdberk.co.uk/trees/olea-europaea/> —
<https://www.vdberk.co.uk/trees/cupressus-sempervirens-stricta/>

A Dutch nursery's catalogue: height, width, crown, branching, bark, soil and exposure per tree, the
same fields for every species, for grown trees in towns.
*Bearing:* the one source with a spread for every tree species, and the crown and branching words
the parameters were fitted to.

**Royal Horticultural Society. Plant pages: *Salvia rosmarinus*, *Cistus albidus*, *Cupressus
sempervirens* Stricta Group.** [web] [still-current]
<https://www.rhs.org.uk/plants/384211/wd/details> — <https://www.rhs.org.uk/plants/3862/wd/details> —
<https://www.rhs.org.uk/Plants/133025/Cupressus-sempervirens-Stricta-Group/Details> (the last
search-recorded)

Ultimate height and spread, the time to reach them, aspect, soil and exposure.
*Bearing:* the shrubs' sizes and `maturity` (5–10 years: 8).

**The American Conifer Society. "Pinus halepensis".** [web] <https://conifersociety.org/conifers/pinus-halepensis/>
**Sacramento Tree Foundation. "Aleppo pine".** [web] <https://sactree.org/trees/aleppo-pine-2/>
**Wikipedia. "Pinus halepensis", "Quercus ilex", "Olive", "Cupressus sempervirens", "Rosemary",
"Cistus albidus", "Quercus coccifera", "Garrigue".** [web]
**EUFORGEN. "Pinus halepensis", "Quercus ilex".** [web] <https://www.euforgen.org/species/pinus-halepensis> —
<https://www.euforgen.org/species/quercus-ilex>
**Beth Chatto's Plants & Gardens. "Cistus albidus".** [web] (search-recorded)
**Parc national des Calanques. "Kermes oak".** [web] <https://calanques-parcnational.fr/en/kermes-oak>
(search-recorded: the page's certificate did not match its host)

Each read for the figures in the table. EUFORGEN gives no numbers for either species but the
traits placement uses: the Aleppo pine fast-growing, drought-resistant, on all substrates in hot,
fire-prone places; the holm oak shade-tolerant, germinating and resprouting in woodland
understoreys.

---

## 3. Where they grow: a Mediterranean hillside

**Wikipedia. "Garrigue".** [web] <https://en.wikipedia.org/wiki/Garrigue>

Garrigue is low, discontinuous scrub on limestone, a degradation stage of the evergreen oak forest
(grazing, clearing, charcoal-making, fire), with kermes oak thickets, rosemary, thyme, lavender,
white cistus and stunted holm oak; maquis is the denser scrub of the siliceous soils.
*Bearing:* the garrigue's shrubs share the open, shallow, sunny ground; the holm oak is the
forest the garrigue stands in for, where the soil is deep enough.

**M. Sternberg, M. Shoshany. "Influence of slope aspect on Mediterranean woody formations:
Comparison of a semiarid and an arid site in Israel." *Ecological Research* 16(2), 2001, 335–345.**
[paper] [still-current]
<https://agris.fao.org/search/ar/records/65df4f470f3e94b9e5d74424> (DOI 10.1046/j.1440-1703.2001.00393.x)

Sixteen quadrats of 10 by 10 m per site, half on north-facing and half on south-facing slopes:
aspect changes the woody communities' composition, structure and density significantly over the
short distance between opposite slopes, at both ends of the rainfall gradient. The abstract does not
say which side carries more cover; the review cited in the same search reports the strongest effect
where water is most limiting.
*Bearing:* aspect is a placement input of its own. Which way it pushes is taken from the French
sources below (the sunny slopes are the driest).

**Annales des Sciences Forestières, 1965, on the Aleppo pine in Provence; Options Méditerranéennes
(CIHEAM) on the Aleppo pine's ecology; a general-public page (techno-science.net); a ZNIEFF record
of the Alpes-de-Haute-Provence (INPN 930020378).** [paper] [web] (all search-recorded; the PDFs
could not be read here)
<https://www.afs-journal.org/articles/forest/pdf/1965/02/AFS_0003-4312_1965_22_2_ART0001.pdf> —
<https://ressources.ciheam.org/ressources/om/pdf/s10/CI010857.pdf> —
<https://www.techno-science.net/fr/definitions/pin-d-alep> —
<https://inpn.mnhn.fr/docs/ZNIEFF/znieffpdf/930020378.pdf>

As the search results quote them: the Aleppo pine settles "most often in the degradation facies of
the holm oak forest", on hard limestones, then marly ones (AFS 1965); it tolerates much active lime
and is found above all on warm, well-drained substrates, the skeletal limestone soils where other
species are eliminated (CIHEAM); in France up to 600–800 m on the south-facing slopes, in the
garrigue country, the one large tree that grows naturally in poor, dry limestone (techno-science);
north-facing steep slopes with thalwegs carry downy oak, the drier, shallower south-facing slopes
holm oak (ZNIEFF).
*Bearing:* the Aleppo pine on the dry, shallow ground (the sunny slopes and the ridges), the
garrigue with it, the oaks where the soil is deeper and wetter (the valleys and the shaded slopes).
These are statements from search results, marked so in the placement's rules.

**Spatial pattern of *Quercus ilex* and *Quercus pubescens* recruitment in *Pinus halepensis*
dominated woodlands (Spain).** [paper] (search-recorded, AGRIS record
<https://agris.fao.org/search/ru/records/65de15c50f3e94b9e5ca9c11>)

Oak seedlings are positively associated with pines: the pines are safe sites for the oaks'
recruitment.
*Bearing:* light under a canopy is a rule per species: the holm oak may stand under the pines (it
is shade-tolerant, EUFORGEN), the garrigue's shrubs and the pine itself need open ground.

**Robert Bridson. "Fast Poisson Disk Sampling in Arbitrary Dimensions." *ACM SIGGRAPH 2007 Sketches*,
2007.** [paper] [foundational]
<https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf>
**Li-Yi Wei. "Multi-Class Blue Noise Sampling." *ACM Transactions on Graphics* 29(4), 79:1–8, 2010
(SIGGRAPH 2010).** [paper] [still-current] (search-recorded; DOI 10.1145/1778765.1778816; the method of
US 8,219,517 B2 below)

Dart throwing with a background grid gives points no closer than a radius in linear time; the
multi-class form gives each pair of classes its own radius (a matrix), so a pine keeps its distance
from a pine and less from a shrub.
*Bearing:* spacing by species: each pair keeps the sum of its two radii, the canopy's from the
canopy, the understorey's from the trunks.

The procedural.md §4 entries (Deussen et al. 1998's distribution → plant → rendering pipeline, Lane
and Prusinkiewicz 2002's global-to-local direction) and the SideFX Labs biome entry of
planet-environment.md §2 (viability per species from tolerances, then competition) are the
architecture placement follows.

---

## Patents

Found with WebSearch and Google Patents pages on 2026-10-16. Statuses and expiry dates are Google
Patents' own (it labels them assumptions, not legal conclusions). A record of what was met, not a
freedom-to-operate review.

| Patent | Holder | Jurisdiction | Status (as listed) | Expiry (estimate) | Technique | Alternative |
|---|---|---|---|---|---|---|
| US 8,115,765 B2 (and US 2005/0264567 A1, US 2005/0264566 A1, JP 4 937 907 B2) | Sony Online Entertainment, now Daybreak Game Company | US, JP | granted 2012-02-14, **active** | 2028-01-07 | terrain from layered procedural rules (boundaries, filters, affectors) blended where they overlap; claim 38: flora generated by rules in a user-defined area around an avatar, claim 39: its density, locations, and new locations as the avatar moves (claims read) | place by region, once, as a pure function of the ground and a seed (`forge-flora`'s placement), not in a window that follows the player. **Ask the owner before building a flora streamer centred on the player before 2028.** |
| US 8,219,517 B2 | Microsoft | US | granted 2012-07-10, **expired** (fees) | — | multi-class Poisson disk sampling with a matrix of radii (Wei 2010) (read) | free to use |
| US 2010/0128046 A1 | Microsoft | US | **abandoned** application | — | parallel Poisson disk sampling on a grid of cells (read) | free to use |
| KR 10-1295063 B1 (KR 2013-0073523 A) | Hanbat National University | KR | granted 2013, **lapsed** (fees, 2020) | — | tree branches grown recursively from candidate buds in a growth space (claims read) | free to use |
| US 9,367,944 B2 | ETRI | US | granted 2016-06-14, **expired** (fees) | — | a tree model from a photograph's skeleton, with library branches and a "self-growth" step (read) | not needed |
| US 2009/0174703 A1 | Disney | US | **abandoned** application | — | 3-D trees grown from a seed particle steered by light and obstacles (`procedural-content.md`) | free to use |

**Looked for and not found** (searches only, here and in `procedural-content.md`): a patent on space
colonisation or on self-organising trees (Runions 2007, Pałubicki 2009); a SpeedTree or IDV patent
on tree modelling, wind or LOD. Their absence from the results is not proof that none exists.

---

## Recommendation for Forge

*Opinion, shaped by the engine's constraints: CPU-side generation, deterministic by seed (D-016),
the renderer's ladder of D-013 to feed.*

**Grow every plant from its species' record.** Space colonisation for the free space, the
self-organising vigour for its sharing, the pipe model for the radii: one loop for trees and shrubs,
tested on its graphs (branches by age, taper, no wood through wood, the published sizes,
determinism). This is D-013's "grown, not modelled" made concrete; nothing here needs a new
decision. A species is a TOML record; a seed an individual; age, site, pruning and wind inputs.

**Keep the species' numbers honest.** Heights and spreads from nurseries and floras with their
sources; the growth parameters marked as estimates fitted to the descriptions; a field trip's
photographs (or the reconstruction route of `vegetation-materials.md` §1) would replace the
estimates later.

**Place by rules over the ground, region by region.** Viability per species from slope, aspect,
soil depth, wetness and height; light under the canopy placed first; spacing by pairs of species;
everything a pure function of the ground and the seed. Not a window following the player: that is
where Sony Online's US 8,115,765 B2 sits until 2028.

**Cook, don't grow per instance.** A few individuals per species and age class, grown once and
cached (D-053's derived data), instanced by the placement; the renderer's ladder (D-013) builds its
bands from the graph and the leaf clusters.

---

## Checked and left out

- **Weber–Penn's recursive parameters as the generator** (`vegetation-materials.md` §1): kept as the
  idea of a flat species file; the branches themselves come from colonisation, which reads the
  envelope rather than four levels of recursive angles.
- **L-systems:** a grammar fixes the topology; the self-organising model gets it from the space and
  the light, which is what makes neighbours and pruning act on it.
- **Shadow propagation** (Pałubicki 2009's other environment): left for the forest scale, where
  neighbours shade each other; within one plant, colonisation's markers stand for both space and
  light.
- **Pirk et al. 2012's plastic trees:** the next step for trees that bend away from a wall or a
  neighbour after growing; not needed for isolated plants.
- **Measured allometry for the Aleppo pine's crown:** searched (Greek fuel-load allometry, Sicilian
  height–diameter models, Algerian stand structure), no crown width against height found; the
  nursery and city-tree figures stand in.

## Verification notes

- **Read on 2026-10-16:** the Wikipedia pages listed, the Conifer Society, the Sacramento Tree
  Foundation, Van den Berk's four tree pages, the RHS pages for rosemary and *Cistus albidus*,
  EUFORGEN's two pages, the algorithmicbotany.org page of Pałubicki 2009 (abstract), the AGRIS
  record of Sternberg and Shoshany 2001, the Google Patents pages of US 8,115,765 B2 (including its
  claims), US 8,219,517 B2, US 2010/0128046 A1, KR 2013-0073523 A and US 9,367,944 B2.
- **Search-recorded only:** Borchert and Honda 1984, Shinozaki et al. 1964 (through the 2018 review
  and the Polish pine study), Richards 1959, Bridson 2007's venue, the RHS Stricta Group page, Beth
  Chatto, the Calanques national park (a certificate error), the AFS 1965 article (403), the CIHEAM
  PDF (unreadable without a PDF tool), techno-science.net, the ZNIEFF record, the Spanish recruitment
  study.
- **Not read:** Pałubicki 2009's full text and its parameter values (the PDF could not be extracted
  here); the numbers in `data/mediterranean.toml` are Forge's.
