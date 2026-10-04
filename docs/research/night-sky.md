# A night sky over the ground (Phase 4's sky, D-046)

Phase 4 lists "a night sky" beside the clouds (`docs/ROADMAP.md`). Today:
- the sky over the island and the city is the atmosphere's (D-023), lit by the sun alone;
- `--day` (#57) runs from sunrise to sunset, and the sky after sunset is the atmosphere's
  twilight going dark;
- the stars and the nebula are `forge_render::Starfield`'s, hashed from the view direction and
  art-directed to read next to sunlit rocks in space (the ballad);
- the automatic exposure (#57) clamps EV100 to a range, so a night either goes black or is
  pushed up with its noise.

Sources were read with WebSearch on 2026-10-04; claims taken from a search result's summary,
not the paper, are marked †.

## 1. What the night sky is made of

**H. W. Jensen, F. Durand, J. Dorsey, M. M. Stark, P. Shirley, S. Premože, "A Physically-Based
Night Sky Model", SIGGRAPH 2001** ([ACM](https://dl.acm.org/doi/10.1145/383259.383306),
[Stanford](https://graphics.stanford.edu/papers/nightsky/)). The reference for a physical
night: the direct appearance of the sky and the light from the Moon, the stars, the zodiacal
light and the atmosphere (confirmed on the Stanford page).
- **Stars:** each with its position, magnitude and temperature (a catalogue).
- **The Milky Way and nebulae:** a processed photograph.
- **Zodiacal light, galactic light, airglow:** from measured data.
- **The Moon:** a geometric model lit by the sun, with measured elevation and albedo maps and a
  BRDF of its own†.
- *Bearing:* the list of layers to choose from. Forge's atmosphere already scatters the light
  of one source (the sun); the Moon is a second, much fainter source through the same tables.

**How bright** ([Wikipedia, orders of magnitude of illuminance](https://en.wikipedia.org/wiki/Orders_of_magnitude_(illuminance));
[J.S. Held, "Nighttime Visibility in Varying Moonlight Conditions"](https://www.jsheld.com/uploads/Nighttime-Visibility-in-Varying-Moonlight-Conditions.pdf)):
- a full Moon high in a clear sky: 0.05–0.3 lux on the ground†, at most about 0.36†;
- a clear moonless night with airglow: about 0.002 lux†; overcast, 0.0001†;
- a natural starlit zenith: 0.2–0.3 mcd/m²†;
- against direct sun at about 100 000 lux, a full Moon is 18–21 stops darker, and a moonless
  night about 25.

## 2. The Moon

**Its BRDF.** The lunar regolith is not Lambertian: it is about as bright at the limb as at the
centre when full, and it brightens sharply towards opposition (the opposition surge).
Lommel–Seeliger, inside Hapke's 1981 model, is the usual photometric law for it†
([A. Kuzminykh, "Physically Based Real-Time Rendering of the Moon", DLR 2021](https://elib.dlr.de/203152/1/Bachelorarbeit_Alexander_Kuzminykh_20210818.pdf);
[Villa et al., AAS 23-122, JPL](https://robotics.jpl.nasa.gov/media/documents/Villa_ea_Image_Rendering_AAS_GNC_2023.pdf)).
*Bearing:* a disc shaded with Lommel–Seeliger and a phase from the sun's and the Moon's
directions is enough at the size the Moon is on screen. An albedo map (public NASA imagery)
would be a download.

**Its light.** As a second directional light with the sun's machinery, at the Moon's
illuminance for its phase: ray-traced shadows (#45) and the probes (#53) take it like the sun.
At night the sun is under the horizon, so the frame traces one light either way.

## 3. Stars

- **A catalogue:** the Yale Bright Star Catalogue, 9 110 stars to about magnitude 6.5, with
  positions, magnitudes and spectral types†. NASA's copy is a US government work†; the
  original is at the [Harvard CfA's catalogue page](http://tdc-www.harvard.edu/catalogs/bsc5.html) (served over http only)
  ([NASA data.gov entry](https://catalog.data.gov/dataset/bright-star-catalog)). Getting it is
  a download, a few hundred kilobytes.
- **Procedural:** `Starfield`'s hashed stars, with magnitudes drawn from a realistic
  distribution and colours from temperature. No download; the sky is believable but not ours.

## 4. Seeing in the dark

**A. G. Kirk, J. F. O'Brien, "Perceptually Based Tone Mapping for Low-Light Conditions",
SIGGRAPH 2011** ([Berkeley](http://graphics.berkeley.edu/papers/Kirk-PBT-2011-08/),
[ACM](https://dl.acm.org/doi/10.1145/1964921.1964937)). The Purkinje shift: as the eye moves
from cones to rods, colours fade and shift towards blue. They map RGB to rod and cone
responses, then back to what a photopic viewer sees as the same colour†. *Bearing:* the
"moonlit blue" of films comes from this; a cheap version is a blend towards a desaturated blue
by the scene's luminance, in the tone curve's pass.

**Exposure.** A physical night at a day's exposure is black. Films and games raise the exposure
at night but keep it a few stops under the day's, so the night still reads as night. With the
owner's rule ("beautiful and playable over realism"), the night is an art-directed exposure range
and a blue shift, not the eye's full adaptation.

## 5. Proposal

See D-046 in `docs/DECISIONS.md`: the questions for the owner, and a first step.
