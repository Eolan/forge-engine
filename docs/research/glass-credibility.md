
# Glass that reads as real

*Research note, 2026-10-21. The owner looked at the engine's windows and said the glass looks "too perfect". This note covers four things: what makes real glass read as real, how shipped games and offline rendering get there, what each technique costs and how likely it is to shimmer, and what Forge should build first. A research agent wrote it from web sources checked that day and from Forge's code (`shaders/glass.slang`, `shaders/meshlet.slang`) and `docs/PROFILE.md`. No code was changed.*

**How each claim was checked:**
- **[read]**: the page was fetched and read.
- **[search]**: only a search summary or an abstract was seen. Either the PDF could not be read on this machine, or the site refused the fetch.
- **[local]**: read in Forge's own code or docs.
- **[inferred]**: reasoning or an estimate, still to be measured.
- **[recalled]**: from knowledge of the paper, not re-read today.

> **In short.**
> 1. **Why Forge's glass looks too perfect.** Today every pane is the same flat, spotless mirror. The village's clear glass has one reflectance for every pane (F0 0.04, a single surface). It has a perfect mirror ray, no roughness, no sun glint, and nothing that changes from pane to pane except its room's light [L1]. Real glass is never flat, never clean, and never the same twice.
> 2. **What sells glass at street distance:** pane-to-pane differences.
>    - Reflections bend differently in each pane. Double glazing bows with temperature and pressure, heat-treated glass carries roller waves, and panes are not quite in one plane [6][9][10][14].
>    - Each pane carries a different overall haze.
>    - What is behind each window differs: curtains, blinds, lit or dark rooms. Games vary this per window [31][32][37].
>
>    **Up close:** smudges and fingerprints, dust gathered at edges and corners, streaks, a faint double image from double glazing, and a green cast at grazing angles [15][18][21].
> 3. **What games do.** The shipped answers are variation per window and ray-traced reflections on transparent surfaces:
>    - City Sample, SCS's trucks and Marvel's Spider-Man 2 vary interiors, curtains and lights per window [31][32][37].
>    - Cyberpunk 2077, Hitman 3 and Alan Wake 2 ray-trace reflections on glass [41][42][43][44].
>
>    No game source found today models the bow of a pane. Architectural simulation does (Eclat Digital's Ocean) [14].
> 4. **Recommendation, in order.**
>    1. Give every pane a "condition" drawn from a hash of where it stands: bow, a small tilt, roller waves, a haze level and a glazing type (F0 about 0.08 for single glazing, about 0.15 for double), with geometric specular AA on the bent normals. This is arithmetic only, about +0.02 to +0.06 ms at 1600×900 [inferred].
>    2. Add grime from CC0 textures in pane space, with edge and corner masks, different sets inside and outside, and fingerprints and stickers as per-pane decals.
>    3. Add sun glints through the mirror direction, once the AA holds.
>    4. Vary what is behind the glass: blinds at random heights, frosted bathroom panes.
>    5. Optionally, a second-pane reflection near the camera.
>
>    Weather (condensation, rain) and old glass come later.
> 5. **Shimmer.** Keep every addition low-frequency. Fade each one out by the pixel's footprint and fold what is lost into the lobe width. Test under `--dlaa` against `--ssaa`, in motion and with a still camera. Two pitfalls specific to Forge:
>    - The clear glass carries the room's motion vectors, so grime on it is reprojected as if it were the room.
>    - The reflection history stores weighted results, so a per-pixel grime weight must be applied after the history.
> 6. **Patents.**
>    - Nothing found on grime or edge masks, pane bow, double reflections, thickness tint, Toksvig, or geometric specular AA.
>    - Avoid LEAN mapping. Take-Two's US 9,684,975 B2 runs to about 2033. `trim-sheets.md` lists it as "none found": correct that.
>    - Ask before using ray cones to choose texture levels at reflection hits. NVIDIA's US 10,460,504 B2 runs to about 2038, and other family members to about 2040.
>    - The interior patents already on record (Celsys, Bosch, Sony) are not touched.

## 0. What Forge draws today [L1][L2][L3]

- **Clear glass over built rooms (the village).**
- `glass/requests` asks for one mirror ray per pixel.
- `glass/draw` lays the pane over the room in three parts: the room dimmed by (1 − Fresnel) times a fixed share, the sky in the mirror direction weighted by Schlick's Fresnel, and a lit room's glow.
- One F0 serves every pane (`look.y`, 0.04 by default).
- The shader has no roughness, no texture and no sun highlight. Its own comment says there is no sun highlight on the glass and therefore no shadow ray.
- The glass writes no depth. TAA (now DLAA) reprojects the room through it using the room's own depth.
- **Faked rooms** (houses without insides, and the city):
- `shade_standard`'s interior branch draws the pane's sun highlight (Blinn-Phong about 3000) and its sky reflection over an interior-mapped room.
- Curtains are chosen by a hash of where the window stands, so a city that gains or loses an instance keeps its rooms.
- **Tower glass** is a coated curtain wall: F0 about 0.3, roughness about 0.285.
- It traces its whole mirror ray, because it is under `MIRROR_RAY_FULL` (0.30).
- `sky_reflection` blends 2r² of it towards the sky's irradiance. That is about 16 % at roughness 0.285.
- **Reflection history** (#176, D-050). `steadied_reflection` reprojects each ray's result through a virtual point behind a flat mirror. It stores the result already weighted by the request's weight (Fresnel × haze).
- **Costs** at 1600 × 900 [L3]:

| Zone | Cost | Where measured |
|---|---|---|
| `glass/requests` | 0.011–0.018 ms | map-check views, 15:00, 2026-10-18 |
| `glass/draw` | 0.004–0.006 ms | same |
| `glass/reflections` | 0.035–0.141 ms | same |
| `shading/reflections` | about 0.19 ms (with history) | the city's default view |

**Diagnosis:** every pane is a flat, spotless, identical mirror. Everything below aims to break that, without adding shimmer.

## 1. What makes real glass read as real

### 1.1 How strongly glass reflects: Fresnel, panes and coatings

- **Uncoated glass** (n ≈ 1.5) reflects about 4 % at normal incidence per surface, and about 8 % for both faces of a pane [1]. Flat soda-lime glass has n_D ≈ 1.520 [3].
- **Double glazing** has four surfaces. That gives roughly 15 % at normal incidence (1 − 0.92², ignoring absorption) [inferred].
- Manufacturer data for clear low-E double glazing (6 mm / 13 mm air / 6 mm) list 11–14 % outdoor visible reflectance [4].
- Reflective coatings reach 26–27 % and read as "silver" [4][5].
- **Forge's values.** The village's F0 of 0.04 is one surface. A real double-glazed window reflects about three to four times that. The towers' F0 of about 0.3 matches reflective coatings [L1][4].
- **At grazing angles,** every interface turns mirror-like [1].
- **Day and night, inside and outside.** Light passes equally both ways. What a viewer sees depends on which side is brighter: the one-way-mirror principle [2]. So:
- by day, a facade with dark rooms reads as a mirror;
- at night, lit rooms dominate;
- from inside at night, the window mirrors the room.

Forge's Fresnel and room glow already follow this [L1]. The problem is uniformity, not balance.

### 1.2 Flatness: roller waves, bow, edge curl, tilt

- **Roller waves (heat-treated glass).** Soft glass sags between the oven's rollers and the waves set as it cools. Reflections show a wavy pattern [6].
- A common criterion is 0.08 mm peak-to-valley at the centre and 0.20 mm at the leading and trailing edges. No formal standard sets these limits [6].
- ASTM C1651 measures depth and wavelength and assumes a sine wave [6][7].
- The wave's period follows the roller pitch. One furnace patent cites rollers about 100 mm apart [8].
- Installing panes with the waves horizontal makes them less noticeable [9].
- **Optical power** is measured in millidiopters, with Ø = 2/R [6].
- Concave glass magnifies reflections; convex glass shrinks them.
- A 20 m radius is about 100 mD; a 4 m radius is about 500 mD.
- One float-glass supplier requires over 95 % of the surface within ±100 mD [6].
- **Edge curl.** Tempered and heat-strengthened panels curl slightly at their edges, which distorts reflections there. Distortion shows most when the glass reflects straight geometric lines [9].
- **Bow of sealed double glazing.** The gas in the cavity was sealed at the factory's temperature and pressure.
- Warmer weather or lower site pressure pushes the panes outwards; cooler weather or higher site pressure pulls them in [10].
- How far they move depends on the pressure difference, the unit's size, its glass and gap thicknesses, and its aspect ratio [11].
- Bowing changes both the reflected and the transmitted image, most visibly with tinted or reflective glass [11][13].
- Wind loads also bend glass [12].
- **A consequence of the bow** [inferred from 6 and 10]. When the cavity is overpressured, both panes bulge away from it. Seen from the street, the outer pane is then convex (shrinking) and the inner one concave (magnifying). Its two reflections differ in scale.
- **Offline simulation.** Eclat Digital's Ocean renders facade glazing from optical data with Monte Carlo rays [14]. Its examples show:
- pillowing that bends a curtain wall's reflected grid (deflection = diagonal/1000);
- roller waves of ±0.15 mm rippling reflections;
- lensing in laminated glass.

### 1.3 Two surfaces, two images

- **Ghost images.** Each pane of double glazing reflects a shifted, dimmer copy. The pane thickness and the air gap set the shift. The ghosts are faint but show against bright sources [15].
- **A single pane** puts its two images only 4–6 mm apart, which merge beyond arm's length [inferred].
- **In double glazing,** the inner pane's images sit about one cavity behind (13 mm in a 6/13/6 unit [4]). The lateral shift is roughly 2·d·tanθ: about 3 cm at 45° [inferred].
- **How bright the ghost is.** In an uncoated unit the inner pane's reflection, after crossing the outer pane twice, is close to the outer pane's in strength (about 0.92² × 8 % against 8 %) [inferred]. A low-E coating makes one surface dominate.
- **When it shows.** With the panes bowed opposite ways (1.2), the two images also differ in magnification. The doubling is then visible on tower facades and up close.
- **Archviz practice** models two panes to get the double reflection [16][17].

### 1.4 Tint and thickness

- **Iron gives float glass a green tinge at its edges,** whatever the sheet's thickness. The cast grows with thickness, which is why frameless designs use low-iron glass. Clear 6 mm glass transmits about 89 % of visible light; low-iron about 90 % or more [18][19].
- **Absorption** follows Beer–Lambert. Filament writes transmittance as e^(−absorption·distance), with absorption = −ln(colour)/distance [20].
- **Viewing angle matters.** The path inside the glass grows as 1/cos θt, so grazing views look greener and darker. Exposed edges (frameless balustrades, shelves) look strongly green [inferred from 18].
- **The refraction offset is negligible.** Snell's law with n = 1.52 gives a lateral shift of about 2 mm through 6 mm glass at 45° [inferred]. Visible "refraction" in windows comes from waviness and old glass (1.7), not from flat panes.

### 1.5 Dirt, smudges, water, cracks

- **Dirt is a thin scattering layer.** Gu, Ramamoorthi, Belhumeur and Nayar (EGSR 2007) model dust, dirt and lipids (fingerprints) on glass as an optically thin layer [21].
- They measured spatially varying thicknesses and scattering parameters on real panes.
- They built a tool to synthesise new contamination patterns.
- The pattern of the dirt matters as much as its optics.
- **What follows for rendering** [inferred from 21]. Dirt:
- lowers the contrast of both the reflection and the view through;
- adds light of its own, scattered;
- glows most when the bright side is behind it: looking out of a dirty window towards the sun, or past a lit room at night.

This is haze, not blur.
- **Where dirt sits:** at edges and corners, in streaks under sills, fingerprints at hand height on doors. This is common observation; no measured source was found [inferred]. Archviz tutorials put dust, dirt and streaks in the reflection and bump channels, and use low-intensity bump maps for fingerprints and slight distortion [17].
- **Rain.**
- *Remember Me* (Lagarde) samples a droplet texture twice, at different offsets and scales. The result drives a distortion, and a low-resolution cubemap restores view-dependent light. Drops on the camera lens cost 0.32 ms on PS3 and 0.54 ms on Xbox 360 [22].
- A forum answer on *The Last of Us* window drips describes normals read from a texture, plus a UV distortion map. The poster's link to Naughty Dog is not confirmed [23].
- **Condensation.** Modern low-E double glazing keeps its outer pane cold, so in spring and autumn it mists on the outside in the morning. One pane can mist while its neighbour stays clear, because of small differences in orientation and nearby trees or buildings [24].
- **Frost:** not researched in depth.
- **Cracks.** The Last of Us Part II added breakable glass. Its shards are built from a texture, and a fractal glass shader extends it [45].

### 1.6 Strain patterns of heat-treated glass

- **What they look like.** Quench marks are faint spots, blotches or lines [25][26]. In architectural glass they often appear as lines; observers call them "tiger stripes" or "leopard spots" [9].
- **When they show:**
- under polarized light, skylight or reflected light;
- at oblique angles, for example a 30–60° reflected view from outside;
- more in thick, tinted or coated glass [25][26].
- **Car windows.** They are common on car rear and side windows seen through polarized sunglasses [26].
- **For Forge:** a subtle cue. Forge has no polarization; a faint pattern modulating reflectance at oblique angles under a clear sky would fake it. Low priority.

### 1.7 Old glass

- **Crown glass** was spun into a disc: thin at the rim, thick at the "bullseye" [27].
- **Research.** Recent work ray-traces the bubbles and chords (streaks of uneven refractive index) of crown and cylinder glass in real time [28], and the caustics behind deformed old panes [29].
- **For Forge:** only historic buildings need this.

### 1.8 Variation between neighbouring panes

Physical reasons panes differ:
- their bow depends on their size and aspect ratio [11];
- heat treatment varies [6][9];
- condensation forms on one pane and not the next [24];
- cleaning and replacement differ [inferred].

Games answer with per-window variation of what is behind the glass: City Sample's windows [32][33], SCS's "Randomize Rooms" [31], and Spider-Man 2's curtain variations [37]. None of the game sources found varies the pane itself (its bow, haze or tint). That gap is where Forge can gain.

### 1.9 What matters most, and where

Forge's own estimate, ranked from the sources above [inferred]:

| Cue | Street distance (10–100 m) | Up close (under 5 m) |
|---|---|---|
| Pane-to-pane differences in reflection (bow, tilt, waves) | **highest**: straight reflected lines break differently in each pane [6][9] | high |
| What is behind varies (curtains, blinds, lit rooms) | **highest** [31][32][37] | medium |
| Overall haze per pane and per building | high (lower reflection contrast) | high |
| Correct F0 per glazing, Fresnel, day/night balance | high | medium |
| Sun glints, distorted by the bow | high at low sun | medium |
| Smudges, fingerprints, dust at edges and corners, streaks | low (sub-pixel) | **highest** |
| Double image from double glazing | low (towers excepted) | medium |
| Green tint at grazing angles and on edges | low | medium (frameless glass) |
| Refraction offset of flat glass | none | very low |
| Condensation, rain, frost | weather-dependent | high when present |
| Strain patterns | very low | low |

## 2. How shipped games and offline rendering do it

| Title | What it does to glass | Source quality |
|---|---|---|
| **The Matrix Awakens / City Sample (UE5)** | City Sample's windows are interiors made from HDRIs with a parallax effect; some windows are lit by emissive materials. `M_Window` has an "Interior" section with an exposure parameter [32]. Epic described the windows as emissive rather than point lights [35]. A developer who reused the window material on new meshes found every window used the same seed. The data had been built in Houdini [33]. Buildings come from volumes styled by a shape grammar, each made of hundreds of instances [34]. No source describes the glass's own reflection or grime. | blogs [read], a forum post [read], Epic docs [read] |
| **Marvel's Spider-Man (2018)** | Rooms behind windows are very likely interior mapping. Insomniac never explained it; van Dongen inferred it from artefacts such as a door where a window should be [30]. Digital Foundry noted visible interiors; reflections mixed screen-space and cube maps [40]. | [read] / [search] |
| **Marvel's Spider-Man 2** | One artist's portfolio describes several pieces [37]: a revamped window shader, curtain variations using depth offset, and parallax-occlusion interiors with emissive cubemaps from a custom baking pipeline. These are separate from "RT interiors". The PC version offers ray-traced interiors and ray-traced reflections on the city's glass [38]. Digital Foundry (PS5 Pro) saw low-resolution textures and simplified objects in reflections [search]. | [search] / [read] |
| **Cyberpunk 2077** | Ray-traced reflections on opaque and transparent surfaces, one bounce [44]. Nothing found on window interiors. | [read] |
| **The Last of Us Part II** | Breakable glass: shards built from a texture, then a fractal glass shader [45]. The studio's technical-art talk names glass and rain among the game's hard surfaces [46]. For the first game's window drips, see 1.5 [23]. | [read] |
| **Red Dead Redemption 2** | No primary source found on its glass. | – |
| **Hitman 3 (Glacier)** | Window panes show live reflections, which players use to spot guards behind them [41]. PCGH, to its knowledge, describes the method as a re-render of the scene into a texture of reduced resolution. With ray tracing on, ray-traced reflections replace blurry cube maps on storefronts and car windows; large surfaces such as the Dubai skyscraper windows keep planar reflections [42]. | [read] |
| **Alan Wake 2 (Northlight)** | Opaque and transparent surfaces are separate "main surfaces". The PS5 Pro version adds ray-traced reflections on transparent surfaces, with a dedicated denoiser for them [43]. | [read] |
| **Forza Horizon** | Forza Horizon 6 adds window decals (from sun strips to full wraps) with adjustable opacity, and a refraction shader for headlights [47]. Forza Motorsport mixes screen-space, planar, cube-map and ray-traced reflections [75]. | [read] / [search] |
| **Half-Life: Alyx** | No source found on its glass. Valve's GDC 2015 VR talk covers specular aliasing, which bears on detailed glass normals [50]. | [search] |
| **Battlefield V / Frostbite** | DICE's ray-traced reflections talk (GTC 2019) [48]. EA's coverage says transparent objects get reflections. Frostbite's stochastic SSR (SIGGRAPH 2015) handles per-pixel roughness: it traces at half resolution, reuses neighbours' rays, and filters spatially and temporally [49]. | [search] |
| **SCS Software (Euro Truck Simulator 2, American Truck Simulator)** | Parallax interiors, with each room a render on the inside of a box [31]: three lighting renders for times of day plus a normal map; a tool that groups faces into windows; a "Randomize Rooms" button; per-window glass on or off; atlases by building type and price range. The illusion breaks at some angles. | [read] |
| **Film: Big Hero 6** | A "Virtual Window Shader" in a ray-traced GI renderer gave parallax interiors to a city of about 83,000 buildings [51]. | [search] |
| **Architectural simulation (Ocean)** | Measured-data Monte Carlo glazing: pillowing, roller waves, lensing [14]. | [read] |
| **Archviz practice** | Fresnel at IOR about 1.5; low-intensity bump for fingerprints and slight waviness; dirt in the reflection and bump channels; two panes for the double image [16][17]. | [search] |

**Lessons:**
- Games buy credibility with variation per window and with correct reflections.
- Pane flatness and grime are left to offline work and archviz.
- Forge already has the expensive part (ray-traced mirror rays and reflection history), so the cheap per-pane parts are where it gains most.

## 3. Techniques, their cost and their shimmer risk

Costs are estimates at 1600 × 900 on the RTX 5070 Ti. They are scaled from Forge's measured glass zones (section 0) and are not measured [inferred].

| Rank | Technique | Credibility gained | GPU cost (estimate) | Shimmer risk |
|---|---|---|---|---|
| 1 | Per-pane condition from a position hash (bow sign and size, tilt, haze, glazing type) | high | ~0 (arithmetic) | none by itself |
| 2 | Bow and tilt as an analytic normal; reflection rays follow it | high at street distance | ALU; rays a little less coherent: +0–30 % of `glass/reflections` and `shading/reflections`, ≤ +0.05 ms (the water's rays: 0.13 ms flat, 0.17 ms with waves [L3]) | low (low frequency), but see the history (3.3) |
| 3 | Geometric specular AA on the bent normals | enabler | ~0.002 ms | lowers the risk |
| 4 | Haze level per pane (contrast loss plus scattered light) | high | ~0 | none |
| 5 | F0 by glazing type, tint by path length | medium | ~0 | none |
| 6 | Grime textures in pane space plus edge and corner masks, inside and outside sets | high up close | +0.005–0.02 ms (`glass/draw` is 0.005 ms) plus about the same on faked-room glass | low if features stay several pixels wide and mips are right; see the DLAA pitfall (3.11) |
| 7 | Sun glints through the mirror direction | high at low sun | ~0 | **high without AA** |
| 8 | Roller waves (analytic sine, orientation per building) | medium (grazing, up close) | ALU | medium: fade by footprint |
| 9 | Fingerprints, stickers, tape, cracks as per-pane decals | medium up close | ≤ 0.01 ms | low |
| 10 | Varied coverings (blinds at random heights, frosted bathrooms) | high at street distance | ~0 | low |
| 11 | Second-pane (inner) reflection near the camera | low to medium | up to +100 % of `glass/reflections` in range (≤ +0.14 ms in the worst village view), plus one more history image | low if it has its own history, high if not |
| 12 | Blurred reflections for frosted or smudged glass via a spatial blur of the mirror result | medium | +0.02–0.05 ms | low if not temporal |
| 13 | Rain streaks, condensation, frost | when present | small | low to medium |
| 14 | Old glass (normal map, bubbles, refraction of the view through) | for historic buildings | needs a colour copy, ~0.02 ms | medium to high |

### 3.1 The per-pane condition

- **One hash per pane,** drawn from where it stands, as the faked rooms already do [L2]. Never from its index: a city that gains a pane would reshuffle every window after it.
- **From the hash:**
- haze (0 freshly cleaned … 1 neglected);
- bow sign and size;
- two small tilt angles;
- grime texture layer, offset, rotation and flip;
- curtain or blind state;
- frosted or not, by the room's use.
- **Per building rather than per pane:** roller-wave orientation and strength, coating tint, and a baseline cleanliness (an office cleaned weekly against an abandoned house).
- **Data-driven:** the material row names the glazing type and a grime set [inferred].
- **Clear-glass panes** need their rectangle in pane space (origin, two axes, size). `GlassVertex` has no UVs today; the `panes` buffer could carry it [L1].

### 3.2 Grime and haze

- **Model it as haze h ∈ [0, 1], not blur** (1.5) [inferred from 21]:
- reflection: `R' = lerp(R, hazy, h·k_r)`, where `hazy` is the sky's irradiance term `sky_reflection` already uses;
- view through: `T' = T·(1 − h·k_t)`;
- scattered light: `S = h·albedo·(E_front·p_back + E_back·p_forward)`, with forward scattering stronger. E_back is the irradiance from the far side: the room's glow or probe from the street, the sky and sun from inside.
- **The mask** is the product of three parts:
- an edge-and-corner term from the distance to the pane's frame in metres, heavier at the bottom;
- a CC0 smudge or dust texture in pane space, low frequency, a random layer and rotation per pane;
- on outer faces only, a vertical streak texture.

Glass is drawn from either side and knows which [L1]. Fingerprints belong inside or on door glass; streaks and water spots outside [inferred].
- **Avoid the "sliding grime" bug.** `steadied_reflection` stores the request's weight × the hit's difference [L2]. If grime lowers that weight per pixel, the history reprojects the grime with the reflection's virtual point, and the grime slides. Keep the request's weight free of grime, and multiply by (1 − h·k_r) after the history, when the trace writes the colour.
- **Sun visibility for the scattered term.** Start with one value per pane (one shadow ray per pane centre, or the probes) [inferred]. Per-pixel shadow rays cost less than mirror rays [L3] if partial shadow across a pane matters.

### 3.3 Waviness and bow as normals

**Bow (pillowing).**
- Model the pane as `h = δ·cos(πx/W)·cos(πy/H)`, zero at the frame.
- Its optical power at the centre is `Ø = 2δ(π/W)²` [inferred, with Ø = 2/R from 6]. A 1 m pane bowed 2–5 mm gives about 40–100 mD, inside the ±100 mD band one supplier accepts [6].
- The largest reflected-ray deviation is 2·δπ/W: about 1.8° for 5 mm on 1 m. That is enough to shift a building 100 m away by about 3 m in the reflection [inferred]. This "fun-house" effect is what real facades show.
- The inner pane of double glazing bows the other way (1.2).

**Tilt.**
- No source gives magnitudes. Start near 0.1–0.3° and tune by eye [inferred].

**Roller waves.**
- `h = (PV/2)·sin(2πx/λ)`, with peak power `Ø = PV·(2π/λ)²`.
- With PV 0.08 mm: λ ≈ 180 mm gives about 100 mD, λ ≈ 300 mm about 35 mD [inferred from 6][8].
- One orientation per building, preferably horizontal [9].

**Old glass.**
- Higher amplitude and shorter wavelength. It needs a normal map and specular AA (3.4) [28].

**Stability.**
- These normals are analytic and low-frequency.
- Fade each term's amplitude once its wavelength covers fewer than about 8 pixels, and add its slope variance to the lobe [inferred].
- The reflection history assumes a flat mirror's virtual point (`steadied_reflection`) [L2]. Curvature moves the true virtual image, so more history texels fail the one-pixel test, and a rejected history shows the jitter. Keep the bow gentle (R ≥ ~20 m on ordinary panes) and measure the rejections.

### 3.4 Specular anti-aliasing for detailed glass normals

- **Normal-map detail** (old glass, droplets, decals): **Toksvig**. The shortening of an averaged normal measures normal variance and widens the lobe [52]. No patent found.
- **Do not use LEAN/LEADR.** LEAN mapping is patented (section "Patents").
- **Analytic normals** (bow, waves): **geometric specular AA**, after Kaplanyan, Hill, Patney and Lefohn (HPG 2016) and Tokuyoshi & Kaplanyan (I3D 2019, JCGT 2021) [53][54].
- The method filters the NDF over the pixel footprint using derivatives of the normal.
- Filament exposes it with a screen-space variance of 0.15 and a threshold of 0.2 by default [20]. Unity HDRP has the same two parameters [55].
- **A sketch, after Filament's normal filtering** [recalled; check the source before use]:

```
float3 du = ddx(n), dv = ddy(n);                  // the bent normal
float variance = SIGMA2 * (dot(du, du) + dot(dv, dv));   // SIGMA2 ≈ 0.15
float kernel = min(2.0 * variance, THRESHOLD);           // THRESHOLD ≈ 0.2
float a2 = saturate(alpha * alpha + kernel);             // alpha = roughness²
```

Convert back to Forge's Blinn-Phong power with `RenderLayer::roughness_for_power` [L2].
- **Where the derivatives come from:**
- `glass.slang`'s fragment shaders have `ddx`/`ddy`.
- The visibility resolve has analytic position derivatives (`dpdx`, `dpdy`) [L2][L7].
- The trace pass is compute. The request pass should store the widened lobe beside the ray direction (`RequestTargets.sun` is unused today) [L1].
- **What the lobe widens:** only the lobe (the sun glint, and the share blended to the sky's irradiance). The mirror ray stays one deterministic ray in the bent mirror direction.

### 3.5 Sun glints

- **The cue.** Real windows flash in a low sun, each pane at a different spot because of its bow. Forge's clear glass shows no sun today [L1].
- **No extra ray needed.** Put the sun's disc, widened by the pane's lobe from 3.4, into the sky term in the mirror direction. Put the same term into the trace pass's subtracted `mirror` value, so a hit hides the glint. The mirror ray is the sun's shadow test for a mirror [inferred].
- **Clamp the peak.** Build this only after the AA passes its shimmer tests.

### 3.6 Two-surface reflections

- **What it takes.** A second deterministic ray from the inner pane, using the inner pane's normal (opposite bow), weighted by about T² × F of the inner pane [inferred].
- **It needs its own history.** The two images sit at different virtual depths, so one history cannot steady both. That adds an RGBA16F image of about 11.5 MB at 1600 × 900, and a second reprojection.
- **Limit it to near panes** (under about 10–15 m) and fade it out.
- **Low priority:** most viewers only see the double image against bright edges [15].

### 3.7 Blurred reflections that stay stable

- **Don't sample GGX stochastically on glass.** It brings noise, and the glass layer has no denoiser [inferred]. Forge's `mirror_ray_share` comment records how a rough floor's sampled rays streaked [L2].
- **Smudges:** the haze of 3.2 is enough: deterministic contrast loss towards the irradiance.
- **Frosted or obscure glass:**
- *Over faked rooms:* replace the room with its mean colour (`interior_room` already fades far rooms into their mean [L2]) and keep a hazy reflection.
- *Over real rooms:* a diffuse transmission of the room's glow and probe irradiance, with the pane opaque (alpha 1). A real blur of the room needs a copy of the colour target.
- **If true glossy reflections are wanted later:** blur the deterministic mirror result spatially, with a radius set by roughness × hit distance and stopping at the pane's edge. This is not temporal [inferred], in the spirit of mip-chain cone tracing.
- **Prefiltered probes** (the split-sum's mips by roughness) are the classic stable fallback [56]. Forge's probes store irradiance, not prefiltered radiance [L2].

### 3.8 F0 by glazing type and tint by thickness

| Glazing | F0 (a single surface in the shader) | Source |
|---|---|---|
| Single glazing | 0.08 | two faces, about 4 mm apart [1] |
| Uncoated double glazing | about 0.15 | [inferred]; low-E data say 0.11–0.14 [4] |
| Reflective coating | 0.26–0.30 | [4][5] |

- **Raise F0 only together with the variation of steps 1–2.** Stronger, identical mirrors would look even more perfect.
- **Tint:** `T = exp(−σ·t / cos θt)`, with σ fitted to clear glass (about 3 % absorbed through 6 mm, from 89 % VLT minus about 8 % Fresnel loss) [inferred from 18]. That is one `exp` per pixel.
- **Exposed edges** of frameless glass need edge geometry coloured by the path length [inferred].
- **Tower glass.** Its roughness of 0.285 is a microroughness that real coated glass does not have. Its distortion is low-frequency (bow, tilt) [6][14]. A/B test: lower the roughness to about 0.05 and let the bow and the AA provide the blur. Do it only after step 1's AA, because a sharper sun lobe without AA will alias [inferred].

### 3.9 Interiors and the dimming of reflections

- **Forge already gets the balance right** through Fresnel and the room glow [L1].
- **The gain is in variation behind the glass:**
- blinds at random heights (a cut plane in the interior mapping);
- curtains open or closed (already hashed for faked rooms);
- frosted bathroom panes;
- lights on or off at night, from the rooms' lamps (D-065), not from a facade-texture random test (see Bosch under "Patents").
- **SCS's practice:** three lighting renders per room by time of day [31]. City Sample makes some windows emissive [32].

### 3.10 Decals on glass

- **Forge's decals don't reach the glass.** They apply in the resolve. The clear glass is drawn after it as its own layer, so its decals must live in the glass shader: a short per-pane list (array layer, rectangle, rotation) [L7].
- **What goes in it:** fingerprints near door handles, opening-hours and alarm stickers, tape residue, cracks for abandoned buildings.
- **Forza Horizon 6** shows that players read stickers on glass as real [47].

### 3.11 The DLAA question for a layer drawn over the room

- **The problem.** The clear glass writes no depth, and DLAA reprojects the room through it using the room's depth [L1]. Anything that sits on the glass gets the room's motion: grime, stickers, the glass's own edges. Strafing past a window 3 m in front of a room 4 m deep will smear the grime [inferred].
- **Fixes, in order:**
1. Keep grime low-contrast and test.
2. Write the glass plane's motion where the glass's own detail dominates the pixel.
3. Flag those pixels in DLSS's bias-current-colour mask, which marks pixels it may mishandle when they move; NVIDIA's guidance is to set it where heavy ghosting appears [58]. FSR 3.1 has the reactive mask (alpha of blended objects, clamped near 0.9) and the transparency-and-composition mask for reflective surfaces [57].
- **Unreal has the same problem.** Its translucent materials need a velocity-output option for TSR, and ghosting through translucency is a known issue there [59].
- **Faked-room glass** is in the visibility buffer with the glass plane's own motion, so grime there reprojects correctly [L2].

## 4. Free assets

| Source | What | Licence |
|---|---|---|
| **ambientCG** [60][61] | Surface Imperfections (20 assets: smears, stains, dust, water marks, coffee rings), Smear (8: fingerprints, wipes), Fingerprints (9), Leaking (39: droplets and flowing water, for rain streaks). 1K to 4K, JPG or PNG. | **CC0 1.0**; credit optional |
| **ShareTextures** [63] | Surface imperfection decals (e.g. Decal 18: glossiness, normal, opacity at 1K) | **CC0** |
| **TextureCan** [64] | "Fingerprint Imperfection Texture (Imperfection 0002)", 2K/4K | the site's header says CC0; its licence page was not read |
| **Poly Haven** [62] | HDRIs and materials (useful for checking reflections) | **CC0** |
| Poliigon "Hand Smudges" | smudges | not CC0 (paid catalogue) [search] |
| "Heartfelt" rain-on-glass shader | rain on glass | **CC BY-NC-SA 3.0**: not for commercial use [65] |

- **Ask the owner before downloading anything.** For glass, only the roughness, opacity or mask channel is needed. Pack R8 layers into one 2-D array image, as `decals.md` recommends for decals [L7].
- **Credit** in `CREDITS.md`, even for CC0.

## Recommendation for Forge

Costs are estimates for 1600 × 900 on the RTX 5070 Ti, scaled from section 0's measured zones. They are to be measured.

### Step 1: every pane different, without textures

- **What to build:**
- A shared Slang module, imported by `glass.slang` and the faked-room branch of `meshlet.slang`. It computes the pane's condition from a hash of where the pane stands (3.1).
- The bent normal from the bow, tilt and roller waves (3.3), applied to the mirror direction in `glass/requests` and in the faked-room branch.
- Geometric specular AA on that normal (3.4), stored for the trace pass.
- Haze per pane, with the contrast loss applied after the reflection history (3.2).
- F0 by glazing type and tint by path length (3.8).
- **Cost:** about +0.01 ms in the glass zones; +0–30 % of `glass/reflections` (≤ +0.04 ms) and of `shading/reflections` (≤ +0.05 ms) from less coherent rays; a few hundredths of a ms on faked-room pixels. **Total ≤ about 0.06 ms.**
- **Why first:**
- It answers "too perfect" at street distance.
- It needs no assets.
- It sets up the AA that steps 2–3 rely on.

### Step 2: grime on the glass

- **What to build:**
- CC0 smudge, dust and streak masks (section 4) in pane space, with edge and corner terms and inside and outside sets.
- Scattered light lit from both sides.
- A short per-pane decal list for fingerprints, stickers and cracks (3.2, 3.10).
- **Before building:** the owner's approval to download.
- **Cost:** +0.005–0.02 ms on the clear glass, about the same on faked-room glass; about 10 MB of R8 or RG8 arrays.
- **The DLAA check of 3.11 decides** whether the glass must write its own motion here.

### Step 3: sun glints

- **What to build:** the sun's disc in the mirror direction, widened by step 1's lobe, hidden by mirror-ray hits (3.5).
- **Cost:** about 0.

### Step 4: what is behind varies

- **What to build:** blinds at random heights, frosted bathroom panes, and lights from the rooms' lamps (3.7, 3.9).
- **Cost:** about 0.

### Step 5 (optional): the inner pane's reflection near the camera

- **What to build:** a second ray and its own history (3.6).
- **Cost:** up to +0.14 ms in the worst village view, and one more 11.5 MB image.
- **Build it only** if steps 1–4 leave towers or close-ups looking single-layered.

### Later

- Condensation in the morning (per pane, by orientation [24]).
- Rain streaks and drops [22] (weather is low priority).
- Old glass with Toksvig-filtered normal maps [28][52].
- An A/B of the towers' roughness (3.8).

### Verifying that no shimmer is added

1. **A switch per step.** Console settings (D-069), for example `r.glass.condition`, `r.glass.grime`, `r.glass.glints`, and matching CLI flags for captures. Each A/B is then one run.
2. **Still camera.** `--fixed-step --capture-every 10` under `--dlaa`. Diff consecutive frames with `imgdiff`, before and after each step, in three views:
 - a house front at 5 m;
 - a street at 30 m;
 - a tower facade at 100–300 m at low sun.

 The budget is the owner's 0.05 % of pixels.
3. **Motion.** `--dlaa` with a turn, and a `--dolly` that strafes past a window at 2–4 m. Compare each build against its own `--ssaa` run with `imgdiff --then` on whole frames. The strafe is the test for 3.11.
4. **The reflection history.** A debug count of history texels rejected, with the bow on and off. A clear rise means the bow is too strong (3.3).
5. **Images.** Pass the changed captures with `--expect`, with ꟻLIP numbers, using `tools/map-check.sh` and the city's captures. Use `--fixed-exposure --ev100 12` for A/B pairs.
6. **Stable variation.** Rebuild the city with one building more. Every other pane must keep its look (the position hash).
7. **Time.** In the F1 overlay, read `glass/requests`, `glass/draw`, `glass/reflections`, `shading/standard` and `shading/reflections`, best of three. Use `FORGE_ASYNC=0` for an added pass. Record the numbers in `docs/PROFILE.md`.

## Patents

| Patent | Holder | Jurisdiction | Status | Expiry (estimate) | What it covers | Bearing on this note, and the alternative |
|---|---|---|---|---|---|---|
| US 9,684,975 B2 [68] | Take-Two Interactive (inventors Baker, Olano) | US | active per Google Patents | ~2033-03-26 | **LEAN mapping:** normal-map moments in a plane, their covariance, lighting with the projected half-vector | **Avoid LEAN/LEADR for glass normals.** Use Toksvig (no patent found) or geometric specular AA (no patent found). `trim-sheets.md` lists LEAN as "none found" and should be corrected. |
| US 10,460,504 B2 [69] | NVIDIA | US | active | ~2038-01-26 | Claim 1 (read): a ray through a pixel, the pixel's footprint angle, a curvature measure at the first hit, and a texture LOD from both (ray cones) | Only if reflection hits choose mips by cone width. Alternatives: Igehy's 1999 ray differentials [R1], or LOD from distance alone on flat panes (no curvature step). **Ask before building.** |
| US 11,443,475 B2 [70] | NVIDIA | US | active | ~2040-11-06 | ray cone tracing, curvature from differential barycentrics, LOD across textures | as above |
| US 11,676,326 B2, US 12,229,869 B2 [71] | NVIDIA | US (+WO) | listed active | not read | anisotropic ray cones; refracted ray cones | as above; refracted cones bear on old glass |
| US 12,700,170 B2 [66] | Celsys | US; JP, CN granted; EP pending; KR, WO | granted 2026-08-04 | ~2042-10-21 | full rendering of the user's room, interior mapping for the others, a partly transparent door, swapping as the user moves | Already on record (`building-structure.md`); Forge shows no see-through door. Untouched. |
| US 10,380,790 B2 [67] | Robert Bosch | US | active | ~2035-12-22 | Claim 1 (read): floors as texture-space ranges, a per-pixel pseudo-random lit/unlit test, a window lighting layer | Already on record (`city-generation.md`). Light windows from the rooms' lamps and a world-position hash, not from texture-space floor ranges. |
| US 12,293,452 B2 [L5] | Sony | US | active | ~2039-12-05 | stacked painted room layers with parallax across windows | Already on record. Forge uses ray-cast interior mapping (2008). |
| US 7,129,945 B2 [72] | Bandai Namco | US | **expired** | 2024-07-05 | water drops rendered by mapping the frame-buffer image with preset offsets | Free to use for rain drops. |
| EP 3 886 051 A1 [73] | Saint-Gobain | EP | **withdrawn** (2022) | – | physically based rendering of coated (spandrel) glass from its diffuse reflection spectrum | No restriction. |
| US 9,275,477 B2 [74] | Square Enix | US | not checked | – | roughness estimated from sampled light paths in GI (not AA) | Not met. |

**Nothing found** in today's searches on:
- dirt, grime or edge masks;
- per-instance variation;
- pane bow or roller-wave normals;
- double-glazing reflections;
- tint by thickness;
- Toksvig;
- Kaplanyan et al.'s NDF filtering;
- Tokuyoshi & Kaplanyan's geometric specular AA;
- interior mapping itself (van Dongen 2008 is prior art).

Not searched: patents on screen-space reflections, because this note does not recommend them.

## Sources

**Physics and glass industry**
1. Fresnel equations, Wikipedia. <https://en.wikipedia.org/wiki/Fresnel_equations> — read
2. One-way mirror, Wikipedia. <https://en.wikipedia.org/wiki/One-way_mirror> — read
3. Soda–lime glass, Wikipedia. <https://en.wikipedia.org/wiki/Soda%E2%80%93lime_glass> — read
4. Vitro Architectural Glass, Solarban 60/65/Champagne short-form specifications. <https://www.vitroglazings.com/media/qklhlbxn/solarban-65-short-form-specs-final-1-12-24.docx> — search-recorded
5. Guardian Glass, SunGuard SNR 50 and SNE 50 on clear. <https://www.guardianglass.com/us/en/our-glass/sunguard-snr-50/snr-50-on-clear> — search-recorded
6. IIBEC, "Glass distortion: not such a clear view". <https://iibec.org/publication-post/glass-distortion-not-such-a-clear-view/> — read
7. ASTM C1651, roll wave optical distortion in heat-treated flat glass. <https://store.astm.org/c1651-11.html> — search-recorded
8. US 2017/0029314, "Method of heating a glass sheet for tempering" (rollers about 100 mm apart). <https://patents.justia.com/patent/20170029314> — search-recorded
9. WFM Media, "Curtain wall glass: preserving appearances". <https://wfmmedia.com/curtain-wall-glass-preserving-appearances/> — read
10. Construction Canada, "Insulating glass: compensating for internal pressure changes". <https://www.constructioncanada.net/?p=76966> — read
11. FGIA/IGMA TM-3200-21 coverage (Window & Door; Glass Canada). <https://www.windowanddoor.com/news/fgia-releases-new-technical-manual-ig-units> — search-recorded
12. Cardinal, IG18-12/20 "Distortion in glass products". <https://www.cardinalcorp.com/wp-content/uploads/2023/01/IG18_12-2020.pdf> — search-recorded
13. Pilkington LOF (1997), "Visual distortions in heat treated glass and sealed insulating glass". — search-recorded
14. Eclat Digital, "Virtual prototyping of architectural glazing". <https://eclat-digital.com/virtual-prototyping-of-architectural-glazing/> — read
15. Shih et al., "Reflection removal using ghosting cues", CVPR 2015. <https://openaccess.thecvf.com/content_cvpr_2015/html/Shih_Reflection_Removal_Using_2015_CVPR_paper.html> — search-recorded
16. CGArchitect forum, "Glass reflection on double glazed windows". <https://forums.cgarchitect.com/topic/26639-glass-reflection-on-double-glazed-windows> — search-recorded
17. Novedge, V-Ray tips on realistic glass (imperfections, thickness); Archi Hacks double-pane advice. <https://novedge.com/blogs/design-news/v-ray-tip-achieving-realistic-glass-in-v-ray-essential-tips-for-material-refraction-thickness-and-imperfections> — search-recorded
18. Guardian Glass, float glass manufacturing brochure (green cast at the edges, VLT at 6 mm). <https://www.guardianglass.com/content/dam/guardianindustriesholdings/collateral/usca/brochure_expert_series_EN_doc5_mfg_process_float_glass_2020.pdf> — search-recorded
19. General Glass, low-iron glass data sheet. <https://www.generalglass.com/wp-content/uploads/2022/07/Low-Iron-Glass-Data-Sheet_Final.pdf> — search-recorded
20. Filament, Materials guide (specular AA variance and threshold; absorption, thickness). <https://google.github.io/filament/main/materials.html> — read
21. Gu, Ramamoorthi, Belhumeur, Nayar, "Dirty Glass: Rendering Contamination on Transparent Surfaces", EGSR 2007. <https://diglib.eg.org/handle/10.2312/EGWR.EGSR07.159-170> — search-recorded
22. S. Lagarde, "Water drop 2a – Dynamic rain and its effects" (2012). <https://seblagarde.wordpress.com/2012/12/27/water-drop-2a-dynamic-rain-and-its-effects/> — read
23. Real Time VFX forum, "Last of Us – how are the windows rain drips made?". <https://realtimevfx.com/t/last-of-us-how-are-the-windows-rain-drips-made/2526> — read
24. Pilkington, "Autumn and condensation: the dew point". <https://www.pilkington.com/en/gbl/news/articles/autumn-and-condensation-the-dew-point> — read
25. SAFTI, "Glass quench and strain marks". <https://safti.com/?p=14333> — read
26. Payette, "Quench patterns in architectural glass". <https://www.payette.com/research-innovation/quench-patterns-in-architectural-glass/> — read
27. Crown glass (window), Wikipedia. <https://en.wikipedia.org/wiki/Crown_glass_(window)> — read
28. Josse, Huan, Rousselle, Renaud, "Real-Time Rendering of Old Glass Panes", Digital Heritage 2025. <https://diglib.eg.org/handle/10.2312/dh20253142> — search-recorded
29. "Optimised Light Rendering through Old Glass", WSCG 2023. <https://dspace5.zcu.cz/handle/11025/54432> — search-recorded

**Games and film**

30. J. van Dongen, "Interior mapping: rendering real rooms without geometry", Game Developer. <https://gamedeveloper.com/programming/interior-mapping-rendering-real-rooms-without-geometry> — read
31. SCS Software, "Under the hood: parallax interiors" (2025-02-15). <https://blog.scssoft.com/2025/02/under-hood-parallax-interiors.html> — read
32. Versluis, "How to dim the window lights in Epic's City Sample project" (2024-03). <https://www.versluis.com/2024/03/how-to-dim-the-window-lights-in-epics-city-sample-project/> — read
33. Unreal Engine forum, "Help reusing City Sample interior cubemap on new meshes". <https://forums.unrealengine.com/t/help-reusing-city-sample-interior-cubemap-on-new-meshes/2664448> — read
34. Epic Games, City Sample project documentation. <https://dev.epicgames.com/documentation/en-us/unreal-engine/city-sample-project-unreal-engine-demonstration> — read
35. RedShark News, "This Unreal Engine 5 Matrix Awakens demo is a glimpse into the future". <https://redsharknews.com/this-unreal-engine-5-matrix-awakens-demo-is-a-glimpse-into-the-future> — search-recorded
36. SideFX, "The Matrix Awakens: Generate a World" (no window detail). <https://www.sidefx.com/learn/talks/the-matrix-awakens-generating-a-world/> — read
37. J. Tuason, "Marvel's Spider-Man 2: Window Shader and Pipeline" (ArtStation). <https://jtuason.artstation.com/projects/XgVxQL> — search-recorded
38. PlayStation Blog, Marvel's Spider-Man 2 PC features. <https://blog.playstation.com/?p=400768> — read
39. 80.lv, "Marvel's Spider-Man: a technical postmortem" (GDC 2019 summary; no window detail). <https://80.lv/articles/marvels-spider-man-a-technical-postmortem> — read
40. Wccftech, Digital Foundry's Spider-Man findings (2018). <https://wccftech.com/spider-man-no-visual-downgrade/amp/> — search-recorded
41. PC Games Hardware, Hitman 3 benchmark (render-to-texture reflections). <https://www.pcgameshardware.de/Hitman-3-Spiel-72804/Specials/Release-CPU-GPU-Benchmark-Test-1365529/> — read
42. PC Games Hardware, Hitman 3 ray tracing review. <https://www.pcgameshardware.de/Hitman-3-Spiel-72804/Specials/Hitman-3-Raytracing-Nvidia-Geforce-AMD-Intel-Benchmarks-Review-1396024/> — read
43. Remedy, "Alan Wake 2 on PlayStation 5 Pro – behind the scenes". <https://remedygames.com/article/alan-wake-2-on-playstation-5-pro---behind-the-scenes> — read
44. NVIDIA GeForce News, "Cyberpunk 2077: ray-traced effects revealed". <https://nvidia.com/en-us/geforce/news/cyberpunk-2077-ray-tracing-dlss-geforce-now-screenshots-trailer> — read
45. 80.lv, "How Naughty Dog created the immersive world of The Last of Us Part II". <https://80.lv/articles/how-naughty-dog-created-the-immersive-world-of-the-last-of-us-part-ii/> — read
46. Naughty Dog, "Naughty Dog at SIGGRAPH 2020". <https://www.naughtydog.com/blog/naughty_dog_at_siggraph_2020> — read
47. Shazoo, Forza Horizon 6 window decals and headlight refraction (citing IGN First). <https://shazoo.ru/2026/03/21/181307/forza-horizon-6-polucila-novye-seidery-prelomleniia-na-farax-i-dolgozdannye-nakleiki-na-stekla> — read
48. Deligiannis, Schmid, Uludag, "It Just Works: Ray-Traced Reflections in Battlefield V", GTC 2019. <https://developer.nvidia.com/gtc/2019/video/s91023> — search-recorded
49. T. Stachowiak, "Stochastic Screen-Space Reflections", SIGGRAPH 2015 Advances. <https://www.ea.com/frostbite/news/stochastic-screen-space-reflections> — search-recorded
50. A. Vlachos, "Advanced VR Rendering", GDC 2015. <https://media.steampowered.com/apps/valve/2015/Alex_Vlachos_Advanced_VR_Rendering_GDC2015.pdf> — search-recorded
51. "Visualizing building interiors using virtual windows", SIGGRAPH Asia 2014 (Big Hero 6). <https://doi.org/10.1145/2669024.2669029> — search-recorded

**Rendering techniques**

52. M. Toksvig, "Mipmapping Normal Maps", NVIDIA. <https://developer.download.nvidia.cn/whitepapers/2006/Mipmapping_Normal_Maps.pdf> — search-recorded (PDF not readable here)
53. Kaplanyan, Hill, Patney, Lefohn, "Filtering Distributions of Normals for Shading Antialiasing", HPG 2016. <https://research.nvidia.com/publication/2016-06_filtering-distributions-normals-shading-antialiasing> — read (abstract page)
54. Tokuyoshi & Kaplanyan, "Improved Geometric Specular Antialiasing", I3D 2019; "Stable Geometric Specular Antialiasing with Projected-Space NDF Filtering", JCGT 2021. <https://www.jcgt.org/published/0010/02/02/> — search-recorded
55. Unity HDRP, "Geometric Specular Anti-aliasing". <https://docs.unity3d.com/Packages/hdrp/manual/Geometric-Specular-Anti-Aliasing.html> — read
56. LearnOpenGL, "Specular IBL" (split sum, prefiltered mips by roughness). <https://learnopengl.com/PBR/IBL/Specular-IBL> — read
57. AMD GPUOpen, FidelityFX Super Resolution 3 upscaler (reactive and transparency-and-composition masks). <https://gpuopen.com/manuals/fidelityfx_sdk/techniques/super-resolution-upscaler/> — read
58. Unity scripting reference, `NVIDIA.DLSSTextureTable.biasColorMask`. <https://docs.unity3d.com/ScriptReference/NVIDIA.DLSSTextureTable-biasColorMask.html> — read
59. Unreal forum "TAA limitations" (Output Velocity on translucency); Unreal issue UE-176256 (ghosting through translucency under TSR). <https://forums.unrealengine.com/t/taa-limitations/1853715> — search-recorded

**Assets**

60. ambientCG licence. <https://docs.ambientcg.com/license/> — read
61. ambientCG listings: categories SurfaceImperfections, Smear, Fingerprints, Leaking; asset Fingerprints001. <https://ambientcg.com/view?id=Fingerprints001> — read
62. Poly Haven licence. <https://polyhaven.com/license> — read
63. ShareTextures, Surface Imperfection "Decal 18". <https://sharetextures.com/textures/surfaceimperfection/decal_18> — read
64. TextureCan, "Fingerprint Imperfection Texture (Imperfection 0002)". <https://www.texturecan.com/details/212/> — read (licence page not read)
65. "Heartfelt" shader licence header, CC BY-NC-SA 3.0 (Construct forum; cables.gl). <https://construct.net/en/forum/construct-2/effects-31/effect-rain-fx-133754> — search-recorded

**Patents**

66. US 12,700,170 B2, Celsys. <https://patents.google.com/patent/US12700170B2/en> — read
67. US 10,380,790 B2, Robert Bosch. <https://patents.google.com/patent/US10380790B2/en> — read
68. US 9,684,975 B2, Take-Two Interactive (LEAN mapping). <https://patents.google.com/patent/US9684975B2/en> — read
69. US 10,460,504 B2, NVIDIA (claims read). <https://patents.google.com/patent/US10460504B2/en> — read
70. US 11,443,475 B2, NVIDIA (claims not reached). <https://patents.google.com/patent/US11443475B2/en> — read
71. US 11,676,326 B2 and US 12,229,869 B2, NVIDIA. <https://patents.google.com/patent/US11676326> — search-recorded
72. US 7,129,945 B2, Bandai Namco (expired). <https://patents.google.com/patent/US7129945B2/en> — read
73. EP 3 886 051 A1, Saint-Gobain (withdrawn). <https://patents.google.com/patent/EP3886051A1/en> — read
74. US 9,275,477 B2, Square Enix. <https://patents.google.com/patent/US9275477> — search-recorded
75. Multiplayer.it, Digital Foundry's Forza Motorsport analysis. <https://multiplayer.it/notizie/forza-motorsport-analisi-digital-foundry-evidenzia-limiti-del-ray-tracing.html> — search-recorded

**Recalled, not checked today**

- R1. H. Igehy, "Tracing Ray Differentials", SIGGRAPH 1999.

**Forge's own code and docs**

- L1. `shaders/glass.slang` — local
- L2. `shaders/meshlet.slang` (`shade_standard`'s interior branch, `interior_room`, `sky_reflection`, `mirror_ray_share`, `steadied_reflection`) — local
- L3. `docs/PROFILE.md` (glass zones 2026-10-18; #176; the water's rays) — local
- L4. `docs/research/building-structure.md` (Celsys) — local
- L5. `docs/research/city-generation.md` (Bosch, Sony) — local
- L6. `docs/research/trim-sheets.md` (specular AA; patent table) — local
- L7. `docs/research/decals.md` (decals in the resolve; glass as `RenderLayer::drawn_after`) — local

