# Research — A galaxy's sky: the stars an eye sees, and every one a system you can reach

> Companion to `night-sky.md` (the sky over the ground, D-046: the Yale catalogue and the procedural
> `Starfield`), `worlds-at-every-scale.md` §2 and §B (the galaxy the No Man's Sky and Elite way, a
> 64-bit id per body), `large-worlds.md` §1 (precision) and D-037 (sectors of 2⁴⁰ m). Written
> 2026-10-10 for issue #223, the owner's question of that day: the human eye sees only a bit over
> 9 000 stars; which ones depends on where you are, the bright stars around you and the ambient
> light. Does that help make a galaxy's starfield (voxels?), and how would it work, with the
> galaxy's suns as the sky's stars so that a hyperdrive toward a star goes to that system? The
> owner added the same day: a player navigates with a map, not only the eye, but pointing at a
> star should find a star there, and a ship's bridge could show screens with more stars toward
> the sector it points at. Sources were checked with WebFetch and WebSearch only, no browser pane;
> each entry says whether the page was read or only seen as a search record
> ([Verification notes](#verification-notes)). Numbers marked *(model)* come from a toy galaxy
> written for this note, not from a source (§3.1).

The short answer: **yes, the magnitude limit is the key, and it is cheap.** It does not cut the
galaxy to "the stars near you": about one naked-eye star in seven is more than 1 000 light-years
away. What it does is let a tree of cells sorted by brightness skip almost all of the galaxy, the
way Celestia, SpaceEngine and (as players have reverse-engineered it) Elite Dangerous already
store their stars. From any position, a few hundred cells and about 10⁵ generated stars give the
sky to magnitude 6.5, whatever the size of the galaxy, in milliseconds on the CPU, once per jump.
Every star drawn then carries the id of its cell and its index, so pointing at it gives the system.
The light of the stars too faint to draw (the Milky Way band) and the dust belong in a coarse
emission and absorption volume of the galaxy: that is where voxels help. The eye's sky, a bridge
telescope and the galaxy map are one query with three settings (a limit, a field, a centre).

---

## Contents

1. [How many stars an eye sees](#1-how-many-stars-an-eye-sees)
2. [How shipped software does it](#2-how-shipped-software-does-it)
3. [Techniques and numbers](#3-techniques-and-numbers)
4. [What professional engines do](#what-professional-engines-do)
5. [Recommendation for Forge](#recommendation-for-forge)
6. [What to measure](#what-to-measure)
7. [Checked and left out](#checked-and-left-out)
8. [Verification notes](#verification-notes)

---

## 1. How many stars an eye sees

**Wikipedia, "Apparent magnitude": the counts per limit.** [web]
<https://en.wikipedia.org/wiki/Apparent_magnitude>

Five magnitudes are a factor of exactly 100 in brightness, so one magnitude is 2.512. The page's
table (1997 counts, which it says have risen since Hipparcos and Gaia) gives the number of stars
brighter than each limit over the whole sky: 4 800 to 6.0, **9 100 to 6.5**, 14 000 to 7.0,
42 000 to 8.0, 121 000 to 9.0. The owner's "a bit over 9 000" is the 6.5 row, which is also the
Yale catalogue's 9 110 that D-046 draws. Only half the sky is above the horizon at once.
*Bearing:* the count grows about threefold per magnitude near the eye's limit. A limit a magnitude
fainter costs three times the stars, so the limit is the one number that sets the cost.

**Wikipedia, "Limiting magnitude", "Naked eye" and "Bortle scale": the limit on Earth.** [web]
<https://en.wikipedia.org/wiki/Limiting_magnitude> ·
<https://en.wikipedia.org/wiki/Naked_eye> · <https://en.wikipedia.org/wiki/Bortle_scale>

On a clear sky the limit is "about 6th magnitude"; dark rural sites reach about 7. Suburbs near a
city fall to about 4 (about 250 stars), Midtown Manhattan to about 2 ("only about 15 stars").
"Naked eye" puts it as about 5 600 stars brighter than 6 in a typical dark sky, 45 000 brighter
than 8 in perfect conditions, and as few as 50 in a city centre. The Bortle scale gives 7.6–8.0 for
class 1 (where the Milky Way in Scorpius and Sagittarius casts shadows) and 6.3–6.5 for class 4.
*Bearing:* "9 000" is a dark-sky number on Earth; the same eye sees 50 or 45 000 depending on the
background.

**Andrew Crumey, "Human Contrast Threshold and Astronomical Visibility", MNRAS 442 (2014),
arXiv:1405.4209; and his summary page.** [paper] [web] [still-current]
<https://arxiv.org/abs/1405.4209> · <https://crumey.co.uk/astronomy/astro.html>

A visibility model for targets of any size against backgrounds from darkness to daylight, built
on an empirical relation between contrast threshold and adaptation luminance; it replaces Hecht's
1947 formula. Wikipedia's "Limiting magnitude" quotes its result for skies darker than
21 mag/arcsec²: *m* = 0.426 μ_sky − 2.365 − 2.5 log *F*, where *F* is a personal "field factor"
of about 1.4–2.4 (2 typical), giving 6.25 at the darkest sites. Crumey's page gives a typical
dark-sky limit of 6.18 and says: "If there were no atmosphere (e.g. on the Moon), the same person
would see stars to about 8 mag."
*Bearing:* in space, with no air and no glow, the eye's limit is about **8, not 6.5**: about
42 000 stars from the Sun's position by the table above, not 9 000. The formula also says how the
limit falls as the background brightens: about 0.43 magnitude per magnitude of background.

**The light from a nearby star, a day side, and exposure.** Wikipedia's "Lux" gives the scale: a
star of magnitude 0 gives 2.54 µlx above the atmosphere (2.08 µlx at the ground), a magnitude 6
star about 8 nlx ([Wikipedia, "Lux"](https://en.wikipedia.org/wiki/Lux)). Against that, the
Sun gives about 128 000 lux at 1 AU (`SUN_ILLUMINANCE_1AU` in `starfield.rs`), and
`night-sky.md` §1 puts a moonless night about 25 stops under direct sun. Arithmetic, not a
source: one stop of exposure is 2.5 log₁₀ 2 = **0.753 magnitude**, so 25 stops is 19 magnitudes.
A frame exposed for a sunlit hull or a planet's day side shows no stars at all, which is
`starfield.rs`'s "no stars in the Moon photos" remark; an eye adapted to the dark sees to about 8
in space. A sun in the field of view raises the threshold further through glare, which Crumey's
model treats as a brighter background.
*Bearing:* in a renderer the eye's adaptation **is** the exposure. The limiting magnitude to query
follows it: *m*_lim = *m*_ref + 0.753 × (stops of exposure above the reference), capped at the
eye's 8 (or an art-directed value). D-046's night already draws stars 16 times brighter than
physical (`--star-gain`, 3 magnitudes) so the art direction is part of the limit too.

**Where the naked-eye stars are: Tom Murphy, "How Far Are Stars?", Do the Math, May 2024 (from
the Hipparcos catalogue).** [web] [recent]
<https://dothemath.ucsd.edu/2024/05/how-far-are-stars>

Median distances by apparent magnitude: 40 ly at magnitude 0, 170 ly at 1, 230 at 4, 350 at 5,
450 at 6. The middle 80 % of 6th-magnitude stars lie between 150 and 1 280 ly. About 14 % of
naked-eye stars are beyond 1 000 ly; Deneb is at 1 550 ly and eta Canis Majoris at about 3 200.
*Bearing:* the magnitude limit does **not** make the sky a neighbourhood query. Most visible stars
are within a few hundred light-years, but the brightest supergiants are seen from kiloparsecs. A
query by radius would either miss Deneb or visit millions of cells; a query by brightness does
neither (§3.1).

**Where you are in the galaxy.**
- **The core.** Barbara Ryden's Ohio State lecture notes: "Within a parsec of the galactic
  center, the estimated number density of stars is about 10 million stars per cubic parsec",
  against "a puny 0.2 star per cubic parsec" near the Sun; from there "there would be a million
  stars in our sky with apparent brightness greater than Sirius", and "the total starlight in the
  night sky would be about 200 times greater than the light of the full moon"
  ([notes 31](https://www.astronomy.ohio-state.edu/~ryden/ast162_7/notes31.html)).
- **A globular cluster's core.** Jeremy Webb (York University), on Webb and Harris's simulations
  for *Astronomy* (July 2014): "the night sky would contain 130,000 stars visible to the naked
  eye" and would be "twenty times brighter than during a full moon"
  ([yorku.ca](https://www.yorku.ca/professor/jeremywebb/?p=270)).
- **The disk and the halo.** Wikipedia's "Milky Way": 100–400 billion stars, a disk about
  87 000 ly across and about 1 000 ly thick at the arms (the thin disk 220–450 pc), the Sun
  27 000 ly (8.3 kpc) from the centre; near the Sun about one star per 8.2 pc³ within 5 pc
  ([Wikipedia](https://en.wikipedia.org/wiki/Milky_Way)). Wikipedia's "Source counts" gives the
  rule for a uniform medium: a slope of −1.5 in log *N* against log *S*
  ([Wikipedia](https://en.wikipedia.org/wiki/Source_counts)), which is log *N* = 0.6 *m* + const
  in magnitudes, so the count above a limit scales **linearly with the local density of stars**.
  The real sky's 0.5 per magnitude (`night.rs`'s procedural law) is flatter because the disk is
  thin and dusty.
- *The model's figures* (§3.1), as ratios to its own count at the Sun: 3.5 times at 2 kpc from the
  centre in the plane, 0.15 at 14 kpc, 0.25 at 1 kpc above the plane, and about 0.01 at 10 kpc
  above it, where the model has no halo stars and the sky is the disk seen from outside: about
  200 stars, most of them kiloparsecs away *(model)*.
*Bearing:* the eye's count runs from about a hundred (high above the disk) through 9 000 (here) to
10⁵–10⁶ (a cluster or the core). A galaxy's sky must be computed from the galaxy, not drawn from
one catalogue, and the query must stay bounded where the density is 10⁸ times the Sun's: the
dense places are where the per-cell caps of §3.1 matter.

**The Milky Way band is the unresolved stars.** Wikipedia's "Milky Way": its glow comes from
unresolved stars and other material in the plane; its surface brightness is low, so light
pollution or moonlight hides it. Wikipedia's "Extinction (astronomy)" gives about 1.8 mag/kpc in
the visual near the plane (0.7–1.0 averaged near the Sun) and more than 30 magnitudes towards the
centre ([Wikipedia](https://en.wikipedia.org/wiki/Extinction_(astronomy))). Jensen et al.'s
physical night sky (SIGGRAPH 2001, `night-sky.md` §1) used a processed photograph for the band.
*Bearing:* the band is the integral of every star fainter than the limit, dimmed by dust along the
line of sight: a volume integral, which is what a voxel grid computes well (§3.2).

---

## 2. How shipped software does it

**Elite Dangerous (Frontier, 2014).** Stellar Forge's galaxy is addressed by a 64-bit integer
holding a sector's coordinates, the layer of an "eight-layer octree", the system and the body
([80.lv, Doc Ross, 5 April 2018](https://80.lv/articles/generating-the-universe-in-elite-dangerous/);
`worlds-at-every-scale.md` §2). The catalogues are overlaid on the generated galaxy: Braben, to
PCGamesN, "Every single star that's in our night sky is in the game", the naked-eye "6,000 or
7,000" plus "the 160,000 or so stars that are only visible with telescopes", with extra dust so
the sky looks right ([PCGamesN, updated 14 August 2017](https://pcgamesn.com/elite-dangerous/every-single-star-in-our-night-sky-is-in-elite-dangerous-david-braben-on-re-creating-a-galaxy-all-over-again));
"every single star in the real night sky is present ... and you can visit each one"
([Engadget, 13 July 2014](https://www.engadget.com/2014-07-13-visit-every-system-in-the-milky-way-in-elite-dangerous.html)).
Frontier announced "some 400,000,000,000 star systems" on the galactic map
([Engadget, 10 April 2014](https://www.engadget.com/2014-04-10-elites-premium-beta-starts-today-expands-may-30.html)).
- **The sky.** No official text on the skybox was found. A 2016 procedural-generation blog sums it
  up as "Every point of light you can see is a star that you can visit", which "requires that the
  generator be predictable and deterministic", with the galaxy's density from a 3D version of the
  earlier games' density bitmap ([procedural-generation.tumblr.com, 9 August 2016](https://procedural-generation.tumblr.com/post/148698050964/order-chaos-and-scale-in-elite-dangerous-the)).
  Players on Steam (2015–2019) report the sky "calculated on the fly" from the surrounding stars
  while jumping, in "a fraction of a second", with catalogue systems replacing generated ones
  ([Steam thread](https://steamcommunity.com/app/359320/discussions/0/521643320355026480/));
  none of them is a developer. A 2014 newsletter (#21) is reported to say Alphas 1–3 used a
  painted "skydome", and that a skydome 100 000 ly out still holds the other galaxies (search
  record only).
- **The cells.** The players' reverse-engineering of the system address (the "id64") shows the
  octree's layers are *mass codes*: a 3-bit code *a*–*h*, cubes of 10 × 2^code light-years inside
  sectors of 1 280 ly, the cube's position in the sector, the sector, and the system's index in its
  cube as the remainder ([`19h/edm`, `id64.rs`, GitHub](https://github.com/19h/edm/blob/master/crates/edm-core/src/domain/id64.rs)).
  Forum guides add that the mass code is the upper bound of the system's mass, *h* cubes holding
  giants and black holes, *a* cubes brown and red dwarfs (search record).
- **The map.** Route plotting with filters by star type and jump range; no text on how the map
  queries or draws its systems was found.
*Bearing:* the closest shipped match to the owner's idea, and its storage is the technique of §3.1:
heavy (bright) stars in big cells, light ones in small cells, the name of a star being its cell
and index. That is why a sky can be generated in a fraction of a second from anywhere.

**SpaceEngine (Vladimir Romanyuk).** "Procedural stars update", 1 February 2010: a "hierarchical
octal tree, 10 levels", where "nodes of level 0 contains the brightest stars"; the density in the
nodes "corresponds to a typical luminosity function for the vicinity of the Sun"; nodes and stars
are generated and removed on demand during flight, each node with "a unique seed ... so the same
stars are always created at the same places"; a star's name is "the galaxy index, tree level
number, node number, and star index in the node"; about 100 vertex buffers a frame held about
300 000 stars; and "the search for a nearby star ... performs very bad", a linear scan of the
300 000 ([blog100201](https://spaceengine.org/news/blog100201/)). The manual: the whole Hipparcos
catalogue, the known exoplanets and over ten thousand galaxies (over 130 000 real objects) beside
the procedural universe, and an F7 menu for "the camera's exposure, the ambient light level, and
magnitude limit settings" ([manual](https://spaceengine.org/manual/)). Its star browser examines
each generated planetary system against filters ([blog140829, 29 August
2014](https://spaceengine.org/news/blog140829/)); players report a cap of 10 000 systems a search
(forum records). On volumes: "almost every thing in space is volumetric and transparent", galaxies
and nebulae first ([blog161008, 8 October 2016](https://spaceengine.org/news/blog161008/)); the
0.990 release raymarched its nebulae (search record).
*Bearing:* the same tree as Elite's and Celestia's, used for drawing as well as naming. Two
lessons: the exposure and the magnitude limit are the player's sky controls, and a separate
nearest-star search must not be a scan (Forge's systems-near-the-ship list is the bottom levels of
the same tree).

**Celestia (open source, GPL-2.0).** The star database is an octree in which "the fainter the
star, the deeper the node in which it will reside. Each node stores an absolute magnitude; no
child of the node is allowed contain a star brighter than this value, making it possible to
determine quickly whether or not to cull subtrees"; each level down is a factor of four fainter,
1.505 magnitudes; a node splits past 75 stars ([`stardbbuilder.cpp`](https://github.com/CelestiaProject/Celestia/blob/master/src/celengine/stardbbuilder.cpp)).
The visit tests a node against the view frustum, then skips it when its brightest magnitude plus
the distance modulus of its nearest point is fainter than the limit ([`staroctree.cpp`](https://github.com/CelestiaProject/Celestia/blob/master/src/celengine/staroctree.cpp)).
Picking runs the same visit with the click's ray as a cone of the tolerance's width and keeps the
star of smallest angle, after a precise test for stars within a light-year
([`universe.cpp`](https://github.com/CelestiaProject/Celestia/blob/master/src/celengine/universe.cpp)).
"Auto-magnitude" deepens the limit as the field narrows (`Renderer::autoMag` scales the faintest
magnitude by the field's correction, [`render.cpp`](https://github.com/CelestiaProject/Celestia/blob/master/src/celengine/render.cpp)).
*Bearing:* the clearest public statement of the query. Its one property worth copying exactly: four
times fainter is twice as near, so halving the cell size at each 1.505 magnitudes keeps the number
of cells visited per level constant. The code is GPL, so Forge takes the published technique, not
the code.

**Gaia Sky (ARI Heidelberg) and OpenSpace.** Gaia Sky draws the Gaia catalogues (up to 1.46 billion
stars, [Wikipedia](https://en.wikipedia.org/wiki/Gaia_Sky)) from a level-of-detail octree, culled
by an octant's solid angle as seen from the camera, with a cap on the stars resident and the
least recently seen octants unloaded ([docs, "LOD catalogs"](https://gaia.ari.uni-heidelberg.de/gaiasky/docs/3.6.5/LOD-catalogs.html));
its paper (Sagristà et al., IEEE TVCG 25(1), 2019) describes a "magnitude–space" octree and a
floating camera for single precision (search record; the PDF did not decode). OpenSpace's
`RenderableGalaxy` combines "a volume component rendered by raycasting" with "a point-based star
component", the volume with dust absorption and emission factors
([OpenSpace docs](https://docs.openspaceproject.com/releases-v0.20/generated/asset-components/RenderableGalaxy.html)).
*Bearing:* streaming a real catalogue (a page pool of octants, D-037's clipmap by another name) and
the points-plus-volume split of §3.2, both shipped in scientific viewers.

**No Man's Sky (Hello Games).** The galaxy map is the navigation instrument. The 2016 game drew a
system's sun in a skybox: Destructoid's report of the September 2026 update says stars "existed
only in the skybox" until it added "real, 3D, interactive stars to every system"
([Destructoid, 11 September 2026](https://www.destructoid.com/no-mans-sky-new-update/)); the 7.0
"Cosmos" notes (released 9 September 2026 by Gematsu, search record) list a star-system map and "a point of interest at the star(s)
of a system", and fix "the starfield to fail to render when jumping through the centre of a
galaxy" ([Nintendo Everything's copy of the notes](https://nintendoeverything.com/no-mans-sky-7-0-cosmos-update-announced-patch-notes/)).
Whether its night-sky points are the galaxy map's systems is not documented; players doubt it
(Steam threads, search record).
*Bearing:* a shipped galaxy can live with a decorative sky and a map; the owner's idea is Elite's,
not No Man's Sky's.

**Star Citizen (CIG).** A few hand-built systems joined by jump points: the Stanton–Pyro jump point,
first shown at CitizenCon 2019, sits "in a gas cloud" with structures and a station
([starcitizen.tools](https://starcitizen.tools/Pyro_-_Stanton_jump_point)). How its skies are made
is not published. *Bearing:* travel by gates, not by pointing at stars; not the model asked for.

**Kerbal Space Program (Squad).** One star system; the sky is six textures, a cube map
(`GalaxyTex_PositiveX` … `NegativeZ`, "skybox right face" and so on, in the
[TextureReplacer README](https://github.com/ducakar/TextureReplacer)). *Bearing:* the baseline
Forge's `Starfield` already beats.

**Starfield (Bethesda, 2023).** "More than 1,000 planets" in fictional and real systems within
about 50 light-years of Sol ([Wikipedia](https://en.wikipedia.org/wiki/Starfield_(video_game))).
Mod authors report the sky as mostly the real one, mirrored, with a static Milky Way texture, and
the grav-jump marker not on its star (Nexus Mods pages, refused the fetch). *Bearing:* the failure
the owner wants to avoid: a marker and a point of light that disagree.

---

## 3. Techniques and numbers

### 3.1 The magnitude-limited query

**The tree.** Level ℓ holds the stars whose absolute magnitude *M* lies in a band
(*M*_top + 1.505(ℓ − 1), *M*_top + 1.505 ℓ], in cubic cells of side *s*₀ / 2^ℓ (Celestia's rule;
Elite's mass codes and SpaceEngine's levels are the same idea). A star of magnitude *M* is
brighter than the limit *m* within *d*(*M*) = 10 pc × 10^(0.2 (*m* − *M*)). Each level's band is
four times fainter and its cells half the size, so *d* / *s* is the same at every level, and the
cells visited per level are a constant: those within *d* + half a diagonal of the observer, about
(4/3)π (*d*/*s* + 0.87)³. With *d* ≈ 2 *s* that is about 100 cells a level, fewer where the
galaxy is empty. The visit at each cell asks the galaxy model for the expected count of its band
there (density × luminosity-function fraction × volume), draws the count from the cell's seed, and
generates only those stars. The whole galaxy outside a few hundred cells is never touched: the
cost depends on the limit and the local density, **not on the galaxy's size**.

**The numbers, from a toy galaxy *(model)*.** For this note, a double-exponential disk (scale
length 2.6 kpc, 0.1 star/pc³ at 8.3 kpc), nine stellar classes with the main-sequence shares from
Wikipedia's "Stellar classification" (M 76 %, K 12 %, G 7.6 %, F 3 %, A 0.61 %, B 0.12 %,
O 0.00003 %, [Wikipedia](https://en.wikipedia.org/wiki/Stellar_classification)), a giant and a
supergiant class, absolute magnitudes with a spread per class, scale heights 50–300 pc and dust at
1 mag/kpc in a 100 pc layer: 62 billion stars. Integrated over the sphere (node.js scripts
written for this note, not kept):
- **From the Sun to 6.5:** 16 500 stars (the real sky: 9 100; the toy is within a factor of two),
  half within 156 pc, 90 % within 620 pc, 99 % within 2.5 kpc (Hipparcos: 90 % within about
  300 pc). To 8: 81 000. To 10: 620 000.
- **The tree** (cells of 2^68 m ≈ 9.6 kpc at the top, 20 levels, *M*_top = −7.5): **568 cells
  visited and about 144 000 stars generated** for the 16 500 seen, about 44 cells per active level
  over 13 levels. At 2 kpc from the centre: 608 cells, 1.6 million generated (the toy does not prune
  by dust, which would cut most of them).
- **A narrow field, deeper:** a 5° cone to magnitude 9, 276 cells and 85 000 stars generated; a
  1° cone to 12 in the plane, 768 cells and 209 000; the same toward the pole, 646 and 38 000. The
  whole sky to 9: 6 344 cells and 1.1 million.
- **The cost.** At 50–200 ns to generate a star (a few hashes, an inverse-CDF lookup, a position;
  an assumption to measure), 144 000 stars take 7–30 ms on one core, a few milliseconds on Forge's
  six workers: once per jump, not per frame.
- **Cutting the waste.** The tree generates whole cells, so it makes about nine stars for each one
  seen, and many more for a narrow cone through big cells. A cell's count can instead be split among
  its eight children by a seeded multinomial draw, recursively, so a cone generates only the
  sub-cells it crosses. This is a design for Forge, not something a source documents.

**The bottom of the tree.** Faint stars (red dwarfs, *M* ≈ +9 to +16) are seen only within
0.1–6 pc at 6.5, but they are most of the systems a player may want to visit. Elite stops at eight
layers with a 10 ly bottom cube whose index field is wide; Celestia splits cells by count (75). A
bottom level that holds every star fainter than about +3 in cells of 2^56 m (2.3 pc) costs, at
6.5, about 45 000 cells near the Sun with about one star each *(arithmetic)*; deeper levels do the
same with fewer stars per cell. In the core, 10⁷ stars/pc³ would put 10⁸ in such a cell: either the
generator caps the density ("beautiful and playable over realism") or dense cells split further.

### 3.2 Points near, a volume far: what voxels are for

The query draws the stars above the limit as points. Everything below it is light without
identity: the Milky Way band, the glow of the core, the dark lanes of dust. A coarse grid of the
galaxy (density per population, dust) answers it as a volume integral from the observer:
- **The emission per voxel depends on the distance.** At distance *d* along a ray, the stars
  already drawn as points are those brighter than *M*_cut = *m*_lim − 5 log₁₀(*d*/10 pc) − *A*(*d*).
  The voxel adds only the light of the fainter ones: density × the luminosity function's light
  fraction fainter than *M*_cut (a 1-D table per population). Near the observer that fraction is
  small, far away it is all of it, and nothing is counted twice. *(A design, derived here.)*
- **Dust absorbs along the same ray,** and dims the points by the same *A*(*d*), which the query
  takes from the volume before testing the limit (Braben's "more dust" is this term).
- **Size.** A 256 × 256 × 32 grid over a 30 kpc disk is 117 pc × 117 pc × 94 pc a voxel,
  2.1 million voxels, about 17 MB at 8 bytes (half-float emission and dust). A sky cube of 6 × 256²
  rays at 256 steps is 100 million samples: well under a few milliseconds on the RTX 5070 Ti, once
  per jump *(estimate)*.
- **The same grid is the generator's input** (Elite's density map, Stellar Forge's "top-down map of
  the galaxy", the one painted input) and the galaxy map's far view.
OpenSpace's points-plus-raycast galaxy and SpaceEngine's "everything is volumetric" are the shipped
precedents. *Bearing:* voxels help for the diffuse sky, the dust and the map; the stars themselves
stay points.

### 3.3 Caching the sky: per system, recomputed per jump

A star at distance *D* moves across the sky by Δ*x* / *D* when the observer moves Δ*x*. At 1440p
with a 70° field a pixel is about 0.027° (4.8 × 10⁻⁴ rad). A star system's frame reaches 10¹³ m
(D-037), 3.2 × 10⁻⁴ pc; seen against the nearest star at 1.3 pc (Alpha Centauri from the Sun) that
is 2.5 × 10⁻⁴ rad, **half a pixel**, and D-037's sector (2⁴⁰ m, 3.6 × 10⁻⁵ pc) moves it a twentieth
of a pixel *(arithmetic)*. So:
- **One sky per star system is exact** for everything outside it. The system's own star and its
  companions are bodies of the scene, drawn as such.
- **A hyperdrive jump recomputes the sky** (a few milliseconds of query, plus the volume's bake).
  During a visible cruise, the few nearest stars move: recompute the bottom levels near the ship
  every few frames and keep the rest.
- **Precomputing per sector or per system** (the issue's question) is unnecessary: the query is
  cheaper than reading a stored sky, and a stored sky per system for 10¹¹ systems is impossible
  anyway. A cache keyed on the system id (the last few skies) covers going back and forth.
- **The exposure changes per frame** but not the query: query to the deepest limit the frame may
  need (8, or the art's limit), and fade stars by magnitude against the current exposure in the
  shader. Requery only when the limit moves by more than a magnitude.

### 3.4 Precision and determinism

- **Positions.** A galaxy of 30 kpc is about 2^70 m. An `f64` resolves 2^17 m (131 km) there
  (`large-worlds.md` §1's arithmetic), fine for a direction (10⁻¹⁶ rad) and not for arriving.
  D-004 and D-037 already give the answer: `i64` sectors of 2⁴⁰ m and `f64` inside. A star's
  position must be **integer**: its cell's corner in sector units plus a hashed offset in fixed
  point (for example in units of 2³⁰ m, 7 AU / 1 024), so every platform places it on the same
  bit, and the system's frame origin comes from it exactly.
- **Determinism (D-016).** The query is integers and hashes (`Seed::derive` from the cell id), the
  magnitudes from a table, the count from a hashed draw; distances and magnitudes for the *drawing*
  may be `f32`, but the *choice* of which stars exist and their ids may not depend on floating
  point. Two clients, the server and a replay then agree on every star's id.
- **The id.** A star's name is its level, its cell at that level and its index in the cell:
  SpaceEngine's and Celestia's naming, and Elite's id64 (mass code, cell in the sector, sector,
  index). For Forge a candidate is 3 bits of level, the cell's coordinates in the galaxy cube at
  that level, and the index in the rest: at a bottom level of 2^56 m cells in a 2^70 m cube,
  3 × 14 bits of cell, 3 of level, 19 of index (524 288 stars a cell). Bodies inside a system get
  their own index beside it. Name it `StarId` or `SystemId`: `forge-physics` already has a
  `BodyId`.
- **Catalogue stars.** As Elite does, real stars (Yale today, Hipparcos later) are placed into the
  same tree and replace the generated stars of their cell; around the Sun the sky is then the real
  one, and the Yale switch of D-046 becomes the galaxy seen from Sol.

### 3.5 Three views, one query

The owner's three views are the same visit with different parameters:

| View | Centre | Field | Limit | Cells / stars *(model)* | Drawn as |
|---|---|---|---|---|---|
| **The eye** (the sky) | the ship | the whole sky | 6.5–8, following exposure | ~600 / ~10⁵ generated, ~10⁴ kept | points over the volume's sky cube |
| **A bridge screen or telescope** | the ship | a cone of 0.25–5° along the ship's axis | 9–15, deeper as the field narrows (Celestia's auto-magnitude) | 300–800 / 10⁴–2 × 10⁵, fewer with the count split | points in a screen's render target |
| **The galaxy map** | a focus point the player moves | a box or sphere round it | a level of the tree (the top levels are the bright stars), plus filters | bounded by a budget, say 10⁴–10⁵ systems | points and labels over the volume drawn as a map |

- **The map is the tree read top-down.** Shown from far, only the top levels (the giants, as Elite's
  *h* cubes) and the volume; zooming in reveals deeper levels round the focus, never more than the
  budget (SpaceEngine's browser caps a search at 10 000 systems, players report). Filters (star
  type, explored, reachable with the jump range) are predicates on the generated records.
- **Picking.** The view's star list (id, direction, magnitude) is the pick set: a click is a ray,
  and the star of smallest angle within a few pixels (Celestia: the same visit with a cone of the
  tolerance's width) is the target; preferring the brighter of two close ones matches what the eye
  sees. With 10⁴ stars a CPU scan is microseconds; with 10⁶ (a deep screen), a GPU id buffer read
  back at one pixel. The hyperdrive then targets that `StarId`, whose position is exact (§3.4).
- **The GPU cost.** Schütz, Kerbl and Wimmer rasterise "up to two billion points in real time
  (60fps)" in compute ([arXiv:2204.01287](https://arxiv.org/abs/2204.01287)); 10⁴–10⁶ stars are
  four to six orders of magnitude below that, so drawing is not the cost at any view. Forge's
  night pass bins stars into a 64² × 6 cube of cells and loops per pixel (`night.rs`), which suits
  10⁴; a deep screen with 10⁵–10⁶ stars wants a splat pass (each star's Gaussian added into an HDR
  target). A list of 32 bytes a star is 320 KB at 10⁴ and 32 MB at 10⁶.

---

## What professional engines do

| Software | Sky's stars are visitable systems | Storage of stars | Diffuse light | Limit and exposure | Picking / map |
|---|---|---|---|---|---|
| Elite Dangerous | yes (developer claims; mechanism not published) | 8-layer octree by mass code, id64 = cell + index (reverse-engineered) | a generated galaxy background; extra dust | not documented | galaxy map with filters and route plotter |
| SpaceEngine | yes (every star is generated where it is) | 10-level octree, brightest at level 0, seeded per node | volumetric nebulae; galaxies planned as volumes | exposure, ambient and magnitude limit in F7 | star browser by radius and filters, ~10 000 cap (players) |
| Celestia | catalogue stars, all selectable | octree by absolute magnitude, 1.505 mag a level | galaxies as point sprites (forum, search record) | faintest magnitude, auto-magnitude by field | pick = the same visit in a cone |
| Gaia Sky / OpenSpace | catalogue stars | LOD octree streamed with a resident cap / points + raycast volume | volume with dust absorption and emission | camera-based | viewer controls |
| No Man's Sky | not documented; stars in systems only since 7.0 (2026) | seeds per region and system | painted / generated skybox | — | galaxy map, system map since 7.0 |
| Star Citizen | no (hand-built systems, jump points) | authored | authored | — | starmap |
| Kerbal Space Program | no (one system) | a cube map | in the cube map | — | — |
| Starfield | no (mods report the marker off its star) | the real sky, mirrored (mods) | static texture (mods) | — | starmap |

---

## Recommendation for Forge

**Verdict.** The magnitude limit is useful, but not in the way the question first suggests: it does
not shrink the sky to the stars nearby, it makes a brightness-sorted tree visit a few hundred cells
out of the galaxy's billions. That makes the owner's idea, every point of light a system you can
point at and jump to, cheap: milliseconds per jump, independent of the galaxy's size, and the same
code gives the telescope screen, the map and the pick. Voxels are worth having for what the eye
cannot resolve (the band, the core's glow, the dust) and as the generator's density input, not for
the stars. Nothing here is needed before the galaxy itself (`worlds-at-every-scale.md` §B), and the
first steps run without a GPU.

**Steps, fitted to what exists:**
1. **The id and the tree, CPU only** (`forge-world` or a new `forge-galaxy`): `StarId` (§3.4), the
   cell levels at 1.505 magnitudes and half the size, a galaxy model (a density grid per population
   with a luminosity function per population, painted or analytic), seeded counts and integer
   positions. Test: the same ids at one and six workers, debug and release (D-016's digests);
   counts against the toy's numbers.
2. **The query:** `visible(observer, cone, m_lim) -> Vec<(StarId, direction, magnitude, colour)>`,
   with the multinomial split for cones. Measure cells, stars generated and milliseconds from the
   Sun, the inner disk, above the disk.
3. **The sky from it,** into the existing night pass: `night::Star` gains its `StarId`, and
   `bin_stars` takes the query's list instead of `procedural_stars` or the Yale file. From Sol with
   the catalogue merged in, the sky must match D-046's `--real-sky`, which becomes the regression
   image. The procedural `Starfield` stays for scenes without a galaxy.
4. **The limit follows the exposure** (0.753 mag a stop, capped at 8 or an art value), the fade by
   magnitude in the shader, with D-046's star gain counted in.
5. **The volume:** the density and dust grid, the per-distance emissivity of §3.2, a sky cube baked
   per jump on the GPU, composed under the points. Compare from Sol with a Milky Way photograph's
   shape (Jensen et al.'s reference layer).
6. **Picking and the jump:** the pick on the view's list, the target's exact position from its id,
   the sky recomputed on arrival (and the nearest stars refreshed during a visible cruise).
7. **The other views:** a bridge screen as a second camera with a cone query to a deeper limit; the
   galaxy map as the tree read top-down round a focus, with a budget and filters.

**For a decision later (D-0xx, after D-057, when the galaxy phase starts):**
1. **The `StarId` layout:** levels, the bottom cell size, bits for the cell and the index, how
   bodies are numbered inside a system, and the generator version in the key (D-053).
2. **The tree's bands:** 1.505 magnitudes a level (Celestia's constant-cost rule) or Elite-like mass
   codes, and the bottom level's catch-all; the density cap or extra splitting in clusters and the
   core.
3. **The galaxy model's inputs:** analytic (disk, bulge, arms, halo) or a painted density grid, and
   which catalogues (Yale only, or Hipparcos as Elite and SpaceEngine use, a download needing the owner's go).
4. **The limits:** the eye's limit in space (8 as Crumey, or 6.5 as on Earth, for a sky the owner
   knows), how exposure drives it, and the telescope's law with the field.
5. **The diffuse sky:** the grid's size and the bake per jump, or a painted band for a first version.
6. **Realism or play** where they part: the core's 10⁵–10⁶ visible stars and a sky 200 times a full
   Moon, against a capped and readable sky.

---

## What to measure

- **The query:** cells visited, stars generated, stars kept and milliseconds (p50/p99 over 1 000
  random positions), at 6.5 and 8, from the solar neighbourhood, the inner disk, above the disk and
  a cluster; one and six workers; the digest of the ids.
- **The cone:** the same for 0.25°, 1° and 5° to 9, 12 and 15, with and without the count split.
- **The sky:** from Sol with the Yale stars merged, the ꟻLIP against `--real-sky`'s capture; the
  count per magnitude against the table (4 800 to 6.0, 9 100 to 6.5, 42 000 to 8).
- **The volume:** bake milliseconds per jump, bytes, and the band's brightness against the points'
  integrated light at the boundary (no visible seam when the limit moves by a magnitude).
- **Picking:** the angle error and the time for a pick at 10⁴ and 10⁶ stars; that the jump's
  target is the picked id, every time, across two runs.
- **The GPU:** the night pass's milliseconds at 10⁴, 10⁵ and 10⁶ stars, binned against splatted.

---

## Checked and left out

- **Elite Dangerous's own description of its skybox and galaxy map:** Frontier's forum and the
  Elite wiki refused the fetch (403 and 402); the Space.com interview with Braben loaded without its
  text. The id64 layout is players' reverse engineering, read in a third-party decoder.
- **Starfield's sky:** only mod pages (Nexus Mods refused the fetch) and fan analyses.
- **Star Citizen's skies:** no source on how they are made.
- **Gaia Sky's paper:** the PDF fetched but did not decode; its claims here come from the abstract
  as search records and the documentation.
- **A measured naked-eye limit in orbit:** none found; Crumey's 8 is a model's figure for the Moon,
  and a forum's "about a magnitude fainter" was left out.
- **The halo's real star density and a cited stellar luminosity function table:** not found in a
  readable source; the toy galaxy uses class shares and spreads instead, and its absolute
  numbers are within a factor of two of the real sky.
- **No Man's Sky's skybox:** only player threads say whether its points are systems.

---

## Verification notes

Checked on 2026-10-10 with WebFetch, WebSearch and `gh api` (for GitHub files) only; no browser
pane, nothing downloaded into the repository. "Read" means the page or file was returned and the
quotes are from it; "search record" means only a search engine's extract was seen.

- **Read:** Wikipedia's "Apparent magnitude" (the counts table), "Limiting magnitude" (Crumey's
  formula as quoted there), "Naked eye", "Bortle scale", "Milky Way", "Source counts", "Extinction
  (astronomy)", "Lux", "Stellar classification", "Gaia Sky", "Starfield (video game)"; arXiv
  1405.4209 (Crumey's abstract) and crumey.co.uk's summary page; arXiv 2204.01287 (Schütz et al.'s
  abstract); dothemath.ucsd.edu "How Far Are Stars?" (May 2024); Ryden's notes 31 (Ohio State);
  Webb's page (York University); 80.lv's Elite interview (5 April 2018); PCGamesN on Braben (updated
  14 August 2017); Engadget, 13 July 2014 and 10 April 2014; procedural-generation.tumblr.com
  (9 August 2016); the Steam thread on reaching seen stars (2015–2019); SpaceEngine's blog posts
  100201, 140829 and 161008 and its manual; the Gaia Sky "LOD catalogs" page (3.6.5); OpenSpace's
  `RenderableGalaxy` page (0.20); Destructoid (11 September 2026); Nintendo Everything's copy of the
  No Man's Sky 7.0 notes; starcitizen.tools "Pyro – Stanton jump point"; on GitHub (via `gh api`):
  Celestia's `staroctree.cpp`, `stardbbuilder.cpp`, `universe.cpp`, `render.cpp` and
  `celestiacore.cpp` (master, licence GPL-2.0), `19h/edm`'s `id64.rs`, TextureReplacer's README.
- **Search record only:** Elite's newsletter #21 on the Alpha skydome; the Frontier forum guides to
  mass codes and boxels; SpaceEngine 0.990's raymarched nebulae and the star browser's 10 000 cap;
  Gaia Sky's TVCG paper (Sagristà, Jordan, Müller, Sadlo 2019); the Starfield mod pages; No Man's
  Sky players' threads.
- **Refused or empty:** forums.frontier.co.uk (403), elite-dangerous.fandom.com (402),
  elitedangerous.com (403), nexusmods.com (403), the KSP forum (403), space.com's Braben interview
  (no article text), the Gaia Sky PDF (did not decode): each [not checked] where cited.
- **From Forge, read in the repository:** `docs/research/night-sky.md`, `worlds-at-every-scale.md`,
  `large-worlds.md`, `docs/DECISIONS.md` (D-004, D-016, D-037, D-046), `crates/forge-render/src/
  night.rs` and `starfield.rs`, `crates/forge-world/src/frame.rs` and `cells.rs`,
  `crates/forge-physics/src/lib.rs` (`BodyId`).
- **Numbers to re-check before a spec:** every *(model)* figure (a toy galaxy in node.js, not
  calibrated: 16 500 stars to 6.5 against the real 9 100); the 50–200 ns a generated star; the
  volume's bake time; the `StarId` bit counts (arithmetic only).
