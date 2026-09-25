# Planetary environment

An annotated bibliography for the *living surface* of Forge's planets: where a climate comes from
and how a game can compute or bake it, how climate becomes biomes and how biomes meet, what grows
and lives in them and how that changes, how weather is drawn, and — the part no other file covers —
how all of it is represented and streamed so that rendering, materials, vegetation, audio and
gameplay read one state. It answers issue #11, queued with D-019 when the owner asked for whole
Earth-like planets with biomes, biome and weather transitions, and ecosystems. Companion to
[RESEARCH.md](../RESEARCH.md). Entries already in sibling files are referenced by section and not
repeated: [procedural.md](procedural.md) (terrain genesis §1; No Man's Sky and WorldBrush §3; the
plant-ecosystem papers of §4 — Deussen 1998, Lane and Prusinkiewicz 2002, Pałubicki 2009, Pirk 2012,
Synthetic Silviculture 2019, Ecoclimates 2022, Kapp 2020, EcoBrush 2017; texture blending §5),
[large-worlds.md](large-worlds.md) (Horizon's GPU placement §5; the cube-sphere, S2 cell ids and CDLOD
§8), [lighting-gi.md](lighting-gi.md) (RDR2's integrated atmosphere §3; Hillaire's sky, the Nubis
clouds, froxel fog and the night sky §6), [vegetation-materials.md](vegetation-materials.md) (grass and
the Tsushima wind field §3; Lagarde's rain parts 2b and 3a and snow deformation §7; the unified
material row §8) and [audio.md](audio.md) (wind and rain sound, §2.5 and §4). Labels follow the house
convention — **[paper] [book] [talk] [web] [code] [docs]** and **foundational / still-current /
recent** — and every citation was confirmed against at least one reachable page; what could not be
confirmed is listed at the end, with what was tried.

