# The colour of twilight (research, 2026-10-22, #347)

The owner, 2026-10-22: "is it normal for me to see violet / purple like illumination in your
tests?" The capture was a dawn with the sun 4° under the horizon (`--time-of-day 2.0`) at a
fixed EV 2: every surface lavender, the sky pale violet, a warm band at the horizon.

**Short answer.** Part of the sky really is purple at −4°: the "purple light" above the sun's
side peaks near that depth. The light on the ground is not purple. Measured, it stays blue
(about 10 000–15 000 K), with red under green in linear Rec. 709. Forge's lavender comes mostly
from sampling the red channel at 680 nm. At that wavelength ozone absorbs about a quarter of
what it absorbs across the sRGB red channel, and Rayleigh scattering is 20 % weaker. At
twilight, the sunlight that lights the air overhead has crossed hundreds of kilometres of the
ozone layer. Forge's green loses its ozone share there and its red keeps most of its own, so
the overhead sky turns red plus blue: lavender. A single-scattering model of the zenith at −4°
(§4) gives x,y = 0.317, 0.274 (lavender) with Forge's three samples, and 0.266, 0.263 (blue)
with 41 wavelengths.

**Built (2026-10-22, the owner's go):** §6's items 1 and 2. `atmosphere.rs`'s test
`the_zenith_keeps_the_spectrums_colour_down_to_twilight` is §5's CPU test, and failed with the
680 nm values (R/G 0.51 against the spectrum's 0.61 at +45°). Red's Rayleigh is now 7.2e-3 and
its ozone 2.4e-3 per km. Items 3 to 6 are left for later.

Sources were read with WebFetch and WebSearch on 2026-10-22. Claims taken from a search
result's summary, not from the source itself, are marked †. Values read off published figures
are approximate: about ±0.003 in u′ and v′, which is about ±0.1 in the ratios below.

**Units of colour in this note.** Colours are linear Rec. 709 at a D65 white with no white
balance, as Forge renders them. R/G and B/G are ratios of linear channel values. Measured
chromaticities were converted from CIE 1976 u′v′ to CIE 1931 xy and then to linear Rec. 709
with the IEC 61966-2-1 matrix, the one Forge's shaders use. For reference, the Planckian locus
gives R/G, B/G = 0.88, 1.44 at 10 000 K, 0.79, 1.72 at 15 000 K, 0.76, 1.86 at 20 000 K and
0.68, 2.28 at infinite temperature.

## 1. The sky and the ground by sun depth

### Light levels

| Sun | Horizontal illuminance, clear sky | Zenith luminance |
|---|---|---|
| 0° | 759 lux [9]; 585–410 lux at sunset [10]† | ~200 cd/m² [8] (an SQM reading this bright is doubtful) |
| −2° | 189 lux [9] | ~30 cd/m² [8] |
| −4° | 30 lux [9] | 4–8 cd/m² [7][8] |
| −6°, end of civil twilight | 3.4 lux [9]; 3.5–2 lux [10]† | 0.5–0.6 cd/m² [7][8] |
| −9° | ~0.12 lux (between 0.35 at −8° and 0.042 at −10° [9]) | 0.02–0.03 cd/m² [7][8] |
| −12°, end of nautical twilight | 0.008 lux [9][10]† | 0.0015–0.002 cd/m² [7][8] |

- [9]'s table names no source. Its end points agree with the AMS glossary's ranges [10]†.
- **Zenith luminances.**
  - [8] is one evening's SQM-L series, fitted as m = 6.75 + 1.057 × depression (mag/arcsec²,
    0–12°).
  - [7] is ESO-Paranal's V-band fit over more than 2 000 FORS1 images, valid for depressions
    of 5–15°. Paranal is at 2 635 m, and its V twilight is about 30 % darker than a low
    site's. For −4°, [7] is extrapolated by a degree (8.4 cd/m²).
  - Both were converted with L = 10.8 × 10⁴ × 10^(−0.4 m) cd/m² (the SQM conversion [8]).
- **Rates.** Through civil and nautical twilight, the horizontal illuminance falls by about a
  decade per 2.5° and the zenith by about a magnitude per degree.
- **The sun's side.** 20° up on the sun's side, the sky is several times brighter than the
  zenith. Lee and Hernández-Andrés measured about 1 cd/m² there at −7.2° [1].
- **Forge's exposure.** Forge's exposure is 1/(1.2 × 2^EV), so at EV 2 white is reached at
  4.8 cd/m². At −4°, a white horizontal surface (30 lux/π ≈ 9.5 cd/m²) clips at EV 2, and the
  zenith is near white.

### Colours

Lee and Hernández-Andrés measured about 100 clear evening twilights at four sites between 1998
and 2001 with spectroradiometers. They recorded sky radiances 20° and 45° up at the sun's
azimuth, the zenith, and horizontal irradiances [1]. The values below were read off their
figures (Figs. 5, 7, 8, 11).

| Where | Sun | u′, v′ | x, y | R/G, B/G |
|---|---|---|---|---|
| Horizontal irradiance (the ground's light) | 0° | ≈ 0.183, 0.447 | 0.277, 0.301 | 0.72, 1.28 |
| Horizontal irradiance | −2.5° | ≈ 0.190, 0.452 | 0.289, 0.306 | 0.86, 1.23 |
| Horizontal irradiance | −3.4° | ≈ 0.190, 0.432 | 0.275, 0.278 | 0.88, 1.58 |
| Horizontal irradiance | −5.7° | ≈ 0.180, 0.416 | 0.252, 0.259 | 0.67, 1.83 |
| Zenith, on a purple-light evening | −3.5° | ≈ 0.187, 0.410 | 0.257, 0.250 | 0.83, 2.01 |
| Zenith | −6.8° | ≈ 0.171, 0.385 | 0.224, 0.224 | 0.47, 2.43 |
| Purple light, 20° up towards the sun (maxima of 35 evenings) | about −4° | 0.19–0.21, 0.44–0.47 | 0.28–0.33, 0.29–0.33 | 0.9–1.3, 1.0–1.45 |

What these say:
- **The ground's light stays blue.** Through civil twilight it stays near the Planckian locus,
  from 8 000 K to beyond 20 000 K, never more than about one MacAdam step (JND) on its purple
  side. In linear Rec. 709 its red stays under its green (R/G 0.65–0.95) and its blue over it
  (B/G 1.2–1.9). Lee: the horizontal irradiances follow the same curves as the purple light,
  "displaced toward the blue". In clear civil twilight, 35–40 % of the horizontal irradiance
  comes from sky more than 45° up [1].
- **Measured at the ground elsewhere.** Spitschan et al. measured downwelling spectra at a dark
  rural site and in Philadelphia [5]:
  - the share of short wavelengths grows as the sun sinks, from ozone's Chappuis absorption
    filtering out green and yellow;
  - the chromaticity stays on or near the daylight locus;
  - the rural light is bluest in nautical twilight, near −12°. The city's is bluest near −6°,
    after which street lights take over.
- **The zenith stays blue.** On a purple-light evening it was at (0.257, 0.250). Lee and
  Hernández-Andrés noted it as "distinctly purplish", seen by an eye adapted to a bluish
  ambient. Two days later the zenith's curve lay entirely on the blue side of the locus and
  the zenith "looked only blue" [1].
- **The purple light** [1]:
  - After sunset, the sky 20° above the sun's azimuth reddens. Over 35 evenings it started at
    a mean of −1.4°, was purest at −3.9° (median −4.15°) and ended at −7.4°.
  - It is 90–150° wide and sometimes covers the whole sky.
  - It is absent when clouds, even hundreds of kilometres beyond the horizon, keep sunlight
    reddened in the troposphere off the stratosphere overhead.
  - Its spectrum has less energy in the yellows and yellow-greens than in the reds and blues.
  - At a D65 white the more vivid ones are a pale lavender or pink: R equal to G or above, B
    1.0–1.4 times G.
- **"Purple" is relative to the eye** [1].
  - Lee and Hernández-Andrés suggest u′ = 0.19, v′ = 0.44 as the achromatic point of a
    twilight observer, about x,y 0.28, 0.29, or roughly 10 000 K. The purplish twilights they
    measured straddle the locus.
  - Their photographs on daylight slide film show the −4° sky entirely purplish on 6 October
    1997, and blue with an orange horizon on 2 October 1997.
- **The anti-solar side.**
  - The Belt of Venus is a pink band roughly 10–20° up, over the bluish-grey Earth's shadow
    [13]. The dark segment is less than 3° tall and is gone about 30 minutes after sunset
    [14].
  - Lee's hyperspectral measurements found a small or negligible colour difference between the
    dark segment and the sky above the anti-twilight arch. The colour and luminance extremes
    sit at different heights, and both are most vivid at modest aerosol optical depth [12]†.
- **The sun's side near the horizon** [6]:
  - The sky grades from orange-red and salmon at the horizon to blue-grey and blue at the
    zenith.
  - At −3° and 30° up towards the sun, Saito and Iwabuchi's camera model gives R/G 0.75–0.8
    and B/G 1.3–1.4. These are simulated camera channels normalised to equal spectral
    radiance, not Rec. 709.
  - More tropospheric aerosol makes the sky near the horizon darker and bluer. More
    stratospheric aerosol makes the glow redder.
  - Lee and Hernández-Andrés add that a turbid troposphere makes the purple light redder and
    more vivid [1].

## 2. Why the twilight zenith is blue: ozone

- **Hulburt (1953)** found that with the sun on the horizon, about two thirds of the zenith's
  blue comes from ozone's Chappuis absorption (peak near 600 nm). The other third comes from
  Rayleigh scattering [3], as summarised in [2].
- **Lange, Rozanov and von Savigny (2023)** redid the calculation with SCIATRAN [2].
  - Ozone's share of the zenith's blue (its distance from white in xy) is 3 % with the sun 80°
    up, 15 % with the sun 10° up and 66 % with the sun on the horizon (300 DU).
  - At the horizon, the share is 39 % at 100 DU and 76 % at 500 DU.
  - So ozone hardly matters by day and dominates from sunset on.
- **LOWTRAN analysis by Lee and Hernández-Andrés** [1]:
  - Sunlight that crossed only the stratosphere (high tangent heights) adds skylight bluer than
    Rayleigh scattering alone can make, because of the Chappuis absorption.
  - Lower rays add yellower, then redder light.
  - The rest, multiple scattering inside the Earth's shadow, is purplish to blue.
- **Kutz's spectral path tracer** reproduced the deep blue twilight zenith. Its poster notes
  that the zenith would look grey without ozone [23]†. A no-ozone run of §4's model agrees:
  x,y = 0.334, 0.346 at −4°, a yellowish white.
- **Aerosols matter as much.**
  - In Antarctica and at a mid-latitude site, aerosol extinction was about as important as
    ozone for twilight colours. The link between the ozone amount and the colours was weak
    [11]†.
  - Adams, Plass and Kattawar modelled both effects, in single scattering, for suns 0–6°
    under the horizon [4], as described in [2] and [6].

## 3. Why renderers go purple at twilight

1. **No ozone.**
   - Most graphics sky models left ozone out [24], and their twilight zenith turns grey or
     yellowish (§2).
   - Bodare and Sandberg added ozone in 2014 and got "far more realistic colors" at sunset.
     They expected more realism still from fully spectral calculations [24].
   - Elek and Kmoch's sky is fully spectral for that reason [25]†.
2. **Ozone through three wavelengths.**
   - **What Forge uses.** Bruneton's and Hillaire's RGB skies sample 680, 550 and 440 nm and
     use the three radiances as RGB [16][17]. Forge's coefficients are exactly these samples:
     - Rayleigh, (5.802, 13.558, 33.1)e-3 per km, is 33.1e-3 × (440/λ)⁴ at the three
       wavelengths.
     - Ozone, (0.650, 1.881, 0.085)e-3 per km, is Bruneton's cross-sections at 680, 550 and
       440 nm (1.209e-25, 3.5e-25 and 1.582e-26 m²) times a 300 DU tent peaking at
       5.37e18 molecules/m³ [16].
   - **What Bruneton found.** His own evaluation: "using a constant solar spectrum and 3
     spectral radiance samples directly as RGB components significantly decreases the
     accuracy". His demo's precomputed mode computes 15 wavelengths and converts them through
     the CIE functions [15][16]. His cheaper fix scales each channel by a constant computed for
     an assumed sky spectrum, the sun's times λ^−3 [15]. That is a white balance: it cannot
     follow ozone's dip at twilight.
   - **What the sRGB red channel sees.**
     - The channel (CIE 1931 x̄, ȳ, z̄ [26] through the Rec. 709 matrix) peaks at 600–610 nm,
       on the Chappuis peak (5.0e-25 m² at 600 nm).
     - Weighting Bruneton's ozone table by each channel's signed response gives an effective
       cross-section of 4.5e-25 m² for red (3.7 times the 680-nm value), 3.35e-25 for green
       (0.96 times the 550-nm value), and about zero for blue (its negative lobe over 520–650 nm
       cancels its positive one).
     - Weighted the same way, red Rayleigh is 1.24 times the 680-nm value (an effective
       645 nm). Green and blue Rayleigh are close to their samples: §4's model matches the
       spectral B/G to within 5 % with them unchanged.
   - **Why only twilight shows it.** By day the paths are short and the error hardly shows. At
     twilight, the sunlight that lights the air above the observer grazes the ozone layer for
     hundreds of kilometres: an ozone air mass of order 35, about 1.2 optical depths at the
     red channel's effective cross-section. Along such a path, Forge's red keeps about 2.4
     times too much light compared with green. The green loses its ozone share and the red
     does not, and red over a blue sky reads as lavender.
3. **Multiple scattering.**
   - In Lee and Hernández-Andrés's analysis of the sky 20° up at −4°, singly scattered
     sunlight covers only about half of the colour's distance from the infinite-temperature
     point. The rest comes from light scattered several times inside the Earth's shadow, and
     it is purplish to blue [1].
   - Hillaire 2020's table assumes isotropic scattering for the higher orders and the same
     luminance around each point [18][20]†. It was compared with a path tracer [19], but I
     could not read the paper's twilight cases. The 2020 slides say that multiple scattering is
     critical at sunset, or the atmosphere looks yellowish [21]†.
   - Unreal does not evaluate multiple scattering for the second light [22]. At −4°, Forge's
     sun is the second light (its key light becomes the Moon under −3°), but Forge does sample
     the table for it.
   - **Forge's table resolution.** The table has 32 texels across the sun's zenith cosine,
     from −1 to 1, so each texel spans 3.6° near the horizon. At −4° the lookup interpolates
     linearly between texels at −5.4° and −1.8°. If multiply scattered light falls about a
     decade per 2.6°, as the ground's illuminance does [9], the interpolated value is about 3
     times too bright, and 94 % of it is the −1.8° texel's light. This is my estimate from the
     code, not a measurement.
4. **White balance.**
   - Cameras and eyes adapt. With a fixed D65 white (Forge's), twilight looks bluer than an
     adapted eye sees it. Lee and Hernández-Andrés's "purplish" skies are purplish against a
     bluish ambient [1].
   - Bruneton found white balance and tone mapping important for realism. His demo's white
     balance normalises by the sun's sRGB colour [15][16].
   - A white balance can make twilight less blue. It cannot turn a lavender ambient blue, so it
     is not the cause here.
5. **Low light and tone mapping.**
   - **Purkinje shift.** At −4° the real scene is photopic (zenith ~4–8 cd/m², a white ground
     ~10 cd/m²), so a Purkinje shift should be almost off. Forge's (D-046,
     `shaders/tonemap.slang`) fades in below 3 cd/m² and tints towards blue (0.63, 1, 2.1),
     not violet. It is on in this run (`--time-of-day` creates the night sky), so darker
     surfaces get some of it.
   - **Tone curves.** A tone curve that desaturates bright blue towards white along straight
     lines makes it look purple (the Abney effect). AgX's punchy look, Forge's default,
     multiplies saturation by 1.4 and amplifies any cast.
   - These can exaggerate a cast; they do not make one. Measure before the tone curve (§5).

## 4. A model of the zenith: Forge's three samples against 41 wavelengths

**Method.** A scratch calculation in PowerShell, single scattering only:
- The zenith is seen from the ground, through Forge's `AtmosphereParams::earth()`: Rayleigh,
  grey Mie with g = 0.8, and the ozone tent.
- Sunlight is shadowed by the planet, and its paths are marched through the spherical shell.
- **Spectral version:** 380–780 nm every 10 nm, with Bruneton's ozone cross-sections, the
  ASTM G-173 sun [16], the CIE 1931 functions [26] and the Rec. 709 matrix.
- **Three-sample version:** 680, 550 and 440 nm with Forge's sun colour (1, 0.96, 0.9).

Each cell gives the zenith's linear Rec. 709 R/G and B/G:

| Sun | 41 wavelengths | Forge's 3 samples | 3 samples with red Rayleigh 7.2e-3, red ozone 2.4e-3 |
|---|---|---|---|
| +45° | 0.61, 1.94 | 0.51, 1.93 | 0.60, 1.93 |
| 0° | 0.89, 1.34 | 1.21, 1.28 | 0.87, 1.28 |
| −2° | 0.96, 1.52 | 1.55, 1.47 | 0.95, 1.47 |
| −4° | 0.89, 1.81 (x,y 0.266, 0.263) | 1.81, 1.77 (0.317, 0.274) | 0.91, 1.77 (0.269, 0.266) |
| −6° | 0.81, 1.90 | 1.85, 1.87 | 0.83, 1.87 |

- **Spectral against measured.** The spectral zenith at −4° is about 15 000 K, on the locus.
  It is close to Lee and Hernández-Andrés's measured zenith (§1).
- **Forge's samples.** The three-sample zenith is about 12 JND on the purple side of the locus
  (u′ 0.225 against 0.189). That is more purple than the most vivid purple light measured 20°
  up.
- **Without ozone,** the spectral zenith at −4° is 1.16, 0.85 (x,y 0.334, 0.346).
- **Red ozone alone** gives an R/G at −4° of 1.03 at 2.1e-3 and 0.92 at 2.4e-3. Adding the red
  Rayleigh also brings the day zenith to the spectral value.
- **The sun's colour.** Converted through the CIE functions, the spectral sun above the air is
  (1.12, 1, 0.98), against Forge's (1.04, 1, 0.94). Each model keeps its own sun, which
  accounts for about 7 % in R/G.
- **Not modelled.** Multiple scattering is left out. Forge's table is built from the same
  three samples and carries the same bias.

## 5. A check for Forge

### A CPU test (no GPU)

Mirror §4 in `atmosphere.rs`'s tests:
1. With the CPU `transmittance`, integrate the single-scattered zenith at sun +45°, 0°, −2°,
   −4° and −6° for the three channels.
2. Integrate the same at 41 wavelengths, with two tables from Bruneton's demo (the ozone
   cross-sections and ASTM G-173) and the CIE 1931 functions.
3. Assert that the channels' R/G is within 10 % of the spectral one.

By §4's numbers it fails today with the sun at or under the horizon, by a factor of 1.4 at 0°
rising to 2.3 at −6°.

### The village at a fixed exposure

```
city-plan.exe --village 600 --time-of-day 2.0 --fixed-step --frames 240 \
  --capture OUT.png --capture-frame 200 --fixed-exposure --ev100 4 \
  --tonemap neutral --no-purkinje --no-local-exposure [--show-gi --no-probes --no-lamps]
```

- **The ground's light.** `--show-gi --no-probes` draws white surfaces lit by the sky's 9 SH
  coefficients alone (`FLAG_SHOW_GI` in `meshlet.slang`). A horizontal patch's pixel p is then
  E/π × exposure, so E = π × p × 1.2 × 2^EV lux.
- **Undo the tone curve.** Khronos PBR Neutral leaves colours under 0.76 unchanged apart from
  an offset. Before taking ratios, decode sRGB, then let m be the darkest channel: add 0.04 to
  every channel when m ≥ 0.04, otherwise add 0.4√m − m.
- **Choose the exposure.** Keep the patches under 0.76: a white horizontal patch is about 0.5
  when EV ≈ log2(E/1.9). That gives EV 9 at 0°, 7 at −2°, 4 at −4°, 1 at −6°, −4 at −9° and
  −8 at −12° (write `--ev100=-4`).
- **Average** 32×32-pixel patches, since the display pass dithers.
- **Read four places:** the zenith (a view straight up), the sky 20° up at the sun's azimuth,
  the anti-solar sky 10–20° up, and an open horizontal patch.

**Targets** (clear sky, D65, no white balance):

| Sun | Horizontal E | Ground's light R/G, B/G | Zenith L | Zenith R/G, B/G | 20° up at the sun's azimuth |
|---|---|---|---|---|---|
| 0° | 410–760 lux | 0.7–0.9, 1.2–1.4 | ~200 cd/m² | blue | warm |
| −2° | ~190 lux | 0.85–0.95, 1.2–1.3 | ~30 cd/m² | blue (model 0.96, 1.52) | turning purple |
| −4° | ~30 lux (20–40) | 0.85–0.95, 1.4–1.6 | 4–8 cd/m² | 0.8–0.9, 1.8–2.1 | 0.9–1.3, 1.0–1.45 on a purple-light evening, otherwise like the zenith |
| −6° | ~3.4 lux | 0.65–0.9, 1.6–1.9 | 0.5–0.6 cd/m² | ~0.5–0.8, 1.9–2.4 | fading |
| −9° | ~0.12 lux | bluer still [5] | 0.02–0.03 cd/m² | blue | dark |
| −12° | ~0.008 lux | the bluest, at a rural site [5] | ~0.002 cd/m² | blue | dark |

**#347 passes** when, at −4°:
- the ground's light has R/G < 1 and B/G > 1.3;
- the zenith has R/G < 1 and B/G near 2;
- only the sky 10–40° above the sun's side has R ≥ G;
- the luminances and illuminances are within a factor of 2 of the table.

### Which parameter

Change one thing at a time and rerun the check:
1. **The red channel's coefficients** (most likely): ozone red 0.650e-3 → 2.4e-3, Rayleigh red
   5.802e-3 → 7.2e-3 per km.
2. **The multiple-scattering table:** 32 → 256 texels across the sun's angle, or texels packed
   near the horizon.
3. **The aerosols.** Forge's air is very clean: 4.44e-3 per km × 1.2 km gives an aerosol
   optical depth of about 0.005 (Bruneton's default). Ground-based measurements range from 0.04
   to 0.43 [6], and Saito and Iwabuchi's prior is 0.13. More aerosol darkens and blues the
   horizon [6] and makes the purple light more vivid [1]. It does not fix the lavender, but it
   is the next knob for the horizon band.
4. **The display:** `--no-purkinje`, and `--tonemap neutral` against `agx-punchy`.

## 6. Recommendation for Forge

1. **Write §5's CPU test first.** It fails today and pins the cause without a GPU.
2. **Set the red channel to the CMF-weighted values:** Rayleigh red ≈ 7.2e-3 per km (an
   effective 645 nm) and ozone red ≈ 2.4e-3 per km at the tent's peak (an effective
   cross-section of 4.5e-25 m²). Keep green and blue.
   - The day sky loses a little saturation (zenith R/G 0.51 → 0.60, the spectral value), and
     the setting sun gets a little less red.
   - Every outdoor capture changes: run a day check and accept it, and let the night's Tier 1
     run.
   - The thresholds of `earth_lets_most_of_the_noon_sun_through_and_reddens_the_sunset` may
     need a look.
3. **Widen the multiple-scattering table along the sun's angle.** 256 texels cost almost
   nothing, because the table rebuilds only when the atmosphere changes. Alternatively, remap
   it to pack texels near the horizon. Compare the ground's light at −4° before and after.
4. **If twilight still reads wrong, go spectral where it is cheap.**
   - The transmittance and multiple-scattering tables rebuild only with the atmosphere, so
     they can be computed at 15 wavelengths and converted to Rec. 709, as in Bruneton's
     precomputed mode [16]. The per-frame sky-view march would then need more channels or a
     per-channel fit.
   - Effective coefficients are a linearisation. They cannot reproduce ozone's spectral dip,
     which is what gives the purple light its colour.
5. **Keep the fixed D65 white for the check.** Whether twilight should then be shown partly
   adapted, as a camera's white balance and the eye do, is for the owner to decide later.
6. **Leave the aerosols** to the horizon band's look (§5).

## Patents

None met. I searched for patents on precomputed atmospheric-scattering tables with ozone and on
converting spectral skies to RGB, and found none. This is a search, not a clearance. The
techniques here are published papers (Hillaire 2020, Bruneton 2008 and 2017) and measurement
science.

If Forge copies tables for the CPU test or a spectral path, they need lines in `CREDITS.md`:
- Bruneton's code, under the BSD licence [16];
- the ozone cross-sections, the University of Bremen's 2011 spectra at 233 K, as Bruneton's
  demo cites them [16];
- the sun's spectrum, ASTM G-173's extraterrestrial column [16];
- the CIE functions, a CIE data set [26].

## Checked and left out

- Rozenberg, *Twilight* (1966), the classic textbook cited by [1] and [7]: not reachable as
  text.
- Hulburt 1953 [3] and Adams, Plass and Kattawar 1974 [4]: not read. Their findings come
  through [1], [2] and [6].
- Hillaire 2020's PDF and the Frostbite 2016 course notes: over the fetch limit. Their twilight
  comparisons are unread, and the multiple-scattering assumptions come from secondary notes
  [20]†.
- Spitschan et al.'s figures: behind a captcha. Their per-depression chromaticities were not
  taken.
- The AMS glossary pages returned 403. Their values come from search summaries [10]†.
- Hernández-Andrés et al. 2001 (2 600 daylight spectra from Granada): a scanned PDF, not read.
- Lee 2015 [12] and Lee, Meyer and Hoeppe 2011 [11]: abstracts only.
- The Prague sky model (Wilkie et al. 2021) fits skies after sunset [27]†. I did not confirm
  its lowest sun elevation or how it treats ozone.
- Brown's 1952 natural illumination charts, the usual source of twilight illuminance tables:
  not reachable.
- How shipped games handle twilight (white balance by time of day, for example): nothing
  published found.

## Sources

1. R. L. Lee Jr., J. Hernández-Andrés, "Measuring and modeling twilight's purple light",
   *Applied Optics* 42(3), 445–457, 2003. [UGR copy](https://colorimaginglab.ugr.es/pages/pdfs/ao_2003_445/!)
2. A. Lange, A. Rozanov, C. von Savigny, "Revisiting the question 'Why is the sky blue?'",
   *Atmos. Chem. Phys.* 23, 14829–14839, 2023. [ACP](https://acp.copernicus.org/articles/23/14829/2023/)
3. E. O. Hulburt, "Explanation of the brightness and color of the sky, particularly the
   twilight sky", *JOSA* 43, 113–118, 1953. [DOI](https://doi.org/10.1364/JOSA.43.000113)
4. C. N. Adams, G. N. Plass, G. W. Kattawar, "The influence of ozone and aerosols on the
   brightness and color of the twilight sky", *J. Atmos. Sci.* 31, 1662–1674, 1974 (its 1973
   NASA report). [NTRS](https://ntrs.nasa.gov/citations/19730020661)
5. M. Spitschan, G. K. Aguirre, D. H. Brainard, A. M. Sweeney, "Variation of outdoor
   illumination as a function of solar elevation and light pollution", *Scientific Reports*,
   2016. [PMC](https://pmc.ncbi.nlm.nih.gov/articles/PMC4895134/)
6. M. Saito, H. Iwabuchi, "A new method of measuring aerosol optical properties from digital
   twilight photographs", *Atmos. Meas. Tech.* 8, 4295–4311, 2015. [AMT](https://amt.copernicus.org/articles/8/4295/2015/)
7. F. Patat, O. S. Ugolnikov, O. V. Postylyakov, "UBVRI twilight sky brightness at
   ESO-Paranal", *A&A* 455, 385–393, 2006. [arXiv](https://arxiv.org/abs/astro-ph/0604128)
8. "SQM twilight" measurements, hnsky.org, 2017. [hnsky.org](https://hnsky.org/sqm_twilight.htm)
9. P. Schlyter, "Radiometry and photometry in astronomy", §10 (solar illumination on a
   horizontal surface). [stjarnhimlen.se](https://www.stjarnhimlen.se/comp/radfaq.html)
10. AMS Glossary of Meteorology: [civil twilight](https://glossary.ametsoc.org/wiki/civil-twilight/),
    [nautical twilight](https://glossary.ametsoc.org/wiki/nautical-twilight/),
    [astronomical twilight](https://glossary.ametsoc.org/wiki/astronomical-twilight/) †
11. R. L. Lee Jr., W. Meyer, G. Hoeppe, "Atmospheric ozone and colors of the Antarctic twilight
    sky", *Applied Optics* 50(28), F162–F171, 2011. [figshare](https://figshare.com/collections/Atmospheric_ozone_and_colors_of_the_Antarctic_twilight_sky/3748787) †
12. R. L. Lee Jr., "Measuring and modeling twilight's Belt of Venus", *Applied Optics* 54(4),
    B194–B203, 2015. [PubMed](https://pubmed.ncbi.nlm.nih.gov/25967826) †
13. "Belt of Venus", Wikipedia. [Wikipedia](https://en.wikipedia.org/wiki/Belt_of_Venus)
14. G. di Giovanni, "Observation of Earth's shadow over antisolar sky during sunset &
    sunrise", British Astronomical Association, 2025. [BAA](https://britastro.org/journal_contents_ite/observation-of-earths-shadow-over-antisolar-sky-during-sunset-sunrise)
15. E. Bruneton, "A Qualitative and Quantitative Evaluation of 8 Clear Sky Models", *IEEE TVCG*,
    2017. [arXiv](https://arxiv.org/abs/1612.04336)
16. E. Bruneton, "Precomputed Atmospheric Scattering: a New Implementation", 2017: the demo's
    ozone cross-sections, ASTM G-173 sun, luminance modes and white balance
    ([demo.cc](https://ebruneton.github.io/precomputed_atmospheric_scattering/atmosphere/demo/demo.cc.html)),
    BSD licence ([GitHub](https://github.com/ebruneton/precomputed_atmospheric_scattering)).
17. E. Bruneton, the model's API and its three wavelengths. [model.h](https://ebruneton.github.io/precomputed_atmospheric_scattering/atmosphere/model.h.html)
18. S. Hillaire, "A Scalable and Production Ready Sky and Atmosphere Rendering Technique",
    *Computer Graphics Forum* 39(4), EGSR 2020. [EG digital library](https://diglib.eg.org/handle/10.1111/cgf14050)
19. S. Hillaire, the paper's reference implementation and path tracer. [GitHub](https://github.com/sebh/UnrealEngineSkyAtmosphere)
20. Notes on Hillaire's multiple-scattering table (isotropic higher orders, no occlusion beyond
    the second order). [Notion](https://jojo-lyu.notion.site/s2020-Scalable-Atmospheres-in-Unreal-Engine-6cbf50aae15f4701a4c01b589f63c2c1) †
21. S. Hillaire, "Physically Based and Scalable Atmospheres in Unreal Engine", SIGGRAPH 2020
    course slides. [selfshadow](https://blog.selfshadow.com/publications/s2020-shading-course/hillaire/s2020_pbs_hillaire_slides.pdf) †
22. Epic Games, "Sky Atmosphere Component in Unreal Engine". [Epic](https://dev.epicgames.com/documentation/en-us/unreal-engine/sky-atmosphere-component-in-unreal-engine)
23. P. Kutz, "Physically Based Atmosphere Rendering", University of Pennsylvania, 2013,
    poster. [UPenn](https://www.seas.upenn.edu/~cis497/dynamic/projects2013/PeterKutzPoster.pdf) †
24. G. Bodare, E. Sandberg, "Efficient and Dynamic Atmospheric Scattering", master's thesis,
    Chalmers, 2014. [Chalmers](https://odr.chalmers.se/handle/20.500.12380/203057?locale=en)
25. O. Elek, P. Kmoch, "Real-time spectral scattering in large-scale natural participating
    media", SCCG 2010. [Springer Professional](https://www.springerprofessional.de/doi/10.1145/1925059.1925074) †
26. CIE, "CIE 1931 colour-matching functions, 2 degree observer", data set
    10.25039/CIE.DS.xvudnb9b. [CIE](https://cie.co.at/datatable/cie-1931-colour-matching-functions-2-degree-observer)
27. A. Wilkie et al., "A Fitted Radiance and Attenuation Model for Realistic Atmospheres",
    *ACM TOG* 40(4), 2021. [CGG Prague](https://cgg.mff.cuni.cz/publications/skymodel-2021/) †