> **State of the art in five sentences.** For a game, a climate is a few smooth monthly fields —
> temperature, precipitation, wind — and a lookup that turns them into zones (Whittaker's diagram,
> Holdridge's life zones, Köppen's thresholds, or BIOME1's plant tolerances), and the generators
> whose maps look right (mapgen4, Dwarf Fortress, WorldEngine, AutoBiomes, Houdini 20.5's biome
> tools) compute those fields from latitude, altitude and wind-carried moisture with rain shadows,
> not from a noise function. The physics that makes the fields plausible is old and cheap — a zonal
> energy balance for temperature (Budyko 1969, North 1981), a linear upslope model for orographic rain
> (Smith and Barstad 2004) — and a real but fast climate model (ExoPlaSim, 2022) now runs on a desktop
> in minutes to an hour, which makes "bake the planet's climate once, validate it against a GCM" a
> realistic pipeline step. Biome boundaries are ecotones, and ecology says they are wide where the
> climate gradient is smooth and sharp where a positive feedback switches the ground from one state to
> another (fire, water, shade), so a game needs both a soft climate blend (Minecraft's nearest-point
> multi-noise selection, scattered-kernel blending) and hard local rules. Weather *rendering* is mature
> and mostly 2004–2013 technique — streak-textured rain, porosity-driven wet darkening and puddles,
> depth-from-above occlusion for rain and snow, weather-map-driven clouds — while physically simulated
> weather (Stormscapes 2020, Weatherscapes 2021, Cyclogenesis 2024) is interactive only over tens of
> kilometres. At planet scale the shipped pattern is a coarse global field with local detail
> (meteoblue's grid behind Microsoft Flight Simulator, FV3's cubed-sphere nests), and for a
> deterministic multiplayer game that global field can be a pure function of climate, seed and time,
> with only the surface state around players integrated.

**Contents**

1. [Climate a game can compute or bake](#1-climate-a-game-can-compute-or-bake)
2. [Biomes and their transitions](#2-biomes-and-their-transitions)
3. [Ecosystems: flora, succession, fauna](#3-ecosystems-flora-succession-fauna)
4. [Weather rendering](#4-weather-rendering)
5. [The state model](#5-the-state-model)
6. [Recommendation for Forge](#recommendation-for-forge)
7. [Checked and left out](#checked-and-left-out)
8. [Verification notes](#verification-notes)

---

## 1. Climate a game can compute or bake

This section follows the order of a generator, read backwards: first the classifications that
consume the fields, then the cheap physics that produces them, then what planet and map generators
actually ship. Terrain genesis (procedural.md §1) is an input here; Ecoclimates (procedural.md §4),
which lets vegetation feed back on local weather, is the principled coupling and is not repeated.

### Classifications: from fields to zones

**R. H. Whittaker. *Communities and Ecosystems*, 2nd ed., Macmillan, 1975 — as digitised by Valentin
Ștefan and Sam Levin, `plotbiomes` (R package, v1.0.0, 2018, MIT).** [book] [code] [foundational]
<https://github.com/valentinitnelav/plotbiomes>

The diagram most game biome tables descend from: about nine biomes, tundra to tropical rainforest,
drawn as regions of mean annual temperature (°C) against annual precipitation (cm). The book is
confirmed only through secondary pages (see *Verification notes*); `plotbiomes` ships the diagram as
polygons in those units, drawn after Ricklefs' *The Economy of Nature* (fig. 5.5), which is the
machine-readable form a generator needs. AutoBiomes (below) uses a discretised, slightly modified
version of the same diagram as its lookup table.
*Bearing:* the first biome lookup Forge can have, because it needs two fields and its polygons load
straight into a table. Its weakness is the lesson: annual means hide seasonality, so a monsoon savanna
and a wet temperate forest can fall in the same cell — which is why the next entries add seasons.

**L. R. Holdridge. "Determination of World Plant Formations From Simple Climatic Data." *Science*
105(2727), 1947, 367–368.** [paper] [foundational]
<https://www.science.org/doi/10.1126/science.105.2727.367> (PubMed:
<https://pubmed.ncbi.nlm.nih.gov/17800882/>)

Life zones from three indicators: mean annual biotemperature, total annual precipitation, and the
ratio of mean annual potential evapotranspiration to precipitation. The third axis is the useful
addition — it says how much of the rain is actually available to plants, which Whittaker's two axes
cannot express.
*Bearing:* the classification WorldEngine uses (below). The PET/precipitation ratio is a field worth
carrying through Forge's whole pipeline: the same number drives drought stress for vegetation, fire
risk, and how fast wet ground dries after rain.

**M. C. Peel, B. L. Finlayson, T. A. McMahon. "Updated world map of the Köppen-Geiger climate
classification." *Hydrology and Earth System Sciences* 11(5), 2007, 1633–1644; H. E. Beck, N. E.
Zimmermann, T. R. McVicar, N. Vergopolan, A. Berg, E. F. Wood. "Present and future Köppen-Geiger
climate classification maps at 1-km resolution." *Scientific Data* 5, 180214, 2018.** [paper]
[still-current]
<https://hess.copernicus.org/articles/11/1633/2007/> — <https://doi.org/10.1038/sdata.2018.214>
(open copy: <https://www.ncbi.nlm.nih.gov/pmc/articles/PMC6207062/>)

Köppen's classes are thresholds on monthly temperature and precipitation, which is why they capture
seasons where Whittaker does not. Peel et al. compute them from station time series interpolated on a
0.1° grid with thin-plate splines, publish the map as a free supplement, and report hot desert (BWh,
14.2 % of land) and tropical savanna (Aw, 11.5 %) as the largest classes. Beck et al. take the same
classification to 1 km for 1980–2016, from four topographically corrected climate maps, and to
2071–2100 from 32 model projections; the gain is detail where gradients are sharp in space or
elevation, and the stated users include species and vegetation distribution modelling.
*Bearing:* Köppen is Forge's *validation* view rather than its biome table: compute it from the baked
monthly fields as a debug overlay and compare the class shares of an Earth-like preset against Peel's.
Beck's 1 km maps show how fine the climate must be near mountains — the argument for the regional
downscale in the Recommendation.

**I. C. Prentice, W. Cramer, S. P. Harrison, R. Leemans, R. A. Monserud, A. M. Solomon. "A global
biome model based on plant physiology and dominance, soil properties and climate." *Journal of
Biogeography* 19(2), 1992, 117–134; with D. M. Olson, E. Dinerstein et al. "Terrestrial Ecoregions of
the World: A New Map of Life on Earth." *BioScience* 51(11), 2001, 933–938.** [paper] [foundational]
<https://researchers.mq.edu.au/en/publications/a-global-biome-model-based-on-plant-physiology-and-dominance-soil/>
(HAL: <https://amu.hal.science/hal-01788308>) —
<https://academic.oup.com/bioscience/article-abstract/51/11/933/227116>

BIOME1 predicts vegetation from plant functional types rather than from a diagram: each type has
physiological limits on the mean temperature of the coldest month, the temperature accumulated above
5 °C over the year, and a drought index that includes the seasonality of precipitation and the soil's
water capacity; the biome is the dominant type that survives. Olson et al. supply a widely used
taxonomy: 867 terrestrial ecoregions grouped into 14 biomes.
*Bearing:* the shape Forge's biome rules should take: species (or plant types) carry *tolerances* on a
few climate indices and a biome is what wins, not a polygon on a chart. It is the same mechanism as
Houdini's plant viability (§2) and extends down to per-species placement without a second system.

### Physics cheap enough to bake

**André L. Berger. "Long-Term Variations of Daily Insolation and Quaternary Climatic Changes."
*Journal of the Atmospheric Sciences* 35(12), 1978, 2362–2367; Alice Nadeau, Richard McGehee. "A simple
formula for a planet's mean annual insolation by latitude." *Icarus* 291, 2017, 46–50.** [paper]
[foundational] [still-current]
<https://journals.ametsoc.org/view/journals/atsc/35/12/1520-0469_1978_035_2362_ltvodi_2_0_co_2.xml> —
<https://arxiv.org/abs/1510.04542>

Berger gives the trigonometric series for the orbital elements (eccentricity, precession, obliquity)
from which daily insolation at any latitude and date follows. Nadeau and McGehee fit the *annual mean*
insolation by latitude for any obliquity with a sixth-order Legendre series, for faster computation
with little loss of accuracy.
*Bearing:* the forcing term of the whole bake. What Forge needs from Berger is the geometry — daily
insolation for a given obliquity, eccentricity and day of year — not the Earth's long-term series; with
it a planet can have its own tilt and year length, and seasons follow.

**Gerald R. North, Robert F. Cahalan, James A. Coakley Jr. "Energy balance climate models." *Reviews of
Geophysics* 19(1), 1981, 91–121; M. I. Budyko. "The effect of solar radiation variations on the climate
of the Earth." *Tellus* 21(5), 1969, 611–619.** [paper] [foundational]
<https://agupubs.onlinelibrary.wiley.com/doi/abs/10.1029/rg019i001p00091> —
<https://tellusjournal.org/articles/10.3402/tellusa.v21i5.10109>

North et al. survey energy-balance models with an emphasis on analytical results: a sequence of
increasingly complicated models with ice-cap and radiative feedbacks, solved, with their parameter
sensitivities. Budyko's paper is the classic ice–albedo argument: it finds that small changes in
atmospheric transparency could be enough for glaciations once the planetary albedo responds to the ice.
*Bearing:* the temperature model for the bake: one diffusive equation over latitude (or over the
cube-sphere cells) with insolation in, albedo from ice and land cover, and outgoing radiation linear in
temperature — seconds of CPU, deterministic, and it gives ice caps and snow lines that respond to a
planet's tilt. Land–sea contrast and seasonality need the heat capacity per cell, not a new model.

**Ronald B. Smith, Idar Barstad. "A Linear Theory of Orographic Precipitation." *Journal of the
Atmospheric Sciences* 61(12), 2004, 1377–1391; G. H. Roe. "Orographic Precipitation." *Annual Review of
Earth and Planetary Sciences* 33, 2005, 645–671.** [paper] [code] [still-current]
<https://journals.ametsoc.org/view/journals/atsc/61/12/1520-0469_2004_061_1377_altoop_2.0.co_2.xml> —
<https://earthweb.ess.washington.edu/roe/Publications/Roe_OrogPrec_AnnRev05.pdf> — implementation:
<https://github.com/pism/LinearTheoryOrographicPrecipitation> (QGIS plugin by Aschwanden and Khrulev,
v3.1, 2020)

Smith and Barstad extend the classic "upslope" model with airflow dynamics, the advection of condensed
water and evaporation on the lee side, and solve the steady, vertically integrated equations with
Fourier transforms; the result is the terrain's spectrum multiplied by a wavenumber-dependent transfer
function, governed by five length scales (mountain width, a buoyancy-wave scale, the moist layer depth
and two advection distances). Roe's review places it among the other orographic models and explains
why orography produces some of the sharpest climate gradients on Earth. The PISM plugin computes it on
a DEM and is described as fast on large rasters given enough memory.
*Bearing:* the rain-shadow model for Forge's regional climate: one FFT of a terrain tile, one multiply,
one inverse FFT per wind direction and month — a compute pass, deterministic, and it gives windward
wet slopes and lee deserts at kilometre resolution from the same terrain the player walks on.

**Adiv Paradise, Evelyn Macdonald, Kristen Menou, Christopher Lee, Bo Lin Fan. "ExoPlaSim: Extending
the Planet Simulator for exoplanets." *Monthly Notices of the Royal Astronomical Society* 511(3), 2022,
3272–3303; with hersfeldtn, `koppenpasta` (GPL-3.0).** [paper] [code] [recent]
<https://arxiv.org/abs/2107.07685> — docs: <https://exoplasim.readthedocs.io/en/stable/> —
<https://github.com/hersfeldtn/koppenpasta>

A modified PlaSim general circulation model for synchronously rotating planets, non-solar spectra
and non-Earth surfaces, installed with `pip`, distributed under the GPL, validated qualitatively against
ExoCAM, LMDG and ROCKE-3D, fast enough for parameter surveys of hundreds to thousands of runs; tested
mostly at T21 (32 × 64) and T42, with netCDF among its outputs. `koppenpasta` converts a greyscale
heightmap to ExoPlaSim's topography format and turns the output into eleven classification maps,
including Köppen–Geiger, Trewartha, Holdridge, Thornthwaite–Feddema, BIOME1 and Whittaker.
*Bearing:* not something to put in the engine — it is a GPL-licensed climate model behind a Python
API — but the *oracle* for Forge's own bake: run it offline on a generated planet's heightmap,
classify with `koppenpasta`, and diff the zone maps against Forge's fast energy-balance-plus-moisture
bake. That is the golden-image test of a climate.

**S. E. Fick, R. J. Hijmans. "WorldClim 2: new 1-km spatial resolution climate surfaces for global
land areas." *International Journal of Climatology* 37(12), 2017, 4302–4315; with H. Hersbach et al.
"The ERA5 global reanalysis." *Quarterly Journal of the Royal Meteorological Society* 146(730), 2020,
1999–2049.** [paper] [still-current]
<https://www.worldclim.org/data/worldclim21.html> (DOI 10.1002/joc.5086) —
<https://rmets.onlinelibrary.wiley.com/doi/10.1002/qj.3803>

WorldClim is the standard *climate normals* product: monthly minimum, maximum and mean temperature,
precipitation, solar radiation, vapour pressure and wind speed for 1970–2000 at about 1 km, interpolated
from 9,000–60,000 stations with thin-plate splines and satellite covariates. ERA5 is the matching
*weather* product: hourly global three-dimensional fields at about 31 km.
*Bearing:* the two data shapes Forge should copy. A baked planet climate is WorldClim's shape — twelve
months of a handful of fields — and the live weather is ERA5's shape at a coarser grid. WorldClim's
field list is a good default for what a climate cell stores.

### What planet and map generators actually do

**Worldbuilding Pasta. "An Apple Pie From Scratch, Part VIa: Climate: Global Forcings" (March 2020),
"Part VIb: Climate: Biomes and Climate Zones" (May 2020) and "Part VI Supplement: Climate: Modeling
Climate with ExoPlaSim" (November 2021).** [web] [recent]
<https://worldbuildingpasta.blogspot.com/2020/03/an-apple-pie-from-scratch-part-via.html> —
<https://worldbuildingpasta.blogspot.com/2020/05/an-apple-pie-from-scratch-part-vib.html> —
<https://worldbuildingpasta.blogspot.com/2021/11/an-apple-pie-from-scratch-part-vi.html>

A thorough worldbuilder's account of planetary climate. VIa explains the three circulation
cells — trade winds from the Hadley cell, mid-latitude westerlies from the Ferrel cell, polar easterlies —
and the pressure belts they imply. VIb turns that into a seven-step method: sketch deserts and forests
from the cells, simulate seasonal temperature, correct for ocean currents and mountains, divide into
climate bands, map winds and fronts with the seasonal shift of the ITCZ, layer precipitation (warm
currents, convergence, fronts, orography, lee cyclogenesis), then assign Köppen zones. The supplement
runs ExoPlaSim on custom topography, from under ten minutes to over an hour a run, at T42.
*Bearing:* the checklist for Forge's fast bake — each of the seven steps is a pass, and each is a
place where a noise-based generator goes wrong. The ITCZ's seasonal migration is the cheapest way to
get monsoons and wet/dry tropics, which Whittaker-style annual fields miss.

**Amit J. Patel. "Mapgen4" (Red Blob Games, 2018, source on GitHub) and "Mapgen4: rainfall" (15
September 2018); with Azgaar, "Biomes generation and rendering" (30 June 2017).** [web] [code]
[still-current]
<https://www.redblobgames.com/maps/mapgen4/> — <https://simblob.blogspot.com/2018/09/mapgen4-rainfall.html>
— <https://github.com/redblobgames/mapgen4> —
<https://azgaar.wordpress.com/2017/06/30/biomes-generation-and-rendering/>

Mapgen4 sorts its Voronoi regions by their projection on the wind direction and visits them in that
order: moisture is copied from upwind neighbours, the air's capacity falls with elevation (∝ 1 −
elevation), and whatever the mountains squeeze out falls as rain — so rain shadows appear behind every
range without a fluid solver, and rivers follow the rainfall. Patel is candid that the straight-line
wind is fast but produces artefacts. His earlier mapgen2 used distance to coast and to water instead.
Azgaar's generator reads biomes from a rectangular temperature × moisture matrix (about 20 biomes)
rather than Whittaker's triangle, because a rectangle is easier to program and rescale.
*Bearing:* the moisture sweep to copy for Forge's coarse bake: order cells along the prevailing wind
of each latitude band (from the circulation cells), carry humidity, rain out on ascent. It is one pass
per wind direction and month, and Smith–Barstad then adds kilometre detail locally.

**Bay 12 Games. "Biome" and "Advanced world generation." Dwarf Fortress Wiki (v53.16).** [docs]
[still-current]
<https://dwarffortresswiki.org/index.php/Biome> —
<https://dwarffortresswiki.org/index.php/Advanced_world_generation>

Dwarf Fortress's world generator exposes the recipe as parameters: fractal fields for rainfall,
temperature and drainage, each with a minimum, a maximum and variances; weighted `*_FREQUENCY`
histograms that control the proportions of those and of elevation, volcanism and savagery; a `POLE`
setting that ties temperature to latitude; and an `OROGRAPHIC_PRECIPITATION` toggle that lets terrain
height affect rainfall. The biome is decided by
elevation first, then by drainage and rainfall together, with temperature choosing cold and tropical
variants.
*Bearing:* the proof that *drainage* deserves to be a first-class field beside rainfall — it separates
swamp from forest under the same rain — and Forge gets it free from the flow accumulation of the
hydrology bake (procedural.md §1).

**Mindwerks. WorldEngine (Python, MIT, v0.20.0).** [code] [still-current]
<https://github.com/Mindwerks/worldengine>

An open world generator that chains plate simulation, noise, precipitation with latitude and rain
shadows, erosion, humidity with terrain permeability, and biome classification by Holdridge life zones,
with rivers and cartographic outputs. Small, readable and still maintained.
*Bearing:* a readable reference implementation of the whole chain in one repository; worth reading for
the order of operations and the permeability term, not for vendoring.

**Jonathan Hill (JonathanCRH). Undiscovered Worlds and Undiscovered Worlds Classic (C++, GPL-3.0).**
[code] [recent]
<https://github.com/JonathanCRH/Undiscovered_Worlds_Classic> —
<https://github.com/JonathanCRH/Undiscovered_Worlds>

Generates roughly Earth-like planets with temperature, precipitation, Köppen climate zones and rivers,
then zooms into regional maps at about one pixel per kilometre; the newer version builds spherical
globes but has no regional maps yet. The README is unusually honest: climate regions come out "more
jumbled together than they should be", monsoon climates are over-represented and savanna and
continental climates under-represented.
*Bearing:* the global-then-regional structure is exactly Forge's (coarse bake, local refinement), and
its stated failures are the test cases for Forge's bake: count the share of each Köppen class and look
for speckle where there should be belts.

**Roland Fischer, Philipp Dittmann, René Weller, Gabriel Zachmann. "AutoBiomes: procedural generation
of multi-biome landscapes." *The Visual Computer* 36(10–12), 2020, 2263–2272 (CGI 2020).** [paper]
[recent]
<https://cgvr.cs.uni-bremen.de/papers/cgi20/AutoBiomes.pdf> (DOI 10.1007/s00371-020-01920-7)

An academic pipeline that goes all the way from terrain → climate → biomes → detail → assets.
Temperature by a latitude-like gradient with an altitude falloff; wind from an iterative, simplified semi-Lagrangian
scheme seeded at the corners; moisture from water cells, evaporated as a function of temperature and
carried downwind, raining out as it cools so rain shadows form; biomes from a discretised Whittaker
table that can be swapped. Biome borders are distorted with fractal noise on a finer grid, and each
biome's example DEM is blended in by a convolution kernel weighted by the biome areas inside it; assets
are placed by a local-to-global rule model. Timings: milliseconds for temperature, up to 36.6 s for
precipitation at the largest grid, O(n³) in cells per axis.
*Bearing:* the closest published analogue of what Forge needs that this search found, and its costs
say where to be smarter: the wind and moisture iteration is the expensive part, so Forge should derive prevailing winds from
the circulation cells rather than iterate them, and use the mapgen4 sweep for moisture.

---

## 2. Biomes and their transitions

A transition between biomes is an ecotone, and ecology has a clear account of why some are a
kilometre-wide gradient and others a line. The games and tools below implement the two halves of that
account — a soft blend in climate space and sharp local rules — in different proportions. Placement
systems that consume biome weights (Horizon's GPU placement, large-worlds.md §5; Unreal's PCG with
Biome Core, vegetation-materials.md §1; Far Cry 5's Houdini pipeline, procedural.md §2; WorldBrush's
statistical palettes, procedural.md §3) and texture blending (Mikkelsen's hex-tiling and Wronski's
Laplacian blending, procedural.md §5; mesh-to-terrain blending through a virtual texture,
vegetation-materials.md §6) are referenced, not repeated.

**Paul G. Risser. "The Status of the Science Examining Ecotones." *BioScience* 45(5), 1995, 318–325;
J. B. Wilson, A. D. Q. Agnew. "Positive-feedback switches in plant communities." *Advances in
Ecological Research* 23, 1992, 263–336.** [paper] [foundational]
<https://academic.oup.com/bioscience/article-abstract/45/5/318/252612> —
<https://www.sciencedirect.com/science/article/abs/pii/S006525040860149X>

Risser frames ecotones as dynamic zones of steep gradient between more homogeneous vegetation.
Wilson and Agnew explain the sharp ones: a community modifies its environment — water, pH, soil,
light, temperature, wind, fire, allelopathy — in a way that favours itself, so a smooth external
gradient produces an abrupt boundary and stable mosaics, and succession is accelerated or delayed.
*Bearing:* the rule for Forge's transitions. Blend width follows the *driver*: where the boundary is
set by climate alone (latitude, altitude), blend over kilometres; where a switch operates (forest
edge, treeline, bog, fire scar, water table), make the boundary sharp and let the local field decide
the side. So the biome weight needs a soft climate term *and* a hard local term, not one noise.

**Mojang. "World generation" and "Biome." Minecraft Wiki (Java Edition 1.18+).** [docs]
[still-current]
<https://minecraft.wiki/w/World_generation> — <https://minecraft.wiki/w/Biome>

The Overworld's multi-noise biome source reads six parameters — temperature, humidity (vegetation),
continentalness, erosion, weirdness (ridges) and depth — the first five from horizontal position only,
and gives each biome intervals in that six-dimensional space; where a point falls outside every
interval, the closest one wins. Each biome carries a temperature and a downfall value: below a base
temperature of 0.15 it can snow at any height, and from height 81 up the temperature drops by 1/800
per block, so snow lines appear on mountains in warm biomes. A "biome blend" option averages grass,
foliage and water colours over a square around each block.
*Bearing:* two ideas worth keeping and one to avoid. Keep the nearest-point selection in a climate
space (it gives smooth, controllable borders and admits new biomes without redrawing a map) and the
lapse-rate snow line. Avoid the square colour average: it is the grid artefact the next entry fixes.

**KdotJPG. "Fast Biome Blending, Without Squareness." NoisePosti.ng, 13 March 2021; code:
Scattered-Biome-Blender.** [web] [code] [recent]
<https://noiseposti.ng/posts/2021-03-13-Fast-Biome-Blending-Without-Squareness.html>

Full-resolution blurring of a biome map is exact but slow; grid interpolation is fast but leaves
borders on grid lines and regularly spaced creases. The fix is sparse convolution over a *jittered*
triangular grid: query the biome at scattered points, give each a smooth circular falloff such as
`max(0, r² − d²)²`, and normalise the weights per position because their sum varies.
*Bearing:* the kernel to use when Forge converts discrete biome ids into continuous weights for the
layer map (D-028) and for scatter densities — cheap, isotropic, and the radius is the ecotone width,
so it can vary per biome pair, as the ecotone entry above implies.

**Andrey Mishkinis. "Advanced Terrain Texture Splatting." Gamasutra / Game Developer, 16 July 2013.**
[web] [still-current]
<https://www.gamedeveloper.com/programming/advanced-terrain-texture-splatting>

Height-based blending: each layer's texture stores a height in alpha, and the layer whose height plus
blend weight is greater wins, with a small depth window blended rather than a hard switch — so sand
settles into the gaps between stones instead of fading over them.
*Bearing:* the texel-scale half of a biome transition, and the "height per layer" D-028 left for
later: climate decides the weights over kilometres, height blending decides which material shows in
each crack. It is also how snow and puddles should meet the ground (§4).

**Hello Games. No Man's Sky — "Biome" (No Man's Sky Wiki, Miraheze, Worlds Part II, 5.50), "Worlds
Part I Update" (July 2024) and "Worlds Part II Update" (2025).** [web] [docs] [recent]
<https://nomanssky.miraheze.org/wiki/Biome> — <https://www.nomanssky.com/worlds-part-i-update/> —
<https://www.nomanssky.com/worlds-part-ii-update/>

The contrast case: "each planet has a single biome to encourage exploration", with weather and hazards
following from the biome. Worlds Part I rebuilt the volumetric clouds (cirrus to rain-laden nimbus,
coverage varying over time with the weather), reworked the wind so grass and trees respond to storms,
and made water respond to weather and depth; Worlds Part II added gas giants, water worlds with oceans
kilometres deep, and localised hazards. The continuous generation architecture is procedural.md §3.
*Bearing:* one biome per planet is a design choice Forge must *not* inherit — the owner asked for
Earth-like planets with transitions — but NMS's later updates show where players notice weather: wind
in vegetation, cloud variety, water state, all driven by one planetary state.

**SideFX. Labs Biome tools (Houdini 20.5+): "Biome Demo" and "Labs Biome Plant Scatter" node
documentation.** [docs] [recent]
<https://www.sidefx.com/contentlibrary/biome-demo/> —
<https://www.sidefx.com/docs/houdini/nodes/sop/labs--biome_plant_scatter-1.0.html>

A production tool built on the ecological model: biome regions place initial environmental attributes
on a heightfield, the terrain "evolves" them (rain shadow, wind exposure, soil quality, temperature
with elevation), each plant definition states ideal conditions and tolerances for temperature,
precipitation and soil, a viability function produces a viability layer per species that acts as its
density map, and nearby plants then compete, keeping the oldest, the most viable or a random one.
*Bearing:* the pipeline Forge should implement in `forge-procgen`, almost node for node: climate fields
→ terrain-modulated attributes → per-species viability → density → competition. It is BIOME1's idea
(tolerances, then dominance) in a production tool, with no biome map painted anywhere.

**QuadSpinner. "Gaea 2.2 released!" (14 July 2025); World Creator, "Biome" (2025.x documentation);
World Machine, "Features".** [docs] [recent]
<https://blog.quadspinner.com/gaea-2-2-released/> — <https://docs.world-creator.com/reference/terrain/biome>
— <https://www.world-machine.com/features.php>

The terrain tools' view. Gaea 2.2 adds snow simulated in cascades so new and old snow interact
(Snowfield), glaciers and ice floes, orographic and selective precipitation in its GPU erosion (rain
limited by mask, slope or altitude), and a Trees node that layers three "biotopes". World Creator
groups filters, materials, objects and simulations into biomes, blends them by filter blending or
overwrites lower ones. World Machine models snowfall, melt and wind drifts and emits flow, wear, talus,
sediment and snow masks for texturing and placement.
*Bearing:* what artists will expect from Forge's editor: masks from simulation (flow, deposition, snow)
as first-class layers, precipitation that respects orography, and biomes as bundles of material,
scatter and simulation rules. None of them models climate globally; that is Forge's part.

**Xbox Wire. "From a Living Desert to a Volcano's Peak: Exploring Forza Horizon 5's Biomes and Seasons"
(26 July 2021); gamepressure.com, "Forza Horizon 5 weather system explained by devs" (reporting a
Playground Games stream of 29 June 2021).** [web] [recent]
<https://news.xbox.com/en-us/2021/07/26/exploring-the-biomes-and-seasons-of-forza-horizon-5/> —
<https://www.gamepressure.com/newsroom/forza-horizon-5-weather-system-explained-by-devs/z034fc>

A shipped game where climate is a table of biome × season: eleven biomes, each passing through the
seasons differently — spring is the rainy season in the jungle and farmland, summer brings tropical
thunderstorms to the coasts, a lake in the arid hills dries up in winter — with dust storms and
tropical storms visible from afar. The developers report matching weather to about 300 sky variants in
Forza Horizon 4 and more than 2,000 in the fifth game.
*Bearing:* the design target for "weather transitions" in a game: weather is not uniform noise but a
distribution conditioned on biome and season, and storms are *objects* that travel and can be seen
coming. Forge's climate atlas plus a travelling weather function gives the same, per cell instead of
per hand-authored region.

---

## 3. Ecosystems: flora, succession, fauna

The plant-ecosystem papers that matter most are already in procedural.md §4 — Deussen et al. 1998 (the
distribution → plant → rendering pipeline), Lane and Prusinkiewicz 2002 (local-to-global and
global-to-local distributions), Pałubicki 2009 and Pirk 2012 (trees that respond to light and
neighbours), Makowski et al. 2019 *Synthetic Silviculture* (a per-plant simulation that scales, with
parameter sets for nine ecologies), Pałubicki et al. 2022 *Ecoclimates* (vegetation–atmosphere
coupling), Kapp et al. 2020 (canopy and understorey fitted to learned fields) and Gain et al. 2017
*EcoBrush* (authoring over a simulation) — and Cordonnier et al. 2017 couples erosion and vegetation
(procedural.md §1). This section adds the runtime-deterministic placement papers, the ecology of
succession and disturbance, seasons, and fauna.

**J. Hammes. "Modeling of Ecosystems as a Data Source for Real-Time Terrain Rendering." In *Digital
Earth Moving*, LNCS 2181, Springer, 2001, 98–111.** [paper] [foundational]
<https://link.springer.com/chapter/10.1007/3-540-44818-7_14>

An early paper written for exactly Forge's constraint: known ecosystem-modelling techniques applied to
place plants on a terrain *at run time*, with algorithms fast enough for real-time computation and
deterministic, so the placement never has to be stored.
*Bearing:* the argument, from 2001, that ecosystem rules can be evaluated at streaming time rather than
baked into data — which is what Horizon's GPU placement (large-worlds.md §5) later shipped.

**Benny Onrust, Rafael Bidarra, Robert Rooseboom, Johan van de Koppel. "Ecologically Sound Procedural
Generation of Natural Environments." *International Journal of Computer Games Technology* 2017,
7057141.** [paper] [still-current]
<https://research.tudelft.nl/en/publications/ecologically-sound-procedural-generation-of-natural-environments/>
(DOI 10.1155/2017/7057141)

A graphics group and a spatial-ecology group together: landscape maps and ecological statistical data
are translated into plant distributions by combining procedural ecosystem generation with neutral
landscape models, rendered interactively on the web with standard LOD and lighting, and validated with
ecologists on two case studies.
*Bearing:* the evidence that *statistics per landscape class* (occurrence and spatial pattern) are
enough for ecologically plausible placement; Forge's per-biome species tables should store exactly
those, and an ecologist-style check (does this look like the place?) belongs in the demo review.

**H. K. M. Bugmann. "A Review of Forest Gap Models." *Climatic Change* 51, 2001, 259–305; Harald
Bugmann, Rupert Seidl. "The evolution, complexity and diversity of models of long-term forest
dynamics." *Journal of Ecology* 110(10), 2022, 2288–2307; after D. B. Botkin, J. F. Janak, J. R. Wallis, "Some ecological consequences of a
computer model of forest growth." *Journal of Ecology* 60, 1972, 849–872 (JABOWA).** [paper]
[foundational] [still-current]
<https://link.springer.com/content/pdf/10.1023/A:1012525626267.pdf> —
<https://besjournals.onlinelibrary.wiley.com/doi/10.1111/1365-2745.13989>

Gap models are individual-tree models of long-term forest dynamics. Their founding assumptions, as
Bugmann's review lists them, include a forest abstracted as many small patches, each of which can have
a different age and successional stage, with trees inside a patch not positioned. The 2001 review covers
those assumptions, the parent model JABOWA (Botkin et al. 1972) and thirty years of variants used to
study the effects of climate change on forest structure, biomass and composition; the 2022 review
follows the family's diversification since.
*Bearing:* the succession model for Forge's bake: per-patch stand state (age, dominant species group,
time since disturbance) evolved in decade steps from the climate, so a region is generated as a forest
of the right *age structure* (young birch on the burn, old spruce beyond) rather than as a uniform
climax. Run it once per region at generation; never per frame.

**Adrien Peytavie, James Gain, Eric Guérin, Oscar Argudo, Eric Galin. "DeadWood: Including Disturbance
and Decay in the Depiction of Digital Nature." *ACM Transactions on Graphics* 43(2), Article 21, 2024
(SIGGRAPH 2024).** [paper] [code] [recent]
<https://dl.acm.org/doi/10.1145/3641816> — code (MIT): <https://github.com/oargudo/deadwood>

Disturbance — fire, windstorm, disease — and decay produce standing dead trees and fallen logs, and
their absence is one reason simulated forests look artificial. The implementation extends Kapp et al.'s
ecosystem simulation and takes a heightfield, a biome database, per-species allometric growth, climate
parameters, *monthly environmental grids of sunlight, temperature and wetness*, and optional disturbance
schedules; it instances snags and logs by decay stage to keep memory bounded.
*Bearing:* both a technique (snags and logs as a function of disturbance history, instanced) and a
confirmation of the data model: a current ecosystem simulator asks for exactly the monthly grids
Forge's climate bake will produce.

**Richard C. Rothermel. "A mathematical model for predicting fire spread in wildland fuels." USDA Forest
Service Research Paper INT-115, 1972; Torsten Hädrich, Daniel T. Banuti, Wojtek Pałubicki, Sören Pirk,
Dominik L. Michels. "Fire in Paradise: Mesoscale Simulation of Wildfires." *ACM Transactions on
Graphics* 40(4), Article 163, 2021.** [paper] [foundational] [recent]
<https://research.fs.usda.gov/treesearch/32533> — <https://dl.acm.org/doi/10.1145/3450626.3459954>
(project page: <https://computationalsciences.org/publications/haedrich-2021-wildfires.html>)

Rothermel's model predicts rate of spread and intensity for a wide range of wildland fuels and became
the basis of the US National Fire Danger Rating System. *Fire in Paradise* simulates the combustion of
individual trees and the resulting spread at forest scale at interactive rates.
*Bearing:* fire is where weather, ecosystems and gameplay meet — dryness and wind from the weather
state, fuel from the ecosystem, scars that restart succession. Rothermel's spread rate on a cell grid
is the gameplay-scale model; *Fire in Paradise* is the per-tree reference for a hero fire, not a
system to run on a planet.

**W. M. Jolly, R. Nemani, S. W. Running. "A generalized, bioclimatic index to predict foliar phenology
in response to climate." *Global Change Biology* 11, 2005, 619–632; Norishige Chiba, Ken Ohshida,
Kazunobu Muraoka, Nobuji Saito. "Visual Simulation of Leaf Arrangement and Autumn Colours." *The
Journal of Visualization and Computer Animation* 7(2), 1996, 79–93.** [paper] [foundational]
[still-current]
<https://onlinelibrary.wiley.com/doi/10.1111/j.1365-2486.2005.00930.x> (repository:
<https://scholarworks.umt.edu/ntsg_pubs/148/>) — DOI `10.1002/(sici)1099-1778(199604)7:2<79::aid-vis139>3.3.co;2-n`

Jolly et al.'s Growing Season Index combines three limits — day length, evaporative demand (vapour
pressure deficit) and low minimum temperature — into one daily number that tracks satellite greenness
(r > 0.8 against NDVI across nine ecosystems). Chiba et al. are the early graphics treatment of autumn
colour, driven mainly by an estimate of the sunlight each part of each leaf receives.
*Bearing:* seasons in Forge's vegetation should be a function, not a calendar: GSI computed from the
climate atlas and the date gives each species a phenophase (dormant, leaf-out, green, colouring,
bare), which the renderer turns into a leaf-atlas blend and a leaf density — and the same planet with
a different tilt gets different seasons for free.

**S. J. Phillips, R. P. Anderson, R. E. Schapire. "Maximum entropy modeling of species geographic
distributions." *Ecological Modelling* 190(3–4), 2006, 231–259.** [paper] [still-current]
<https://collaborate.princeton.edu/en/publications/maximum-entropy-modeling-of-species-geographic-distributions/>
(DOI 10.1016/j.ecolmodel.2005.03.026)

Maxent: a species' distribution estimated from presence-only records as the maximum-entropy
distribution over environmental variables consistent with the observed presences — a general-purpose
method with a simple, precise formulation that the authors argue suits species distribution
modelling.
*Bearing:* the shape of Forge's fauna habitat model, not the fitting: each animal species gets a
suitability function over the same fields plants use (temperature, moisture, biome weights, slope,
distance to water, canopy), normalised like a Maxent output, and population density follows it.

**Vito Volterra. "Fluctuations in the Abundance of a Species considered Mathematically." *Nature* 118,
1926, 558–560.** [paper] [foundational]
<https://www.nature.com/articles/118558a0>

Volterra's mathematical treatment of interacting species' abundances, the work that led to the
Lotka–Volterra predator–prey equations: populations as continuous quantities coupled by their
interactions.
*Bearing:* at planet scale, fauna should be *numbers per cell*, not agents — a per-species density per
climate cell, updated in game-day steps by growth, predation and hunting — with individuals
instantiated only near players (next entry). Keep the equations damped; undamped cycles look like bugs.

**Strange Loop Games. Eco (Steam store page, released 2018); theHunter: Call of the Wild, "All need zone
times for every animal" (Steam community guide by tiltaaa, 2019, updated 2024).** [web] [recent]
<https://store.steampowered.com/app/382310/Eco/> —
<https://steamcommunity.com/sharedfiles/filedetails/?id=1767037181>

Two game-scale fauna designs at opposite ends. Eco runs "a fully simulated ecosystem" of thousands of
plants and animals, where disrupting one species cascades across the planet. theHunter anchors each
species to *need zones* — feeding, drinking and resting areas it visits at species-specific times of
day — which players learn to read.
*Bearing:* Forge takes Eco's persistence at the population level (cell densities, above) and
theHunter's legibility at the individual level: animals spawned near players are placed at need zones
derived from the terrain (water, cover, forage from the ecosystem) at the times their species keeps.
Per-animal simulation across a planet is out of scope.

---

## 4. Weather rendering

Clouds, the froxel fog and the sky are in lighting-gi.md §6 (Nubis 2015/2022/2023, Wronski 2014,
Hillaire 2016/2020) and RDR2's integrated atmosphere in §3; Lagarde's rain parts 2b and 3a,
Barré-Brisebois's and Michels–Sikachev's snow deformation are in vegetation-materials.md §7; rain and
wind *sound* are in audio.md §2.5 and §4. What follows fills the gaps lighting-gi.md left open: rain,
wet surfaces, snow, lightning, fog density from weather, cloud authoring and transitions, and
simulated weather.

### Rain

**Kshitiz Garg, Shree K. Nayar. "Photorealistic rendering of rain streaks." *ACM Transactions on
Graphics* 25(3), 2006, 996–1002 (SIGGRAPH 2006).** [paper] [foundational]
<https://history.siggraph.org/learning/photorealistic-rendering-of-rain-streaks-by-garg-and-nayar/>

A falling raindrop oscillates, so the light it reflects and refracts during one exposure makes complex
brightness patterns inside a single motion-blurred streak. The paper models streak appearance as a
function of lighting direction, view direction and the oscillating drop shape (from an atmospheric-
science oscillation model); the precomputed streak textures, indexed by three angles, were published as
a database, which Tariq's sample (next entry) uses.
*Bearing:* the texture source for Forge's rain: a streak array indexed by light and view angle is what
makes rain glint in a street lamp and vanish against the sun, for the price of a texture lookup.

**Sarah Tariq. "Rain." NVIDIA Direct3D 10 SDK white paper, 2007.** [docs] [code] [foundational]
<https://developer.download.nvidia.com/SDK/10/direct3d/Source/rain/doc/RainSDKWhitePaper.pdf>

Rain as a GPU particle system: particles animated with stream-out, expanded into billboards, and shaded
from a texture array that encodes drop appearance under different view and light directions (Garg and
Nayar's database), so heavy rain responds to wind and to local lights — which, the white paper notes,
camera-centred scrolling textures struggle to do.
*Bearing:* the architecture to translate: compute-shader simulation instead of stream-out, mesh-shader
expansion instead of the geometry shader (which Forge never uses), the same streak array. Particle
count scales with the weather's rain rate.

**Natalya Tatarchuk. "Artist-Directable Real-Time Rain Rendering in City Environments." *SIGGRAPH 2006
course: Advanced Real-Time Rendering in 3D Graphics and Games*, chapter 3.** [talk] [foundational]
<https://advances.realtimerendering.com/s2006/index.html> — chapter:
<https://advances.realtimerendering.com/s2006/Chapter3-Artist-Directable_Real-Time_Rain_Rendering_in_City_Environments.pdf>

The ATI ToyShop demo's complete rain: image-space rainfall, particle drips and splashes, a GPU water
simulation for ripples on puddles, droplets on glass, view-dependent stretched reflections of lights on
wet ground, lightning illumination, and material shaders for the wet city.
*Bearing:* the inventory of effects a rainy street needs, still a complete checklist; Forge's
dusk-town storm front can be checked against it item by item.

**Sébastien Lagarde. "Water drop 1 – Observe rainy world" (10 December 2012), "Water drop 2a – Dynamic
rain and its effects" (27 December 2012), "Water drop 3b – Physically based wet surfaces" (14 April 2013),
and "Water drop 4a/4b – Reflecting wet world" (titles as linked from part 1; not summarised here).**
[web] [still-current]
<https://seblagarde.wordpress.com/2012/12/10/observe-rainy-world/> —
<https://seblagarde.wordpress.com/2012/12/27/water-drop-2a-dynamic-rain-and-its-effects/> —
<https://seblagarde.wordpress.com/2013/04/14/water-drop-3b-physically-based-wet-surfaces/>

The rest of the series whose parts 2b and 3a are in vegetation-materials.md §7. Part 1 lists what rain
does to a scene: stretched highlights, water pooling in hollows and cracks, puddles as a two-layer
surface, ripples with density proportional to intensity, splashes, darker diffuse and brighter
specular on porous materials, irregular drying where specular fades before the darkening, glow and fog
around lights, wind-slanted rain wetting horizontal surfaces more than vertical ones. Part 2a is
*Remember Me*'s rain: four camera-attached cylinders of pre-blurred streaks, a 256 × 256 depth map
rendered from above to occlude rain and to spawn splashes only where rain lands, intensity bands light
0.33 / moderate 0.66 / heavy 1.0, and a measured cost of about 1.7 ms on PS3 and Xbox 360. Part 3b gives
the wet-surface model: darkening and roughness from albedo, porosity and roughness, porosity from a
texture, a mask or estimated from roughness, metals left unchanged, water accumulation as a normal blend
towards a flat film, and drying along sigmoid curves.
*Bearing:* the wet-surface model is a per-material `porosity` (vegetation-materials.md §8 sketches it on
the D-007 row) plus the local wetness field; the depth map from above is the one structure that
should serve four consumers in Forge — rain occlusion, splash spawning, wetness exposure and snow
accumulation. The intensity bands are a sensible quantisation for audio and gameplay reads of the
rain rate.

**Yoann Weber, Vincent Jolivet, Guillaume Gilet, Djamchid Ghazanfarpour. "A multiscale model for rain
rendering in real-time." *Computers & Graphics* 50, 2015, 61–70; Y. Weber, V. Jolivet, G. Gilet,
K. Nanko, D. Ghazanfarpour. "A phenomenological model for throughfall rendering in real-time." *Computer
Graphics Forum* 35(4), 2016, 13–23.** [paper] [still-current]
<https://hal.archives-ouvertes.fr/hal-01295379> —
<https://diglib.eg.org/handle/10.1111/cgf12945>

The 2015 paper makes rain one coherent model across scales: visible streaks near the camera and the
progressive loss of visibility that rain causes further away. The 2016 paper renders what happens under
trees: raindrops intercepted by foliage drip from the leaves, modelled with a hydrological,
phenomenological model and evaluated per pixel with a closed form instead of simulating each drop.
*Bearing:* two things Forge would otherwise miss. Far rain is *extinction*, so the rain rate belongs in
the froxel volume (D-032) and the aerial perspective, not only in particles; and rain under a canopy
looks different — drops falling from leaves rather than from the sky — which the throughfall model
gives without simulating drops.

### Wet surfaces

**John Lekner, Michael C. Dorf. "Why some things are darker when wet." *Applied Optics* 27(7), 1988,
1278–1280; Henrik Wann Jensen, Justin Legakis, Julie Dorsey. "Rendering of Wet Materials." *Eurographics
Workshop on Rendering* 1999, 273–282.** [paper] [foundational]
<https://opg.optica.org/ao/abstract.cfm?uri=ao-27-7-1278> —
<http://graphics.ucsd.edu/~henrik/papers/rendering_wet_materials/>

Lekner and Dorf explain the darkening. Ångström had proposed that light diffusely reflected by a
rough surface is partly totally internally reflected by the water film on it and so gets more chances
to be absorbed; they compute that probability more accurately and add the effect of the lower relative
refractive index (water to material instead of air to material) on absorption — both lower the albedo
of the wet surface, in good agreement with experiment. Jensen et al. render wet materials by combining a
surface-water reflection model with subsurface scattering, and show wet materials can look darker,
brighter or more specular depending on material and view.
*Bearing:* the physics under Lagarde's practical model; enough to justify one per-material `porosity`
and to reject the tempting global "multiply albedo by 0.5 when wet".

### Snow

**Per Ohlsson, Stefan Seipel. "Real-time Rendering of Accumulated Snow." *SIGRAD 2004*, Linköping
Electronic Conference Proceedings 13, 25–31; with Paul Fearing, "Computer modelling of fallen snow",
SIGGRAPH 2000, and Niels v. Festenberg, Stefan Gumhold, "Diffusion-Based Snow Cover Generation",
*Computer Graphics Forum* 30(6), 2011, 1837–1849.** [paper] [foundational]
<https://ep.liu.se/en/conference-article.aspx?series=&issue=13&Article_No=7> —
<https://history.siggraph.org/learning/computer-modelling-of-fallen-snow-by-fearing/> — DOI
10.1111/j.1467-8659.2011.01904.x

Ohlsson and Seipel compute snow per pixel at render time: a depth buffer rendered like a shadow map
tells how much snow a surface can receive, the amount is modulated by the surface slope, and 3-D noise
lights the snow surface. Fearing's offline model is the reference it approximates — an accumulation
model that traces flakes back to the sky, with flutter, dusting and wind — plus a stability model for
where snow stays; Festenberg and Gumhold model accumulation as diffusion to get bridges and overhangs.
*Bearing:* Forge's snow cover is Ohlsson–Seipel with the shared depth-from-above map and a *depth*
field instead of a boolean: exposure × slope × accumulated snowfall, where the accumulation comes from
the weather state and melt (§5). The offline papers are for the bake of permanent snowfields.

**Paolo Surricchio. "Reinventing the Wheel for Snow Rendering." *GDC 2023 Advanced Graphics Summit*
(Santa Monica Studio, God of War Ragnarök); interview in 80.lv, 16 October 2023.** [talk] [web] [recent]
<https://gdcvault.com/play/1028844/Advanced-Graphics-Summit-Reinventing-the> —
<https://80.lv/articles/santa-monica-s-senior-programmer-on-how-god-of-war-ragnar-k-s-snow-system-was-made>

God of War Ragnarök replaced screen-space parallax snow with geometric displacement and hardware
tessellation: artists tick a checkbox on a mesh, carving shapes attached to characters push the snow
down, runtime masks persist the trails, and the terrain is split into meshlets culled by indirect draws
so only the chunks that need it are tessellated — a mesh-shader-like path kept compatible with the base
PS4.
*Bearing:* the current production answer, and it maps onto Forge's cluster pipeline directly: snow
depth as displacement on the terrain clusters near the camera, with the deformation layer of D-007
subtracted. The deformation side itself is vegetation-materials.md §7–8.

### Lightning

**Todd Reed, Brian Wyvill. "Visual simulation of lightning." *SIGGRAPH '94*, 1994, 359–364; Theodore
Kim, Ming C. Lin. "Physically Based Animation and Rendering of Lightning." *Pacific Graphics 2004*.**
[paper] [foundational]
<https://history.siggraph.org/learning/visual-simulation-of-lightning-by-reed-and-wyvill/> —
<https://gamma.cs.unc.edu/LIGHTNING/>

Reed and Wyvill generate the channel path with a particle system aimed at aesthetic animation rather
than physics, and light struck objects with implicit surfaces. Kim and Lin use the dielectric
breakdown model for the branching pattern, a simplified Helmholtz equation for sustained arcs, and a
convolution kernel for the glow instead of Monte Carlo ray tracing.
*Bearing:* a bolt is a few hundred segments built once per strike: a Reed–Wyvill random walk (or a
small dielectric-breakdown grid for hero strikes) drawn as emissive ribbons, the glow left to bloom
(D-022), the flash a short-lived light inside the cloud layer — as Nubis 2022 does for its superstorms
(lighting-gi.md §6).

### Fog and clouds from the weather state

**Inigo Quilez. "Better fog." iquilezles.org.** [web] [still-current]
<https://iquilezles.org/articles/fog/>

Beyond distance fog: fog tinted towards the sun's colour when looking towards it, extinction and
in-scattering split with separate falloffs per channel, and height fog whose density falls
exponentially with altitude — with an analytic integral along the ray, so it costs no more than plain
fog.
*Bearing:* the far-field form of weather fog beyond the froxel volume (D-032): the weather state sets
the base density and height falloff (valley fog on a cold, still, humid morning), the analytic integral
extends it to the horizon.

**Andrew Schneider. "Nubis: Authoring Real-Time Volumetric Cloudscapes with the Decima Engine."
*SIGGRAPH 2017, Advances in Real-Time Rendering in Games* course.** [talk] [still-current]
<https://advances.realtimerendering.com/s2017/index.html>

The production follow-up to the 2015 prototype cited in lighting-gi.md §6: authoring cloudscapes at a
regional scale, their animation and *transitions*, integration into the atmosphere system, an improved
lighting model, Perlin–Worley noise generation and the weather simulation that drives them in Horizon
Zero Dawn.
*Bearing:* the missing link between Forge's weather state and its clouds: the clouds read a weather map
(coverage, type, precipitation) and transitions are changes of that map over time. Forge generates the
map from the weather function instead of painting it.

### Simulated weather

**Torsten Hädrich, Miłosz Makowski, Wojtek Pałubicki, Daniel T. Banuti, Sören Pirk, Dominik L. Michels.
"Stormscapes: Simulating Cloud Dynamics in the Now." *ACM TOG* 39(6), 2020; Jorge Alejandro Amador
Herrera, Torsten Hädrich, Wojtek Pałubicki, Daniel T. Banuti, Sören Pirk, Dominik L. Michels.
"Weatherscapes: Nowcasting Heat Transfer and Water Continuity." *ACM TOG* 40(6), Article 204, 2021;
Jorge Alejandro Amador Herrera, Jonathan Klein, Daoming Liu, Wojtek Pałubicki, Sören Pirk, Dominik L.
Michels. "Cyclogenesis: Simulating Hurricanes and Tornadoes." *ACM TOG* 43(4), 2024.** [paper] [recent]
<https://dl.acm.org/doi/10.1145/3414685.3417801> —
<https://computationalsciences.org/publications/amador-herrera-2021-weatherscapes.html> —
<https://computationalsciences.org/publications/amador-herrera-2024-cyclogenesis.html>

The same group as Ecoclimates and Fire in Paradise, building weather from first principles. Stormscapes
derives buoyancy and pressure to simulate cumulus, stratus and stratocumulus and thunderstorm
supercells, up to about 20 km × 20 km at interactive rates. Weatherscapes couples atmosphere and soil:
rain, snow and graupel from microphysics, run-off, infiltration and evaporation, daily heating, Foehn
winds, validated against infrared satellite images. Cyclogenesis adds hurricanes and tornadoes with
heat and water continuity and boundary-layer dynamics, compared with storm soundings.
*Bearing:* the ceiling, not the plan. Their domains are tens of kilometres, which is the size of Forge's
*regional* weather window, so a later "hero storm" could run such a model locally, seeded from the
planet's state. The planetary layer stays analytic (Recommendation).

**Jacques Kerner. "Aerodynamics of Just Cause 4." Game Developer, 22 February 2019.** [web] [recent]
<https://www.gamedeveloper.com/programming/aerodynamics-of-just-cause-4>

Just Cause 4's tornado is not a fluid simulation: it is a stack of horizontal cylinders centred on a
spline that deforms over time, with tangential, radial and vertical wind components shaped by tunable
curves, and objects feel drag from a precomputed per-shape model stored in cube maps, found through a
sparse spatial database with bounding-box early-outs.
*Bearing:* the gameplay-grade wind for Forge's extreme weather: analytic wind fields as events layered
on the weather state, forces on Jolt bodies from a per-shape drag table, no grid solver.

---

## 5. The state model

How the environment is represented and streamed: the global field, the local detail, the rules that
update the surface, and how the result reaches gameplay. The render-side structures (weather map,
surface clipmap) are in the Recommendation; this section is the evidence.

**meteoblue. "How meteoblue Powers Weather in Microsoft Flight Simulator" (business article).** [web]
[recent]
<https://business.meteoblue.com/articles/microsoft-flight-simulator-partnership-meteoblue>

The planet-scale weather state that has shipped: meteoblue's global model divides the Earth's surface
into about 250 million boxes, with 60 vertical layers from the surface to the stratosphere, and feeds
Microsoft Flight Simulator (Asobo, 2020) with forecasts, high-altitude winds, icing risk derived from
cloud microphysics and cloud shapes; the stated difficulty was delivering that much data in real time
without overwhelming the simulator.
*Bearing:* the pattern — a coarse global state, streamed, refined locally for rendering — confirmed at
Earth scale. Forge's global state is far coarser and generated rather than downloaded, which removes
the delivery problem: nothing crosses the network but a seed and a clock.

**William M. Putman, Shian-Jiann Lin. "Finite-volume transport on various cubed-sphere grids." *Journal
of Computational Physics* 227, 2007, 55–78; NOAA GFDL, "FV3: Grids".** [paper] [docs] [still-current]
<https://www.sciencedirect.com/science/article/abs/pii/S0021999107003105> —
<https://www.gfdl.noaa.gov/fv3/fv3-grids/>

Advection tests on gnomonic, conformal and numerically generated cubed-sphere grids; the gnomonic grid
became FV3's choice "due to its best grid uniformity". FV3 refines regions either by stretching one
cube face smoothly (refinement up to 80×, cells down to 500 m) or by two-way nests run concurrently
with the global grid, with improvements reported for orographic precipitation and the diurnal cycle.
*Bearing:* the global climate field lives on the same cube-sphere as Forge's terrain (large-worlds.md
§8; D-014) and inherits its cell ids and streaming; the "nest" is Forge's regional window, and the
reported gains are precisely the orographic rain and daily cycle Forge wants near the player.

**C. W. Richardson. "Stochastic simulation of daily precipitation, temperature, and solar radiation."
*Water Resources Research* 17(1), 1981, 182–190.** [paper] [foundational]
<https://agupubs.onlinelibrary.wiley.com/doi/abs/10.1029/wr017i001p00182>

The weather generator: precipitation occurrence as a Markov chain with exponentially distributed
amounts, and maximum and minimum temperature and solar radiation from a multivariate model whose means
and deviations depend on whether the day is wet or dry — long synthetic daily series with the right
statistics from a station's climate.
*Bearing:* the statistics Forge's weather must reproduce from its climate atlas — wet-day probability,
spell lengths, monthly totals, wet-day cooling — and the test that checks it: run thirty synthetic
years in a cell and compare with the atlas. The implementation can be a closed-form function of seed
and time (Recommendation); Richardson is the acceptance criterion.

**Jos Stam. "Stable Fluids." *SIGGRAPH '99*, 1999, 121–128.** [paper] [foundational]
<https://history.siggraph.org/learning/stable-fluids-by-stam/>

Semi-Lagrangian advection with implicit viscosity: unconditionally stable, so large time steps work,
with texture advection shown in 2-D and 3-D. AutoBiomes' wind step simplifies the same scheme, citing
Stam's game-oriented follow-up, "Real-Time Fluid Dynamics for Games" (GDC 2003).
*Bearing:* the tool for the parts of the state that must *move* on a grid — humidity and cloud fields in
the regional window, smoke from fires — at time steps of game-minutes without blowing up.

**Syukuro Manabe. "Climate and the ocean circulation: I. The atmospheric circulation and the hydrology
of the earth's surface." *Monthly Weather Review* 97(11), 1969, 739–774; Regine Hock. "Temperature index
melt modelling in mountain areas." *Journal of Hydrology* 282, 2003, 104–115.** [paper] [foundational]
<https://journals.ametsoc.org/view/journals/mwre/97/11/1520-0493_1969_097_0739_catoc_2_3_co_2.xml> —
<https://ui.adsabs.harvard.edu/abs/2003JHyd..282..104H/abstract>

Manabe's model predicted soil moisture and snow cover with the land as boxes holding a limited amount
of water — the "bucket" model that fills with rain, empties by evaporation and spills the excess as
run-off. Hock reviews temperature-index (degree-day) melt models, which relate melt to positive air
temperature through a degree-day factor and, despite their simplicity, often do as well as full energy
balances at catchment scale.
*Bearing:* the two update rules for Forge's surface state: ground moisture as a bucket per texel (rain
in, evaporation and drainage out, overflow as puddles by the flow map) and snow depth by degree-days
(snowfall in below the rain–snow temperature, melt out at a factor per degree above zero). Both are one
line of arithmetic per texel per step and both are what hydrologists actually use.

**Hidemaro Fujibayashi, Satoru Takizawa, Takuhiro Dohta. "Change and Constant: Breaking Conventions with
'The Legend of Zelda: Breath of the Wild'." *GDC 2017*; report: Daniel New, Thumbsticks, 2 March 2017.**
[talk] [web] [still-current]
<https://www.gdcvault.com/play/1024562/Change-and-Constant-Breaking-Conventions> —
<https://www.thumbsticks.com/gdc-17-breath-of-the-wild-science-lies/>

The "chemistry engine": a small set of rules by which elements — fire, ice, electricity, wind — change
the state of materials and of each other, and exert forces of their own (wind pushes objects), so
behaviour emerges from rules rather than from scripted cases.
*Bearing:* the gameplay reading of the weather state is the same design: weather is an element source
(rain wets, cold freezes, wind pushes, lightning strikes), materials change state through D-007's
overrides, and no system names a specific puddle or icicle.

---

## Recommendation for Forge

*Opinion, shaped by the project's constraints — whole planets, deterministic multiplayer (D-016,
D-010), hardware ray tracing required for players (D-008), one material row for every system (D-007) —
rather than by citation count. The sizes and costs below are estimates, not measurements.*

**The state model, in one paragraph.** D-019's "one struct" survives as the *sample* every system reads,
but it is backed by three fields at three scales: a **climate atlas** baked once per planet, a
**weather function** that is a pure function of the atlas, the planet's seed and world time, and a
**surface state** (wetness, puddles, snow, frost) integrated only around players and re-derivable
anywhere from recent weather. Nothing continuous is replicated: a client and the server agree on the
weather because they evaluate the same function (D-016), and only authored or gameplay-caused weather
events travel as reliable events (D-010).

**1. The climate atlas (baked, per planet, `forge-procgen::climate`).**

| Layer | Resolution (Earth-sized planet) | Contents | Size |
|---|---|---|---|
| Global | cube-sphere level 7 — 6 × 128² cells of about 78 km | per cell and month: mean temperature, diurnal range, precipitation, wet-day probability, prevailing wind (2 components), relative humidity, cloud fraction — WorldClim's field list, 16 bytes | ≈ 19 MB for twelve months, fully resident |
| Regional | level 13 — about 1.2 km, computed per level-6 tile (156 km, 128² cells) when the tile is first generated, cached on disk | the same twelve months, downscaled: temperature by the lapse rate (about 6.5 K per km) against the tile's real heights, precipitation by Smith–Barstad over the tile's terrain with that month's wind and humidity | ≈ 3 MB per tile |
| Derived, on demand | per regional cell | coldest-month temperature, degree-days above 5 °C, moisture index (P/PET), Köppen class (debug view only) | computed, not stored |

The bake, in order: insolation per latitude and day from the planet's obliquity and orbit (Berger;
Nadeau–McGehee for the annual mean) → a diffusive energy balance with land/sea heat capacity and
ice–albedo feedback (Budyko; North) for monthly temperature → prevailing winds from the three
circulation cells with the ITCZ following the sun (Worldbuilding Pasta VIa–b) → a moisture sweep along
the wind with rain-out on ascent (mapgen4, AutoBiomes) for the global layer → Smith–Barstad for the
regional layer. It is deterministic, runs in seconds on the CPU for the global layer, and each regional
tile is one FFT pass. **Validation** is offline and part of the demo: run ExoPlaSim on the same planet's
heightmap, classify with `koppenpasta`, and compare the Köppen maps and class shares with Forge's bake
(and, for an Earth-like preset, with Peel 2007's shares); Undiscovered Worlds' stated failures — speckled
zones, too many monsoon cells — are the things to look for. Flat-grid worlds (the island) are one
regional tile with a latitude and a prevailing wind given as parameters.

**2. Biomes and transitions (baked with the terrain tiles).** Biomes are not painted and not a noise
field: a **biome table** (data) gives each biome a climate envelope on BIOME1's indices (coldest month,
degree-days, moisture index) plus Whittaker's two axes for the first version, and each biome row lists
its D-028 terrain layers, its species with viability curves, its fauna with habitat suitability, and a
**transition width**. Biome weights at a terrain point are a soft nearest-point selection in climate
space (Minecraft's rule, with a softmax instead of an argmin) blended by KdotJPG's scattered kernel with
the transition width as radius — kilometres where only climate changes — then *sharpened* by local
switches read from the terrain at metre scale: water table and flow accumulation (drainage, as Dwarf
Fortress insists), slope and soil depth, aspect, fire history (Wilson and Agnew). The result is written
where D-028 already expects data: a biome-weight map (top two biomes and a weight, RGBA8 at 4 m) beside
the layer map (1 m), whose per-texel material comes from the biome's layer rules with height blending
(Mishkinis) for the crack-scale transition. Vegetation needs no blending at all: each species' density
is its own viability × competition (Houdini Labs), so an ecotone is simply where two species lists
overlap.

**3. Ecosystems (baked per region, `forge-procgen::ecosystem`, Phase 8).** Species viability maps from
the regional climate and terrain; a gap-model succession (Bugmann) on 32 m patches in decade steps over
a few centuries, with disturbance from the climate (fire return from dryness and wind, windthrow on
exposed ridges) seeded per patch, gives each patch an age and a dominant group; canopy trees are then
fitted to the target density (Kapp 2020, procedural.md §4) and understorey and ground cover come from
the hash-based scatter (vegetation-materials.md §3); snags and logs by decay stage from the disturbance
history (DeadWood). At run time only two things evolve: **seasons**, as a per-species phenophase from
the Growing Season Index of the current date and climate (Jolly), read by the renderer as a leaf-atlas
blend, leaf density and snow load; and **player-caused disturbance** (felling, fire by Rothermel spread
on a cell grid), which re-enters the same succession rules in game-day steps. **Fauna** are population
densities per regional cell (habitat suitability in Maxent's shape × carrying capacity, predator–prey
coupling in damped Volterra steps per game day, decremented by hunting), instantiated as individuals
only within a few hundred metres of players at need zones (water, cover, forage) at their species'
times of day, as theHunter does. Per-plant simulation beyond the player's region and per-animal
simulation anywhere are out.

**4. The weather function (run time, `forge-world::weather`, Phase 3).** `weather.sample(position,
time) -> WeatherSample { temperature, humidity, wind, precipitation_rate, precipitation_kind,
cloud_cover, cloud_type, fog_density, lightning_potential, wetness, puddle, snow_depth, frost }` — the
D-019 struct, now a sample. The synoptic part is a closed form: low-frequency 3-D noise on the unit
sphere, rotated about the polar axis at the angular speed of each latitude band's prevailing wind, so
fronts and pressure systems travel the way the circulation cells say, thresholded per cell so that the
fraction of wet time and the monthly totals match the atlas; convective cells are a smaller-scale noise
with an afternoon peak over land; lightning strikes are a seeded Poisson process in the convective
cells, so their times and places are known to server and client alike. This scheme is Forge's own, not
a cited technique: it is justified by being deterministic and free to evaluate anywhere, and it is
accepted only if thirty synthetic years in a cell reproduce Richardson's statistics of the atlas
(wet-day probability, spell lengths, totals). Authored weather — a quest storm, a tornado — is an
*event* (centre, radius, start, duration, profile) blended over the function and replicated once; its
wind is an analytic field in Just Cause 4's manner. The server samples weather for gameplay at 1 Hz per
player cell; nothing is simulated planet-wide per tick.

**5. The surface state (run time, integrated locally).** Around each camera a clipmap of 3 levels ×
512² texels at 0.5, 2 and 8 m (256 m, 1 km, 4 km) holds wetness, puddle depth, snow depth and frost
(RGBA16, ≈ 6 MB), updated a few times a second on the GPU with Manabe's bucket (rain in, evaporation
and drainage out, overflow into puddles by the flow map) and Hock's degree-day snow (accumulate below the
rain–snow temperature, melt above zero). Exposure comes from the shared **depth-from-above map**
(Lagarde 2a; Ohlsson–Seipel) — one depth-only pass of the cluster pipeline from above over the camera's
surroundings, which also occludes rain particles and places splashes. Texels that scroll into the
clipmap are initialised by integrating the last 72 game-hours of the weather function at that point, so
there is never a dry square behind the camera; the server answers gameplay queries (is this ground
slippery, does this footprint fill with water) with the same integral at the point, cached per 2 m cell
per game-minute. The D-007 overrides (wet, frozen, snow) read these values; the deformation layer
stays D-007's and is refilled by snowfall and rain as specified there.

**6. How rendering consumes it (Phase 4, `forge-render::weather`).**
- **Weather map:** a camera-centred 256² window at 250 m (64 km), two RGBA16F images (≈ 1 MB) filled
  from the weather function each second or each texel of camera travel: cloud cover and type,
  precipitation rate and kind, wind at ground and cloud level, fog density, ground temperature. It is
  Nubis's weather map (2015/2017), generated instead of painted.
- **Clouds and sky:** the Nubis-style layer of lighting-gi.md §6 reads the weather map; cloud cover
  also enters the sky-view table and the sky's irradiance (D-023) and a top-down cloud-shadow map dims
  the sun on the ground.
- **Precipitation:** compute-simulated particles in a box of a few tens of metres around the camera,
  count proportional to the rain rate, expanded by the mesh shader and shaded from a Garg–Nayar streak
  array (Tariq's design without the geometry shader); snow the same with flakes and wind; beyond the
  box, rain is extinction in the froxel volume (D-032) and the aerial perspective (Weber 2015).
- **Surfaces:** the resolve reads the surface clipmap and the material row's porosity for Lagarde's
  darkening and roughness, puddles by height blending against the layer height and the flow map, ripple
  normals scaled by the rain rate, snow as a layer composited as a surface gradient and displaced on the
  terrain clusters near the camera (Surricchio's approach), frost as a roughness and albedo shift.
- **Vegetation:** phenophase per species (leaf-atlas blend, leaf density), snow load on canopies, the
  weather map's wind into the vegetation wind field (vegetation-materials.md §3), throughfall drips
  under canopies after rain (Weber 2016).
- **Lightning:** at each scheduled strike, a bolt of a few hundred segments (Reed–Wyvill walk) as
  emissive ribbons with bloom, a flash light in the cloud layer, and a thunder event to audio delayed by
  distance at 343 m/s.
- **Fog:** the froxel medium's density and height falloff from the weather map's fog field (humidity
  near saturation, still air, cold valleys), Quilez's analytic height fog beyond the froxel range.
- **Audio and gameplay:** the same `WeatherSample` drives rain and wind sound (audio.md §2.5, §4),
  surface sounds through the material overrides, wind forces on bodies, visibility for AI.

**Bake or simulate — the whole table.**

| Thing | Bake (at generation) | Simulate (at run time) |
|---|---|---|
| Insolation, temperature, winds, precipitation normals | global atlas, regional downscale | — |
| Biome weights, layer maps | with the terrain tile | — |
| Succession, stand age, snags | per region | player-caused disturbance only |
| Seasons (phenology) | — | function of date and atlas |
| Fauna | habitat suitability | cell densities per game day; individuals near players |
| Weather | — | closed-form function of atlas, seed, time; events |
| Wetness, puddles, snow, frost | permanent snowfields | clipmap near cameras; point integral elsewhere |
| Clouds, precipitation, fog, lightning visuals | — | every frame from the weather map |

**Phases.**
- **Phase 2 (World):** the climate bake and the biome weights, feeding D-028's layer maps. The `island`
  demo's ground comes from climate and terrain rather than from rules on slope alone, and its planet
  variant shows a biome map and a Köppen debug view compared with ExoPlaSim on the same heightmap.
- **Phase 3 (Simulation, materials, weather):** the weather function and `WeatherSample`, the surface
  state rules, D-007's overrides, determinism digests of weather samples at 1 and 6 workers (D-016). The
  `materials-yard` demo's wetness and snow come from a scheduled weather event over this state rather
  than from values set by hand.
- **Phase 4 (Lighting):** the weather map, clouds, precipitation, wet and snowy surfaces, lightning and
  weather fog; `dusk-town`'s storm front is a travelling front of the weather function.
- **Phase 5 (Netcode):** replicate the seed, the clock and weather events only; test that two clients
  a continent apart render the same storm at the same minute.
- **Phase 6 (Audio):** weather sound from the sample. **Phase 8 (Vegetation):** ecosystems, succession,
  phenology; `four-km-forest` should straddle an altitude ecotone (broadleaf to conifer) and run a
  season cycle.

**Leave out.** A general circulation model in the engine (ExoPlaSim stays an offline oracle, run as a
separate GPL tool); ocean circulation (warm and cold currents are a correction term in the bake, as Worldbuilding
Pasta does by hand); physically simulated weather at planet scale (Stormscapes-class models only as a
possible regional "hero storm" later); neural forecasting (GraphCast forecasts Earth from reanalysis and
has nothing to generate a fictional planet from); live real-world weather data; per-plant ecosystem
simulation beyond the player's region; per-animal simulation anywhere; physical lightning simulation;
granular snow physics. One biome per planet (No Man's Sky) is the explicit non-goal.

---

## Checked and left out

Kept, as in the other files, so the bibliography is auditable.

- **Alex O'Dwyer, "Breathing Life into the Wild West: Animating the Animals of 'Red Dead Redemption
  2'"** — announced for GDC 2020 (16–20 March 2020) on gdconf.com and Game Developer, described as the
  planning of RDR2's "virtual ecosystem"; no GDC Vault page was found (the Vault search redirects to a
  browse page), so its content is unverified and it is not cited.
- **Assassin's Creed Shadows seasons and weather pipeline** — the SIGGRAPH 2025 Advances PDF exceeds the
  fetch tool's 10 MB limit and the GDC 2025 talk is members-only; the 80.lv report of the GDC talk has no
  technical detail on seasons. Only the headline facts (four seasons affecting materials, weather and
  time of day; persistent snow trails) were seen, in search results. The talks are cited in
  vegetation-materials.md §2; nothing more is claimed here.
- **Eco's developer post "How the Eco-Sim Works"** — `developland.strangeloopgames.com` does not
  resolve, the ModDB copy and the Eco wiki returned 403, and the studio's current page has no technical
  text. The Steam store page is cited for the facts used.
- **theHunter need zones on the Fandom wiki** — HTTP 402; the Steam community guide is cited instead.
  The No Man's Sky Fandom wiki also returned 402; the Miraheze wiki is cited.
- **Just Cause 4 weather at GDC** — no talk found; the Game Developer article by the engineer is cited.
- **Far Cry 4 wildlife (Konieczny and Pelletier, GDC 2015)** — verified on the Vault, but it is about
  quadruped locomotion on terrain, not placement or populations; it belongs to animation.md.
- **Hao, "Thunderscapes: Simulating the Dynamics of Mesoscale Convective System"** (arXiv 2412.00703,
  2024–25) — a single-author preprint, not peer-reviewed as fetched; the peer-reviewed Stormscapes line
  covers the same ground.
- **Pan, Cui, Yang, Wang, "A Micro-Ellipsoid Model for Wet Porous Materials Rendering"** (arXiv
  2401.15628) — a preprint and an offline BSDF; Lagarde's model is what Forge needs.
- **Tang, Wu, Fan, "Computational Approach to Seasonal Changes of Living Leaves"** (2013) — verified, but
  a leaf-scale mass-spring and texture model in a medical-computation journal; Jolly's GSI and Chiba 1996
  cover the need.
- **Lam et al., "Learning skillful medium-range global weather forecasting"** (*Science* 382, 2023,
  GraphCast) — verified and deliberately left out: it forecasts Earth from reanalysis and cannot
  generate a fictional planet's weather.
- **An ecotone-modelling paper on feedbacks and seed rain** (*Landscape Ecology*, DOI
  10.1007/BF02698205) — surfaced in a search, but Springer redirected to a login and the authors could
  not be confirmed; Wilson and Agnew carry the argument.
- **The 2010 "Polygonal Map Generation for Games" article** on Stanford's server — the TLS certificate
  did not match the host; it is reached through the mapgen2 page and summarised only as that page does.
- **Nature's page for Beck et al. 2018** — redirects to a login; the PMC copy was used.
- **Weber et al. 2015 and 2016 abstracts** — HAL returned "Access Denied", ScienceDirect, Wiley and the
  Eurographics library 403, and the Semantic Scholar API 429; metadata is from Crossref and the summaries
  from search results quoting the abstracts.
- **The University of California San Diego copy of North et al. 1981** — expired certificate; the Wiley
  page and Crossref metadata are cited.
- **Implementation details not confirmed and therefore not claimed:** Minecraft's biome storage
  resolution, the default size of its biome-blend square, and a "30–60 updates per second" figure for
  mapgen4 that appeared in a search summary but on none of the fetched posts.
- **A shipped game talk on weather-state blending** (Forza Horizon, Ghost Recon, The Division, Watch
  Dogs) — none found on the Vault; Forza Horizon 5 is cited from Xbox Wire and a press report of the
  developers' stream.

---

## Verification notes

- **Method.** Every citation was checked with WebSearch and WebFetch only; **no browser pane was opened
  at any point**. Sources: publisher and society pages (Copernicus, AMS, Science, Optica, Oxford
  Academic), the Crossref API for metadata where publishers blocked fetching (North 1981, Manabe 1969,
  Weber 2015 and 2016, Festenberg 2011, Chiba 1996, Bugmann and Seidl 2022), arXiv abstract pages
  (ExoPlaSim, Nadeau–McGehee), the ACM SIGGRAPH History Archives (Garg–Nayar, Fearing, Reed–Wyvill,
  Stam), GDC Vault session pages, the Advances course indexes, author and project pages, GitHub
  repositories, vendor documentation (SideFX, QuadSpinner, World Creator, World Machine, meteoblue) and
  wikis (Minecraft, Dwarf Fortress, No Man's Sky on Miraheze).
- **PDFs read locally.** The AutoBiomes paper and NVIDIA's rain white paper were fetched as binary and
  converted with `pdftotext`; the AutoBiomes pipeline, timings and blending in §1 come from that text.
- **Blocked hosts.** Nature (login redirect), Springer (login redirect), ScienceDirect, Wiley, HAL, the
  Eurographics digital library, Fandom (402), ModDB and the Eco wiki (403), Semantic Scholar's API (429),
  two hosts with certificate problems (Stanford's student server, UCSD's course server). Each affected
  entry names the page that was used instead.
- **Corrections carried.** Ohlsson and Seipel's pages are 25–31 per the publisher (another listing says
  25–32). Hammes 2001 is a chapter in *Digital Earth Moving*, LNCS 2181, not a journal paper. The Dwarf
  Fortress pages are for v53.16. Whittaker's diagram is cited through `plotbiomes` and the AutoBiomes
  reference list, since the 1975 book has no reachable page of its own; `plotbiomes` states its polygons
  were drawn after Ricklefs' figure rather than from Whittaker directly.
- **Claims made without a primary source.** The closed-form weather function (noise rotated with the
  latitude band's wind, thresholded to the atlas) is Forge's design, not a published technique; the
  lapse rate of about 6.5 K per km used for the downscale is the standard-atmosphere value; all
  resolutions, sizes and update rates in the Recommendation are estimates for an Earth-sized planet and
  are to be measured in the demos.
- **Dates.** Checked 25 September 2026. Tool versions (Houdini 20.5 Labs, Gaea 2.2, World Creator 2025,
  WorldEngine 0.20.0, ExoPlaSim 3.4) are as displayed that day.
