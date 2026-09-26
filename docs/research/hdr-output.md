# Research — HDR display output: swapchains, PQ, reference white, ACES 2.0's HDR presets

> Companion to `lighting-gi.md` §5 and §10 (light units, exposure, the display transform) and to its
> "Research for issue #76" notes (ACES 2.0's SDR preset, shipped as the fourth tone curve, D-022).
> Written 2026-09-26 for issue #94: the ballad on an HDR display, switchable at run time, with
> ACES 2.0's HDR presets on an HDR10 swapchain. Every citation was checked that day against a
> reachable page or, where the network proxy refused the host, against the search engine's record
> of it; the distinction is kept per entry under [Verification notes](#verification-notes) for
> issue #99, and what could not be found is under [Checked and left out](#checked-and-left-out).
> Forge's own facts (formats, passes, tables, timings) were read from the code and from D-022, D-024
> and `docs/PROFILE.md` as of 2026-09-26.

The question is what an HDR display path costs and where it plugs into what Forge already has:
which swapchain format and colour space to ask Vulkan for on Windows (a 10-bit PQ image or a
half-float linear one), what Windows does with either (it composes in scRGB and lets the user move
the white of SDR content), what the display transform becomes (`aces2.rs` already takes a peak
luminance and limiting primaries, so the SDR preset becomes one row of a table of presets), how the
UI and the F1 overlay stay readable (drawn at a reference white, not at the display's peak), how a
user without a colorimeter calibrates (three patterns, adjust until a mark disappears), and how any
of it is verified on a machine whose monitor may be SDR (the whole path except the present can
render offscreen). The short answer: the swapchain change is small and the same on NVIDIA and AMD
(`VK_EXT_swapchain_colorspace` at the instance, `VK_EXT_hdr_metadata` at the device,
`A2B10G10R10_UNORM_PACK32` + `HDR10_ST2084_EXT` first, `R16G16B16A16_SFLOAT` +
`EXTENDED_SRGB_LINEAR_EXT` as the fallback and the debugging aid); the shader must do the PQ
encoding itself, two table fetches plus a `pow` pair; the ACES 2.0 side is a rebake of the existing
65³ table with a wider shaper (the transform's own clamp moves from 1024 to 4096 at 1000 nits); and
the part that needs a decision is a look, not a technique: ACES 2.0's 1000-nit preset puts a
diffuse white at 107 nits where Windows' desktop, BT.2408's 203-nit reference and every engine's
"paper white" slider put it higher, so Forge needs a paper-white control in front of the transform,
and the UI must be drawn at that white rather than through the curve.

> **State of the art in five sentences.** On Windows an HDR game presents either 10-bit PQ over
> Rec.2020 (HDR10) or linear half-float scRGB in which 1.0 is 80 nits and 12.5 is 1000, the
> compositor converts both to the wire format, and the only OS input the game must read is the
> user's SDR white level, since SDR content and UI are otherwise drawn at 80 nits and look dim
> (Microsoft's Advanced Color guide). Vulkan exposes the same two pairs through
> `VK_EXT_swapchain_colorspace` on NVIDIA and AMD, leaves the transfer function to the
> application's shaders, and carries the mastering metadata through `VK_EXT_hdr_metadata`, which
> the presentation engine may use or ignore. The display transform every shipped HDR game runs is
> a scene-referred curve parameterised by the display's peak and a user paper white (Frostbite's
> "grade once, output many" display mapper, Unreal's ACES 1000/2000-nit curves, Unity's
> min/max/paper-white triple, Call of Duty's and Sucker Punch's own), followed by the UI composited
> at paper white and a three-pattern calibration screen (HGiG's MinTML/MaxTML/MaxFFTML, Windows'
> HDR Calibration app). ACES 2.0's output transform is the first standard curve built for this: the
> Academy ships presets at 500/1000/2000/4000 nits with P3-D65 or Rec.2020 limiting inside
> Rec.2100 PQ, OpenColorIO 2.4–2.5 implements them as builtins, and Forge's port already passes
> OCIO's 1000-nit P3 test values, so the HDR presets are a parameter change and a rebake. Ten-bit
> PQ steps are about 1 % of luminance above 100 nits (Miller et al. 2013 designed PQ for 12 bits),
> so smooth skies still want the half-code dither Lottes and Wronski describe, in PQ space, before
> the write.

**Contents**

1. [The HDR display pipeline: Windows, Vulkan, PQ, the reference white](#1-the-hdr-display-pipeline-windows-vulkan-pq-the-reference-white)
2. [Tone mapping for HDR displays: ACES 2.0's presets and the engines' curves](#2-tone-mapping-for-hdr-displays-aces-20s-presets-and-the-engines-curves)
3. [What professional engines do](#3-what-professional-engines-do)
4. [The interplay with what Forge has](#4-the-interplay-with-what-forge-has)
5. [Verifying without an HDR monitor](#5-verifying-without-an-hdr-monitor)
6. [Recommendation for Forge](#recommendation-for-forge)
7. [What the numbers say](#what-the-numbers-say)
8. [Checked and left out](#checked-and-left-out)
9. [Verification notes](#verification-notes)

---

## 1. The HDR display pipeline: Windows, Vulkan, PQ, the reference white

Two encodings reach the wire on Windows, and the operating system sits between the swapchain and
the panel in both cases. What the game controls is the swapchain's format and colour space, the
values it writes, and (optionally) a block of metadata; what the OS controls is the composition,
the SDR white level and, on laptops, the brightness.

**The Khronos Group. `VK_EXT_swapchain_colorspace`, revision 5 (2024-03-16), and the
`VkColorSpaceKHR` chapter of the Vulkan specification.** [spec] [still-current]
<https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_swapchain_colorspace.html>
(source: <https://github.com/KhronosGroup/Vulkan-Docs>, `chapters/VK_KHR_surface/wsi.adoc` and
`appendices/VK_EXT_swapchain_colorspace.adoc`)

An instance extension that "expands VkColorSpaceKHR to add support for most standard color spaces
beyond VK_COLOR_SPACE_SRGB_NONLINEAR_KHR". `VK_COLOR_SPACE_HDR10_ST2084_EXT` "specifies support for
the images in HDR10 (BT2020) color space, encoded according to SMPTE ST2084 Perceptual Quantizer
(PQ) specification"; `VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT` "specifies support for the images in
extended sRGB color space, encoded using a linear transfer function" (scRGB: Rec.709 primaries,
D65, values above 1 allowed). The chapter's table gives the HDR10 primaries as 0.708/0.292,
0.170/0.797, 0.131/0.046 with a D65 white. Two resolved issues fix the contract: "Pixel format is
independent of color space (though some color spaces really want / need floating-point color
components to be useful) ... An application can: call vkGetPhysicalDeviceSurfaceFormatsKHR to query
what a particular implementation supports", and "Extension indicates that implementation must: not
do the OETF encoding if it is not sRGB. That responsibility falls to the application shaders."
*Bearing:* Forge's `Instance::new` enables only the window's surface extensions and debug utils
(`crates/forge-gpu/src/instance.rs`), so `Swapchain::recreate`'s query today cannot list an HDR
pair at all; enabling this extension is step one. `display_output` in `shaders/tonemap.slang` must
PQ-encode itself, exactly as it sRGB-encodes for UNORM targets now.

**The Khronos Group. `VK_EXT_hdr_metadata` (last modified 2024-03-26) and `VkHdrMetadataEXT`.**
[spec] [still-current]
<https://docs.vulkan.org/refpages/latest/refpages/source/VkHdrMetadataEXT.html> (source:
`appendices/VK_EXT_hdr_metadata.adoc` and `wsi.adoc` in Vulkan-Docs)

A device extension that "defines two new structures and a function to assign SMPTE (the Society
of Motion Picture and Television Engineers) 2086 metadata and CTA (Consumer Technology Association)
861.3 metadata to a swapchain": the display primaries, white point, `maxLuminance` and
`minLuminance` of "the display used to optimize the content", `maxContentLightLevel` ("the value in
nits of the desired luminance for the brightest pixels in the displayed image") and
`maxFrameAverageLightLevel` ("the value in nits of the average luminance of the frame which has the
brightest average luminance anywhere in the content"); "If any of the above values are unknown,
they can: be set to 0." The spec is explicit that "This extension does not define exactly how this
metadata is used", that "Presentation engines may process the image based on the metadata before
displaying it, resulting in the image being modified outside of Vulkan", that "The metadata does
not override or otherwise influence the color space and color encoding", and that it is optional;
it persists "until a subsequent vkSetHdrMetadataEXT".
*Bearing:* set it once per swapchain (Rec.2020 primaries, D65, the preset's peak as `maxLuminance`
and MaxCLL, a MaxFALL from the frame's own histogram later), expect Windows' compositor to mostly
ignore it in windowed mode, and never rely on it for the look: the tone mapping is Forge's, to the
peak Forge chose. `ash` 0.38 already ships the loader (`ash::ext::hdr_metadata::Device::
set_hdr_metadata`) and the colour-space enums.

**Advanced Micro Devices. `VK_AMD_display_native_hdr` (2018-12-18; contributors include Timothy
Lottes); GPUOpen, "Using AMD FreeSync Premium Pro HDR Color Spaces / Tone Mapping / Gamut
Mapping" and the FreeSync Premium Pro sample (DX12 and Vulkan); Cauldron's `FreeSyncHDR.cpp`.**
[spec] [docs] [code] [still-current]
<https://gpuopen.com/learn/using-amd-freesync-2-hdr-color-spaces/> ·
<https://github.com/GPUOpen-LibrariesAndSDKs/FreesyncPremiumProSample> ·
<https://github.com/GPUOpen-LibrariesAndSDKs/Cauldron/blob/master/src/VK/base/FreeSyncHDR.cpp>

The AMD extension adds "A new VkColorSpaceKHR enum for setting the native display color space"
and "Local dimming control"; GPUOpen's argument is latency: "games tone and gamut map their frames
to a standard HDR color space, and the monitor then also tone and gamut maps these frames to its
native color space", so FreeSync HDR has "the game do the tone and gamut mapping directly to the
monitor's native color space". Cauldron's Vulkan code is the useful part for a vendor-neutral
engine, because it shows what AMD's driver lists beside its own mode: HDR10 is
`VK_FORMAT_A2R10G10B10_UNORM_PACK32` *or* `VK_FORMAT_A2B10G10R10_UNORM_PACK32` with
`VK_COLOR_SPACE_HDR10_ST2084_EXT`, "HDR10 scRGB" is `VK_FORMAT_R16G16B16A16_SFLOAT` with
`VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT`, and the FreeSync modes use the same formats with
`VK_COLOR_SPACE_DISPLAY_NATIVE_AMD`, reading the monitor's metadata through
`VkSurfaceCapabilities2KHR → VkDisplayNativeHdrSurfaceCapabilitiesAMD → VkHdrMetadataEXT`. For
HDR10 it sets the Rec.2020 primaries, D65, `minLuminance` 0, `maxLuminance` 1000 ("This will cause
tonemapping to happen on display end as long as it's greater than display's actual queried max
luminance"), MaxCLL 1000 and MaxFALL 400. The sample's readme: "Freesync and HDR do not work in
window mode" (for the FreeSync modes) and "Every time display mode is changed, swapchain needs to
be recreated".
*Bearing:* the cross-vendor rule (#67) is satisfied by the standard pair alone: the same query,
formats and metadata call work on the RX 9070 XT with no AGS; the AMD-native colour space is an
optional third mode for FreeSync Premium Pro monitors, not a dependency. Accept either channel
order of the 10-bit format, since the shader writes a `float4`.

**Microsoft. "Use DirectX with Advanced Color on high/standard dynamic range displays." Win32
apps documentation, 2022-10-10.** [docs] [still-current]
<https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range> (source:
<https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/direct3darticles/high-dynamic-range.md>)

The document that fixes what Windows does. "When in HDR mode, the Desktop Window Manager (DWM)
uses a canonical composition color space (CCCS) defined as: scRGB color space (BT.709/sRGB
primaries with linear gamma), IEEE half precision (FP16 bit depth)"; "The DWM converts each app
from its native color space to CCCS before blending" and "The display kernel converts the OS
framebuffer from CCCS to the wire format color space (BT.2100 ST.2084)". On an HDR display 1.0f is
interpreted "As 80 nits (nominal reference white)": "scRGB (1.0, 1.0, 1.0) encodes the standard D65
white at 80 nits; but scRGB (12.5, 12.5, 12.5) encodes the same D65 white at a much brighter 1000
nits", and "scRGB (1.0, 1.0, 1.0) and HDR10 (497, 497, 497)" are the same white. Two swap-chain
options: `DXGI_FORMAT_R16G16B16A16_FLOAT` in linear scRGB ("the same pixel format and color space
used by the DWM", 64 bits per pixel), or `DXGI_FORMAT_R10G10B10A2_UNORM` in
`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020` for an app that "Targets an HDR display" and whose
"Swap chain doesn't require blending with alpha/transparency" ("on certain GPUs this eliminates
some processing needed to convert the content to the HDR10 wire format"). The reference-white
section is the one games get wrong: "Reference white indicates the brightness at which a diffuse
white object (such as a sheet of paper, or sometimes UI) appears in an HDR scene"; "Your HDR app
must allow the user to either set their desired reference white level, or to read the value
configured by the system"; "if your app performs its own composition of SDR and HDR content into a
single surface, then you're responsible for performing the SDR reference white level adjustment
yourself. Otherwise the SDR content might appear too dim under typical desktop viewing conditions";
the factor is `SdrWhiteLevelInNits / 80`. The parameter that matters most for tone mapping is "max
luminance, also known as MaxCLL".
*Bearing:* Forge's overlay is composed into the same surface as the scene, so Forge owns the white
level: read it from the OS at start and on a display change, default the UI's white to it, draw the
scene through the transform and the UI at that white. The 10-bit PQ swapchain is the game's option;
fp16 scRGB is the debugging aid because its values are nits / 80.

**Microsoft. `DXGI_OUTPUT_DESC1` / `IDXGIOutput6::GetDesc1` (dxgi1_6.h, 2018-12-05) and
`DISPLAYCONFIG_SDR_WHITE_LEVEL` (wingdi.h, 2022-08-08).** [docs] [still-current]
<https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_6/ns-dxgi1_6-dxgi_output_desc1> ·
<https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-displayconfig_sdr_white_level>

The three numbers a display reports: `MinLuminance`, `MaxLuminance` ("max luminance in nits,
likely for just a small area of the display") and `MaxFullFrameLuminance` ("unlike MaxLuminance,
this value is valid for a color that fills the entire area of the panel"), which "can adjust
dynamically while the system is running", so apps "should periodically query
IDXGIFactory::IsCurrent and re-query GetDesc1". The white level: "The monitor's current SDR white
level, specified as a multiplier of 80 nits, multiplied by 1000. E.g. a value of 1000 would indicate
that the SDR white level is 80 nits, while a value of 2000 would indicate an SDR white level of 160
nits", read through `QueryDisplayConfig`.
*Bearing:* Vulkan has no query for either, so the Windows path reads them through `windows-sys`
(already in Forge's lock file through winit) behind a `cfg(windows)` function returning
`Option<DisplayCaps>`; elsewhere and on failure the defaults are 1000 nits peak, 0.005 nits black,
80 nits SDR white. The peak seeds the preset choice (the nearest of 500/1000/2000/4000, never above
the panel's `MaxLuminance`), the white seeds the UI level.

**Microsoft. "Calibrate your HDR display using the Windows HDR Calibration app" (Support) and
"The Windows HDR Calibration app is here" (DirectX Developer Blog, 2022).** [docs] [web]
[still-current]
<https://support.microsoft.com/en-us/windows/hardware/display-graphics/calibrate-your-hdr-display-using-the-windows-hdr-calibration-app>
· <https://devblogs.microsoft.com/directx/the-windows-hdr-calibration-app-is-here/>

The OS-level version of the game pattern: three test patterns for "the darkest visible detail,
the brightest visible detail, and your display's maximum brightness", each adjusted by dragging
"the slider until the test pattern is no longer visible", then a saturation adjustment; the result
is "a calibrated HDR color profile" that the OS reports back through the `DXGI_OUTPUT_DESC1`
luminances.
*Bearing:* on a calibrated Windows 11 machine the reported `MaxLuminance` is the user's answer to
the same three questions Forge's calibration overlay would ask, so the overlay's defaults come from
the OS and the overlay exists for the machines that never ran the app.

**Scott Miller, Mahdi Nezamabadi, Scott Daly. "Perceptual Signal Coding for More Efficient Usage
of Bit Codes." *SMPTE Motion Imaging Journal* 122(4), May 2013, 52–59; standardised as SMPTE ST
2084 (the PQ EOTF) and carried into Rec. ITU-R BT.2100.** [paper] [spec] [foundational]
<https://doi.org/10.5594/j18290> (constants as documented by colour-science:
<https://colour.readthedocs.io/en/develop/generated/colour.models.eotf_ST2084.html>)

The curve: "let the human visual system determine the quantization curve used to encode video
signals, in order to maintain optimal efficiency across the luminance range of interest and keep
the visibility of quantization artifacts uniformly small", fitted to Barten's contrast sensitivity
so that 12 bits sit just below the threshold from 0 to 10 000 cd/m². The constants, with
`Y = L / 10000`: `m1 = 2610/4096/4 = 0.1593017578125`, `m2 = 2523/4096·128 = 78.84375`,
`c1 = 3424/4096 = 0.8359375`, `c2 = 2413/4096·32 = 18.8515625`, `c3 = 2392/4096·32 = 18.6875`,
`E = ((c1 + c2·Y^m1) / (1 + c3·Y^m1))^m2`, inverse `Y = (max(E^(1/m2) − c1, 0) / (c2 − c3·E^(1/m2)))^(1/m1)`.
Computed from these (the values Forge's unit test should pin): 80 nits → 0.4859 (code 497 of 1023,
Microsoft's figure), 100 → 0.5081 (520), 203 → 0.5807 (594), 1000 → 0.7518 (769), 4000 → 0.9026
(923), 0.1 → 0.0623 (64), 0.005 → 0.0151 (15). One 10-bit code is 0.98 % of the luminance at 100
nits, 0.90 % at 1000 and 4000, 1.9 % at 1 nit, 3.8 % at 0.1 nit; an 8-bit sRGB code on a 100-nit
display is 1.7 % at code 128 and 5.3 % at code 32.
*Bearing:* PQ in `f32` on the GPU is two `pow`s; the encode goes after the table, on values in
[0, 1] that are `nits / 10000`. Ten bits are one step coarser than PQ's design point, so the
display pass dithers half a code in PQ space (the SDR path has no dither today; grep found none in
`shaders/`), with `spatio_temporal_noise` from `shaders/noise.slang`.

**ITU-R. Recommendation BT.2100 (HDR television: Rec.2020 primaries, PQ and HLG) and
Recommendation BT.2087-0 (10/2015), "Colour conversion from Recommendation ITU-R BT.709 to
Recommendation ITU-R BT.2020".** [spec] [foundational]
<https://www.itu.int/rec/R-REC-BT.2087-0-201510-I>

BT.2087 gives the Rec.709 → Rec.2020 conversion as "Two sets of conversion equations", one through
the OETF and one through the EOTF, with "All matrix values ... calculated with high precision and
then rounded to four decimal digits"; the linear-light matrix is the product of the two RGB↔XYZ
matrices (from memory, its first row is 0.6274, 0.3293, 0.0433; see Verification notes).
*Bearing:* Forge should not type the matrix: `aces2::conversion(&REC709, &REC2020, false)` derives
it from chromaticities, and a `REC2020` `Primaries` constant (0.708/0.292, 0.170/0.797, 0.131/0.046,
D65) is the only addition. The limiting gamut stays P3-D65 (the Academy's preset) inside a Rec.2020
encoding, so the table bakes AP0 → OT(peak, P3-D65) → P3 → Rec.2020 → PQ in one go.

**ITU-R. Report BT.2408-8 (11/2024), "Guidance for operational practices in HDR television
production" (a -9 revision is listed for 2026).** [spec] [still-current]
<https://www.itu.int/dms_pub/itu-r/opb/rep/R-REP-BT.2408-8-2024-PDF-E.pdf>

The reference white of HDR production: "HDR reference white signal level is specified as 58 % PQ
or 75 % HLG respectively", about 203 cd/m², with "Graphics White" defined as "the equivalent in the
graphics domain of a 100 % reflectance white card: the signal level of a flat, white element
without any specular highlights"; a factor of 2.0 maps SDR's 100 cd/m² peak to the 203 cd/m² level.
*Bearing:* 203 nits is the "broadcast" mark on the paper-white slider, next to the OS's SDR white
as the default; ACES 2.0's presets put a scene white lower (§4), which is why the slider exists.

**Timothy Lottes (AMD). "Advanced Techniques and Optimization of HDR/VDR Color Pipelines." GDC
2016, Advanced Graphics Techniques Tutorial Day.** [talk] [still-current]
<https://www.gdcvault.com/play/1023512/Advanced-Graphics-Techniques-Tutorial-Day> (slides:
<https://gpuopen.com/wp-content/uploads/2016/03/GdcVdrLottes.pdf>)

The three-part tutorial games' HDR pipelines were built from: "Variable Dynamic Range,
Tonemapping for VDR, and Transfer Functions and Dithering", covering "optimization and quality at
various stages of the pipeline from eye-adaption, color-grading, and tone-mapping, through film
grain and final quantization", including the PQ encode's cost and the dither before the 10-bit
write.
*Bearing:* the source for the display pass's shape: tone map and grade in a shaper + 3-D table,
encode, dither, write. Forge's pass already has the table; the encode and the dither are the two
additions.

**Bartlomiej Wronski. "Dithering part three – real world 2D quantization dithering." Blog, 30
October 2016.** [web] [still-current]
<https://bartwronski.com/2016/10/30/dithering-part-three-real-world-2d-quantization-dithering/>

"Quantization of 2D images for storing them at limited bit depth", comparing "white noise, ordered
Bayer pattern matrices, interleaved gradient noise and blue noise" as the dither signal; the last
two are the ones that survive a temporal filter.
*Bearing:* one triangular-distributed value per pixel and frame from `spatio_temporal_noise`, of
amplitude one PQ code, added before the 10-bit conversion; the same code would fix any SDR banding
the owner reports.

**Evan Hart (NVIDIA). "HDR Ecosystem for Games" ("Advances in the HDR Ecosystem, Presented by
NVIDIA"). GDC, 23 March 2018.** [talk] [still-current]
<https://gdcvault.com/play/1024803/Advances-in-the-HDR-Ecosystem> ·
<https://developer.nvidia.com/hdr-gdc-2018>

NVIDIA's guidance from the PC side: "the standards and technologies behind HDR from a PC
perspective with a dive into color science, ultimately distilling this down to practical advice
for game developers"; good HDR is "brightness, contrast, and precision that make highlights
brighter, maintain or improve darkness of shadows, preserve detail at both ends of the range, and
allow more vivid mid-tones".
*Bearing:* no NVIDIA SDK is involved in HDR output on Vulkan; the advice (scRGB or HDR10, read the
OS white, calibrate) matches Microsoft's. NVIDIA-specific behaviour shows up only in the field
reports below.

**Field reports on Vulkan HDR swapchains on Windows: NVIDIA developer forum, "Driver switches
display to HDR from Vulkan PQ swapchain but keeps 8-bit display link when windows is in SDR mode"
(24 March 2024); KhronosGroup/Vulkan-Samples issue #638, "Add sample for HDR display formats"
(open since 14 March 2023); kvark/blade issue #158 (14 August 2024).** [web] [recent]
<https://forums.developer.nvidia.com/t/bug-driver-switches-display-to-hdr-from-vulkan-pq-swapchain-but-keeps-8-bit-display-link-when-windows-is-in-sdr-mode/287197>
· <https://github.com/KhronosGroup/Vulkan-Samples/issues/638> · <https://github.com/kvark/blade/issues/158>

Three things the specifications do not say. NVIDIA's Windows driver "still lists HDR modes when
Windows is in SDR mode" and switches the display itself when a PQ swapchain is created. Khronos
has no HDR display sample: the issue asks for one with `A2B10G10R10_UNORM_PACK32` and
`HDR10_ST2084_EXT` since "most modern games support HDR displays", and it is still open. And the
extended-sRGB space is not offered with 8-bit formats: blade's `B8G8R8A8_UNORM` +
`EXTENDED_SRGB_LINEAR_EXT` "isn't reported as supported in HDR mode", and it moved to
`R16G16B16A16_SFLOAT`.
*Bearing:* choose the HDR pair only when the user asked (`--hdr`, or the H key) *and* the OS
reports the display in HDR (`DXGI_OUTPUT_DESC1::ColorSpace` is `RGB_FULL_G2084_NONE_P2020`), not
merely because the driver lists it; otherwise the ballad would flip the owner's monitor into HDR on
every scripted run. Never expect the linear space with an 8-bit format.

---

## 2. Tone mapping for HDR displays: ACES 2.0's presets and the engines' curves

The Academy's transform is the one standard curve designed with HDR presets; every shipped engine
before it built its own display mapper with the same three inputs (peak, black, paper white).

**Academy of Motion Picture Arts and Sciences / Academy Software Foundation. `aces-core` v2.0
(4 April 2025; `Lib.Academy.OutputTransform.ctl`) and its changelog.** [spec] [code] [recent]
<https://github.com/aces-aswf/aces-core>

The reference implementation ("This repository was named aces-dev in versions of ACES prior to the
2.0 release", Apache-2.0). The library file carries the parameters Forge's port copied: AP0 and
AP1, `ref_luminance = 100`, `L_A = 100`, `Y_b = 20`, `surround = {0.9, 0.59, 0.9}; // Dim surround`,
the 360-entry hue tables with two wrap entries. The changelog explains the pieces that move with
peak luminance: developer release 2 (August 2024) set the "upper limit clamp value in AP1 clamping
step to a value equal to 3 stops (8x) the minimum value required to reach maximum output from the
tone scale function"; release 3 (September 2024) changed the "lower hull gamma approximation from a
constant value to a value determined using the log of the peak luminance" because it "minimizes
some clipping artifacts that could occur at edge values at higher luminance outputs", and added an
"extra clamp to PQ and HLG (derived from PQ 1000) EOTF options to protect against rare
reintroduction of negative values"; v2.0 refactored "to parallel the optimizations made by OCIO
(included in their 2.4.2 release)".
*Bearing:* `aces2.rs` is a port of OCIO 2.5.2 and therefore of this v2.0 (its
`lower_hull_gamma_inv = 1 / (1.14 + 0.07·log_peak)` and `forward_limit = 8·r_hit` are these
entries), so the HDR presets need no new maths, only `OutputTransform::new(1000.0, &P3_D65)` and a
PQ encode that clamps negatives first.

**Academy / ASWF. `aces-output`: the Output Transform presets (CTL), `d65/rec2100/`.** [spec]
[code] [recent]
<https://github.com/aces-aswf/aces-output>

"CTL transforms with predefined parameter sets that invoke functions from aces-core to produce
reference output renders for standard and commonly used display configurations", in `d60/` and
`d65/` trees. The Rec.2100 presets are named by limiting gamut, peak and encoding:
`Output.Academy.P3-D65_{500,1000,2000,4000}nit_in_Rec2100-D65_ST2084`, the same four with
`Rec2100-D65` limiting, `P3-D65_1000nit_in_Rec2100-D65_HLG`, and
`Rec709-D65_100nit_in_Rec2100-D65_ST2084` (the SDR look carried in an HDR container). "Users are
not restricted to the included transforms and may define additional output transforms for other
display characteristics as needed."
*Bearing:* Forge's preset table is this list: peak ∈ {500, 1000, 2000, 4000}, limiting ∈ {P3-D65,
Rec.2020}, encoding Rec.2020 PQ, plus the 100-nit Rec.709 row for "SDR in an HDR container", which
is also the fake-HDR debug view's definition (§5). A custom peak (a panel's 780 nits) is legitimate
by the Academy's own words; the presets are the tested points.

**Academy Software Foundation. OpenColorIO 2.4 (September 2024, "ACES 2.0 Output Transforms
(PREVIEW RELEASE)") and 2.5; `src/OpenColorIO/transforms/builtins/ACES.cpp`.** [code] [docs]
[recent]
<https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/main/docs/releases/ocio_2_4.rst> ·
<https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/main/src/OpenColorIO/transforms/builtins/ACES.cpp>

The builtins Forge's port is measured against: `ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 -
HDR-{500,1000,2000,4000}nit-P3-D65_2.0` and `…HDR-{500,1000,2000,4000}nit-REC2020_2.0` (plus
108 and 300-nit cinema and D60-simulation variants), with the display encoding described as "the
ST-2084 transfer function for HDR with an input Y value of 1.0 mapping to 100 nits".
`ACES2_OUTPUT::Generate_output_transform` shows the exact chain: convert to AP1, clamp to
`upperBound = 8 · (128 + 768 · log(peak/100) / log(10000/100))`, back to AP0, the fixed function
with the peak and the limiting primaries, "Post transform clamp" to `peak / 100`, an optional
"White point simulation" scale, then to XYZ D65; `Generate_nit_normalization_ops` notes "The PQ
curve expects nits / 100 as input".
*Bearing:* the HDR chain in `aces2.rs` is `Sdr` with two changes: the clamp to `forward_limit`
(4096 at 1000 nits, already computed) and the output clamp to `peak / 100` instead of 1; then
`× 100 / 10000` and PQ. OCIO's own test file already gave the port 35 colours at 1000 nits in P3
within 1e-5 (`the_transform_matches_ocio_at_1000_nits_in_p3`), so that preset is verified today;
the others need reference values generated with OCIO on the owner's machine (§5).

**Academy Software Foundation. OpenColorIO-Config-ACES 3.0.0 "for ACES 2.0" (OCIO 2.4, March
2025) and 4.0.0 (OCIO 2.5).** [code] [recent]
<https://github.com/AcademySoftwareFoundation/OpenColorIO-Config-ACES/releases/tag/v3.0.0>

The studio config "implement[s] support for ACES 2.0 by leveraging the dedicated OpenColorIO
builtin transforms" and ships eight displays: "sRGB, Display P3, Display P3 HDR, P3-D65, Rec.1886
Rec.709, Rec.2100-HLG, Rec.2100-PQ, and ST2084-P3-D65"; the 4.0.0 config ships inside OCIO 2.5.
*Bearing:* the config names the display Forge targets, `Rec.2100-PQ`, and is the tool that renders
the reference values on the owner's machine (`pip install opencolorio`), no CTL toolchain needed.

**ACESCentral community, "ACES 2 in Game Engines: Variable reference/max luminance" (thread
5734, 2025; opened by allenwp for Godot's HDR output).** [web] [recent]
<https://community.acescentral.com/t/aces-2-in-game-engines-variable-reference-max-luminance/5734>

The one place the Academy's people discuss the game case: a system "with HDR output that treat[s]
SDR content with a variable maximum nit value", "whether the difference between middle grey on SDR
and HDR should match exactly, and how maximum luminance should be adjusted when scaling between
different reference white levels"; and the note that "The 200 nits reference white in HDR comes
from broadcast reality to acknowledge that all TVs in SDR are showing peak white at least 200 nits,
not 100 nits as defined in ITU BT 2035".
*Bearing:* the transform has no paper-white parameter; its grey rises slowly with the peak (§4:
10.0 nits at 100, 14.5 at 1000, 16.8 at 4000) and a scene white lands at 107 nits at 1000. A game
that wants the user's 200-nit white scales the scene in front of the transform (an exposure offset,
which moves grey with it) or the output (which eats the roll-off's headroom). The thread does not
settle it; Forge's recommendation is the exposure offset, shown as "paper white" in nits and
defaulting to the Academy's look.

**Alex Fry (EA DICE / Frostbite). "High Dynamic Range Color Grading and Display in Frostbite."
GDC 2017.** [talk] [still-current]
<https://gdcvault.com/play/1024466/High-Dynamic-Range-Color-Grading> ·
<https://www.ea.com/frostbite/news/high-dynamic-range-color-grading-and-display-in-frostbite>

The engine talk the others measure against: "the display mapping used to implement the 'grade
once, output many' approach to targeting any display and why an ad-hoc approach as opposed to
filmic tone mapping was chosen"; Frostbite "retained 3D LUT-based grading flexibility and the
accuracy differences of computing these in decorrelated color spaces"; "optimizations to achieve
performance parity with the legacy path and why supporting HDR can also improve the SDR version".
*Bearing:* the pattern Forge follows with a standard curve instead of an ad-hoc one: render once in
scene-referred light and let a display mapper parameterised by the display produce every output;
the SDR output is the 100-nit row of the same table, not a separate pipeline.

**Krzysztof Narkowicz. "HDR Display – First Steps." Blog, 31 August 2016.** [web]
[still-current]
<https://knarkowicz.wordpress.com/2016/08/31/hdr-display-first-steps/>

The first-contact report ("NVIDIA sent us an HDR TV", a game "shipping in less than 2 months"):
output as scRGB, and ACES's RRT + 1000-nit ODT fitted "to a simple analytical curve" so that the
existing pipeline gained an HDR mode in days.
*Bearing:* the minimal viable path, and why scRGB is the right bring-up target: the swapchain's
numbers are nits / 80, so a mistake is visible in a debugger. Forge has the full transform rather
than a fit, but should start in the same mode.

**Jasmin Patry (Sucker Punch). "HDR Display Support in Infamous Second Son and Infamous First
Light", parts 1 (21 December 2016) and 2 (4 January 2017).** [web] [still-current]
<https://www.glowybits.com/blog/2016/12/21/ifl_iss_hdr_1/> ·
<https://www.glowybits.com/blog/2017/01/04/ifl_iss_hdr_2/>

Part 1 "discussed HDR tonemapping and color grading solutions, and the HDR-friendly render target
format used to help improve performance", part 2 "how they matched the look of the SDR and HDR
modes, additional performance optimizations, and issues encountered when combining HDR and 4K on
the PS4 Pro".
*Bearing:* part 2 is Forge's acceptance test: the HDR and SDR images must match below paper white
(same grey, mid-tones and hue), with HDR adding headroom only above it; the ꟻLIP comparison of the
fake-HDR view against the SDR frame (§5) is that test in numbers.

**Jasmin Patry (Sucker Punch). "Real-Time Samurai Cinema: Lighting, Atmosphere, and Tonemapping in
Ghost of Tsushima." SIGGRAPH 2021, *Advances in Real-Time Rendering in Games*.** [talk] [recent]
<https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html> (slides:
<https://www.glowybits.com/talks/real-time_samurai_cinema/real-time_samurai_cinema.pdf>)

Covers "the tone mapping techniques used to recreate the samurai cinema experience in-game" after
the atmosphere and indirect lighting: a shipped studio-owned display mapper with HDR and SDR
outputs from one grade.
*Bearing:* a display mapper is a look decision as much as a standard, so Forge's other curves stay
selectable in HDR; but AgX, the ACES fit and PBR Neutral have no HDR definition and fall back to
the SDR-in-HDR row. HDR ships with ACES 2.0 only.

**Paul Malin (Activision). "HDR Display in Call of Duty." Digital Dragons, Kraków, 2018.** [talk]
[still-current]
<https://research.activision.com/publications/archives/hdr-in-call-of-duty> (video and notes:
<https://blog.mousefingers.com/post/publications/hdrincod/>)

"A basic introduction to color science, color terminology, and display standards", "the color
pipeline for previous Call of Duty titles", and "challenges with adding HDR display support,
implementation options, and the approach taken to add HDR display support to Call of Duty: WWII".
*Bearing:* the AAA reference the issue's brief meant by "HDR in Call of Duty"; its structure
(standards → existing pipeline → options → the one taken) is this file's.

**Colin Penty (The Coalition). "The Visual Technology of Gears 5." Unreal Dev Days, November
2019; with Epic's interview "The Coalition dives deep into the tech of Gears 5".** [talk] [web]
[recent]
<https://cdn.gearsofwar.com/thecoalition/publications/The%20Visual%20Technology%20of%20Gears%205%20V2%20PDF%20Version.pdf>
· <https://www.unrealengine.com/en-US/developer-interviews/the-coalition-dives-deep-into-the-tech-of-gears-5>

The presentation has an HDR section; the interview reports that the team "used a machine learning
algorithm that was developed by Redmond's ATG group to train an inverse tone-mapper for
color-space conversion, and blended this 50 percent with a Reinhard buffer to allow maximum
control".
*Bearing:* an inverse tone mapper is what a game needs when its grade lives in display-referred
SDR; Forge grades nothing in SDR (the frame is scene-referred until the display pass), so this
route is not needed, which is the pay-off of D-022.

**Epic Games. "High Dynamic Range Display Output in Unreal Engine" (UE 5.x documentation) and
the `r.HDR.*` console variables.** [docs] [still-current]
<https://dev.epicgames.com/documentation/en-us/unreal-engine/high-dynamic-range-display-output-in-unreal-engine>
· <https://unrealdirective.com/resources/console-variables/r-hdr-display-outputdevice/>

"Currently, only paths for 1000-nit and 2000-nit display outputs are implemented in Unreal
Engine." `r.HDR.Display.OutputDevice`: "3: ACES 1000 nit ST-2084 (Dolby PQ) (HDR); 4: ACES 2000 nit
ST-2084 (Dolby PQ) (HDR); 5: ACES 1000 nit ScRGB (HDR); 6: ACES 2000 nit ScRGB (HDR)" beside the
SDR devices; `r.HDR.Display.MaxLuminance` and `MidLuminance` ("for 18% gray"); `r.HDR.UI.Level`,
"Luminance level for UI elements when compositing into HDR framebuffer (default: 1.0)";
`r.HDR.UI.CompositeMode`, "0: Standard compositing, 1: Shader pass to improve HDR blending", with
the advice that "it's recommended to boost the UI slightly to avoid looking washed out next to the
vibrancy of the main scene".
*Bearing:* Unreal's menu is Forge's: a device (SDR, PQ at a peak, scRGB at a peak), a grey or
paper-white level, a UI level, and a choice between blending the UI in the encoded target or in a
separate linear pass. Forge starts with the first (the overlay draws into the swapchain today) and
keeps the compose pass as the fix if text edges look wrong in PQ.

**Unity Technologies. "High Dynamic Range (HDR) Output", HDRP manual (13.1–17.x); and the
HDR Calibration Sample (GitHub, Unity 2022.3+).** [docs] [code] [still-current]
<https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@17.5/manual/HDR-Output.html>
(source: <https://github.com/Unity-Technologies/Graphics/blob/master/Packages/com.unity.render-pipelines.high-definition/Documentation~/HDR-Output.md>)
· <https://github.com/Unity-Technologies/HDR-Calibration-Sample>

The tone mapping "must take into account the capabilities of the target display, specifically
these three values (in nits): Minimum supported brightness. Maximum supported brightness. Paper
White value", where paper white "represents the brightness of a paper-white surface represented on
the display, which determines the display's brightness overall"; LDR and HDR content "do not appear
equally bright on displays with the same Paper White value ... For this reason, it is best practice
to implement a calibration menu for your application." UI: "in HDR mode, HDRP uses Paper White
values to determine the brightness of Unlit materials", so white UI elements' "brightness matches
Paper White values". Three debug views: **Gamut View** (Rec.709, P3-D65 and Rec.2020 triangles),
**Gamut Clip** (green inside Rec.709, blue inside P3-D65, red outside) and **Values exceeding Paper
White** ("Yellow corresponds to Paper White +1, red corresponds to Max Nits, and blue corresponds to
Max Nits+1"). Supported on "Windows with DirectX 11, DirectX 12 or Vulkan". The calibration
sample's pages: "The background has maximum brightness. The Unity logo brightness value determines
the tonemapping max nits"; "The background is completely dark (0 brightness). The Unity logo
brightness value determines the tonemapping min nits"; "Half of the background is in max brightness
and half in 0 brightness. This page allows users to adjust a suitable paper white value for shader
value 1 elements like UI"; defaults from the platform's `paperWhiteNits`, `minToneMapLuminance` and
`maxToneMapLuminance`; "you should use the brightest and darkest environments in your project".
*Bearing:* the calibration overlay and the debug views to copy, almost literally: three pages, one
value each, OS defaults; the "exceeds paper white" false colour is the fake-HDR view that works on
an SDR monitor.

**HDR Gaming Interest Group. "For a Better HDR Gaming Experience" (HGiG guidelines).** [docs]
[still-current]
<https://www.hgig.org/doc/ForBetterHDRGaming.pdf>

The console-side agreement between game platforms and TV makers: a system-level calibration
captures MinTML ("the darkest level at which it still shows black detail"), MaxTML ("the brightest
level at which it still shows white detail") and MaxFFTML (the same "across the full screen"); the
game tone-maps to those and an HGiG-mode TV does not tone-map again; displays may report categories
(MinTML 0.1 nit, MaxFFTML 600, MaxTML 600 / 1000 / 4000).
*Bearing:* the same three numbers as Microsoft's `DXGI_OUTPUT_DESC1` and Unity's triple, which is
why Forge's `DisplayCaps` record has exactly those fields and the calibration overlay asks exactly
those questions; the consoles' "adjust until the logo is barely visible" screens are HGiG's.

---

## 3. What professional engines do

| Engine / game | Output | Curve | Paper white | UI | Calibration | Source |
|---|---|---|---|---|---|---|
| Frostbite (2017) | HDR10 PQ, scRGB, Dolby Vision | own display mapper, "grade once, output many" | a parameter of the mapper | composited at paper white | in-game | Fry 2017 |
| Unreal Engine 5 | PQ or scRGB at 1000 / 2000 nits | ACES 1.x curves per peak | `MidLuminance` / `MaxLuminance` | `r.HDR.UI.Level`, optional compose pass | game's own | Epic docs |
| Unity HDRP / URP | PQ Rec.2020 (DX11/12, Vulkan, Metal, consoles) | Neutral or ACES, presets for 1000 / 2000 / 4000 nits | min / max / paper white | unlit at paper white | sample with three pages | Unity docs, sample |
| Call of Duty: WWII | console HDR10 | studio pipeline | in menu | at paper white | in menu | Malin 2018 |
| Infamous, Ghost of Tsushima | PS4 HDR10 | studio mapper matching the SDR look | in menu | at paper white | in menu | Patry 2016, 2021 |
| Gears 5 | Xbox HDR10 | UE4's ACES + learned inverse tone mapper | in menu | at paper white | Xbox app + menu | Penty 2019 |
| Godot (4.x, in progress) | PQ / scRGB | ACES 2 discussed | debated | — | — | ACESCentral thread |

What they share: the scene stays scene-referred to the last pass; one display mapper takes the
display's peak, black and the user's paper white; the UI is drawn at paper white and never through
the curve; a calibration screen with the three patterns exists because neither the OS nor the panel
can be trusted; and SDR is a row of the same mapper rather than a second pipeline. Where they
differ is the curve. None of the checked engines ships ACES 2.0's HDR presets yet (`lighting-gi.md`,
"Research for issue #76": Unreal, Unity, Filament and Godot were on ACES 1 or AgX in 2025), so
Forge would be early, with the Academy's presets and OCIO's builtins as the reference rather than
another engine.

---

## 4. The interplay with what Forge has

**Pre-exposure and physical units (D-022).** Every pass writes luminance × exposure, with
`exposure = 1 / (1.2 · 2^EV100)` (`crates/forge-render/src/exposure.rs`), so a pre-exposed 1.0 is
the luminance that saturates the metered camera (39 300 cd/m² at the ballad's EV100 15). The
display transform never sees nits; it sees this normalised frame and decides what 1.0 becomes on
the display. Under ACES 2.0's SDR preset that is 45.8 nits of 100 (0.18 → 10.0). The same frame
through the HDR presets, from the port's own tonescale constants (`ToneScale::new` in `aces2.rs`,
evaluated for this note):

| Peak (nits) | `r_hit` | AP1 clamp `8·r_hit` | scene 0.18 | scene 1.0 | 2.0 | 4.0 | 16 | 64 | 256 |
|---|---|---|---|---|---|---|---|---|---|
| 100 | 128 | 1024 | 10.0 | 45.8 | 64 | 79 | 94 | 99 | 100 |
| 500 | 396 | 3171 | 13.2 | 89 | 159 | 247 | 404 | 476 | 497 |
| 1000 | 512 | 4096 | 14.5 | 107 | 206 | 355 | 705 | 915 | 987 |
| 2000 | 628 | 5021 | 15.7 | 122 | 249 | 468 | 1149 | 1716 | 1947 |
| 4000 | 743 | 5946 | 16.8 | 134 | 284 | 570 | 1716 | 3084 | 3800 |

Exposure stays the artistic anchor exactly as in SDR (the histogram meters the pre-exposed frame,
unchanged), HDR adds four to five stops of headroom above white before the roll-off, and the
metered white lands at 107 nits at 1000 nits: below Windows' typical SDR white (80 nits × the
user's slider, commonly 2–3×) and half of BT.2408's 203 nits, so a Forge scene next to the desktop
will look dim unless a paper-white gain precedes the transform. The gain is an exposure offset in
stops, `log2(paper_white / 107)` at 1000 nits (0.93 stops for 203 nits), applied in the display
pass only (never to the metered or TAA'd frame); it moves grey with it, which is the ACESCentral
thread's open point; default 0, so the Academy's look is the default and the calibration page moves
it.

**The baked table (#76) and the HDR presets.** The SDR table is 65³ over a log2 shaper
`log2(x + 1/1024)` on [0, 1024] (20 stops, 0.31 stop per cell), sRGB-encoded RGBA16F texels in a
65² × 65 image, two fetches (`aces2::bake`, `shaders/aces2.slang`). For HDR: (1) the shaper's top
rises to the preset's AP1 clamp (`OutputTransform::forward_limit()`: 4096 at 1000 nits, 5946 at
4000), 22–22.5 stops, 0.34–0.35 stop per cell at 65³, so the density stays within 12 % and 65³
stays; 129³ (0.17 stop per cell, 17 MB per preset) is the fallback if the saturated-blue tail
measured in #76 (p99.9 5.6 codes, max 13.8) grows with the wider range. (2) The table's output is
stored PQ-encoded rather than sRGB-encoded (PQ is the perceptual axis the interpolation should be
uniform in) and in `R16G16B16A16_UNORM` rather than `SFLOAT`: PQ values live in [0, 1], where fp16's
step near 0.75 (the 1000-nit code) is 4.9e-4, half a 10-bit code, against 1.5e-5 for 16-bit UNORM.
For the scRGB output the table holds linear nits / 80 (12.5 at 1000 nits) and stays fp16. (3) The
bake is one function of (peak, limiting, encoding), 10 ms per preset on the CPU today: bake the
chosen preset at start and the others on first use. (4) The whole chain goes into the table:
Rec.709 → AP1 clamp → AP0 → OT(peak, P3-D65) → P3 → Rec.2020 → clamp [0, peak/100] → × 1/100 → PQ,
so the shader adds nothing but the dither.

**Where it sits in the frame.** Nothing upstream changes: bloom and TAA work on the pre-exposed
HDR frame (bloom is blended into the shown image inside the resolve; the history stays unbloomed),
and DLSS takes "the jittered, pre-exposed HDR colour" and returns "the upscaled HDR colour at the
output size" (`crates/forge-gpu/src/dlss.rs`), after which `post/display transform` runs.
Streamline's flag is one Forge already implies: `dlssOptions.colorBuffersHDR = sl::Boolean::eTrue;
// assuming HDR pipeline`, with `preExposure` and `exposureScale` beside it (`sl_dlss.h`:
"Specifies if tagged color buffers are full HDR or not"). The two passes that write the display
image, `temporal/TAA resolve` (history and display in one pass, `Taa::resolve`) and `post/display
transform` (`Display::draw`), take `output_format` at construction and an `encode_srgb` flag from
`format_encodes_srgb`; that flag becomes an `OutputEncoding` (sRGB on write, sRGB in shader, PQ
Rec.2020 10-bit, scRGB linear fp16) plus the paper-white gain and the preset's table, and both
pipelines are compiled for the swapchain's format, as now. On the Streamline build the interposer
wraps the swapchain functions (D-024), so `vkSetHdrMetadataEXT` and the colour-space query must be
exercised through it too.

**The F1 overlay and UI.** `app/overlay` loads the swapchain image and alpha-blends glyph cells
from a palette of SDR colours (`shaders/overlay.slang`, `crates/forge-app/src/overlay.rs`). In HDR
it writes `encode(paper_white_nits × srgb_decode(colour))`, the palette's white at the reference
white, never at the peak. Two caveats: a UNORM PQ target blends in PQ space, not linear as the
`_SRGB` swapchain does today, so anti-aliased glyph edges shift slightly (Epic's `CompositeMode 1`
exists for this); fp16 scRGB blends linearly again. Start with the in-place write; add a linear
compose pass only if the edges show. The overlay gains a status line: format, colour space, preset,
OS white level, the display's reported peak.

**Captures and ꟻLIP.** `save_capture` (`crates/forge-app/src/lib.rs`) copies the swapchain image
to a buffer and writes RGBA8 PNG; `imgdiff` compares 8-bit PNGs and prints LDR-ꟻLIP. An HDR
swapchain breaks both. Two layers fix it: exact diffs on **PQ codes**, by unpacking A2B10G10R10 to
a 16-bit PNG of the 10-bit codes (lossless, no new crate: `image` 0.25 with `png` writes `Rgb16`;
fp16 captures are converted to codes on the CPU first); and the perceptual check on the **fake-HDR
SDR view** (§5) with today's LDR-ꟻLIP, which is what the owner sees on an SDR monitor anyway.
HDR-ꟻLIP is the third layer when a number on the HDR image itself is wanted: NVIDIA's reference
(`FLIP.h`, BSD-3, already credited) tone-maps the linear image with ACES by default over a range of
exposures, `startExposure = log2(xMax / Ymax)` and `stopExposure = log2(xMax / Ymedian)` from the
reference, and pools the LDR error maps; it would read the PQ-decoded nits. The A/B harness
(`tools/compare.sh`) keeps its 0-pixel rule on the code PNGs, since culling does not touch the
display pass.

**Cross-vendor.** The standard pair is what Cauldron shows AMD's driver listing; no SDK on either
vendor; the 10-bit format's channel order may differ and is irrelevant to a shader writing
`float4`; the subgroup and groupshared rules are untouched, the display pass being a fullscreen
triangle.

---

## 5. Verifying without an HDR monitor

The whole path except the present is testable on any display, which is the property to build in
from the first commit.

1. **Offscreen HDR** (`--hdr offscreen`): the display pass writes an `A2B10G10R10_UNORM_PACK32`
   (or fp16) transient instead of the swapchain, and a `post/hdr preview` pass maps it to the SDR
   swapchain. Every capture, golden image and ꟻLIP number of the HDR path comes from this mode on
   the SDR machine; the real swapchain adds only the format and colour space.
2. **The preview ("fake HDR")** is the Academy's SDR-in-HDR row read backwards: decode PQ → nits,
   divide by paper white, clamp, sRGB-encode (what a 100-nit monitor would show of the HDR frame
   below white), with Unity's false colour as a second mode (yellow above paper white, red at the
   peak, blue above it) and the gamut clip (green / blue / red for Rec.709 / P3 / outside) as a
   third, which is where ACES 2.0's saturated-blue tail will show.
3. **Unit tests**: PQ encode/decode round trip (`|Y − decode(encode(Y))| / Y < 1e-6` over 0.0001–
   10 000 nits) and the pinned codes (80 → 497, 100 → 520, 203 → 594, 1000 → 769, 4000 → 923 of
   1023); the Rec.709 → Rec.2020 matrix derived from primaries against BT.2087's rounded values;
   the tonescale table above (0.18 → 14.5 nits, 1.0 → 106.6 at 1000 nits) as a regression on the
   presets; the 1000-nit P3 OCIO values already in `aces2.rs`.
4. **Reference values for every preset**, generated once on the owner's machine with OpenColorIO
   2.5's Python bindings (the `HDR-{500,1000,2000,4000}nit-{P3-D65,REC2020}_2.0` builtins) over
   the 4096 colours of `tonecheck`, checked in as a small data file and compared in a test at 1e-4
   (OCIO's own GPU tolerance); `meshlets --tone-check` then runs both GPU paths per preset as it
   does for SDR, so `tools/validate.sh` covers HDR without a display. The Academy's test images
   (`aces-output/tests/images`) are the second oracle; they are a download, so ask the owner first.
5. **Table against transform per preset**, as #76 did: p50 / p99 / p99.9 / max in 10-bit PQ codes
   on 400 000 colours and on the six real views, with ꟻLIP on the preview; the pass criterion is
   #76's (within 1–2 codes on real frames) restated in PQ codes.
6. **Banding**: a synthetic 0 → 1000-nit ramp captured at 10 bits with and without dither; count
   the distinct codes and the run lengths; the eye test is the ballad's sky.
7. **On the owner's monitor, if it has HDR** (the overlay line says so): the OS values against the
   calibration overlay's answers; a 10 % white patch and a full white at the chosen peak, by eye or
   with a meter if one exists; the black floor (the ballad's space); the overlay's legibility at
   the OS white; and the desktop next to the game window as the sanity check for the paper-white
   default. Without an HDR monitor the report says so, and steps 1–6 are the verification.

---

## Recommendation for Forge

**Scope.** #94 as filed: the ballad on an HDR display, switchable at run time, ACES 2.0's HDR
presets on an HDR10 swapchain, the cost in the overlay, D-022 and the README updated. Everything
below is buildable on the SDR machine through the offscreen mode, and the same on AMD.

1. **Detect and choose** (`forge-gpu`): enable `VK_EXT_swapchain_colorspace` on the instance
   (skipped silently if absent) and `VK_EXT_hdr_metadata` on the device when offered; extend
   `Swapchain::new/recreate` with a requested `SurfaceMode { Sdr, Hdr10Pq, ScRgbLinear }`, matched
   against `get_physical_device_surface_formats` (`A2B10G10R10_UNORM_PACK32` or
   `A2R10G10B10_UNORM_PACK32` + `HDR10_ST2084_EXT`; `R16G16B16A16_SFLOAT` +
   `EXTENDED_SRGB_LINEAR_EXT`), falling back to today's sRGB pair with a log line. Enter an HDR mode
   only when asked (`--hdr pq|scrgb|offscreen|off`, the H key) and, on Windows, when the OS reports
   the output in HDR, so scripted runs never switch the owner's monitor. Add `DisplayCaps { peak,
   full_frame_peak, black, sdr_white }` read on Windows through `windows-sys` (`QueryDisplayConfig`
   for the white level, `IDXGIOutput6::GetDesc1` for the luminances), `None` elsewhere. Half a day.
2. **Metadata**: after creating an HDR swapchain, `set_hdr_metadata` with Rec.2020 primaries, D65,
   the preset's peak as `maxLuminance` and MaxCLL, the panel's black or 0.005, MaxFALL 0 until the
   histogram supplies one; through the Streamline interposer as well as the plain loader. An hour.
3. **The presets as data** (`forge-render::aces2`): `Preset { peak, limiting, encoding }` with the
   Academy's rows (500/1000/2000/4000 × P3-D65 | Rec.2020, PQ Rec.2020) and the SDR row; a
   `REC2020` constant; the output clamp to `peak/100`; the bake parameterised by preset with the
   shaper's top at `forward_limit()`, PQ output in `R16G16B16A16_UNORM` (fp16 nits/80 for scRGB);
   `ToneTables` holding one table per baked preset, the chosen one at start and the rest lazily
   (10 ms each). One day, including the tests of §5 items 3–4.
4. **The display passes**: `OutputEncoding` in place of `encode_srgb` in `Display`, `Taa` and the
   DLSS path's display pass; the paper-white gain (stops) and the half-code dither from
   `spatio_temporal_noise` in `display_output`; the preset selectable at run time next to the curve
   (G cycles curves, `--hdr-preset` or a second key cycles peaks); curves other than ACES 2.0 in an
   HDR mode go through the 100-nit SDR-in-HDR row at paper white, so nothing breaks when the owner
   presses G. Half a day.
5. **The overlay at the reference white**: palette colours written as `encode(paper_white ×
   linear)`; the status line; blending left in place, the compose pass noted as the follow-up if
   edges show. Two hours.
6. **Calibration overlay** (`forge-app`, drawn by the overlay system): three full-screen pages in
   Unity's and HGiG's form (peak: logo on a full-white field, dim until it disappears; black: logo on
   black; paper white: half/half field, set the UI level), defaults from `DisplayCaps`, values saved
   with the demo's settings and applied as the preset choice (nearest not above the peak), the black
   floor and the paper-white gain. Half a day.
7. **Offscreen mode, preview and captures**: the `post/hdr preview` pass with its three modes;
   `save_capture` writing 16-bit PNGs of PQ codes for HDR targets; `tools/captures.sh` gaining two
   HDR captures of the ballad (frames 240 and 600, offscreen), `compare.sh` comparing them as
   0-pixel lines and the preview with ꟻLIP. Half a day. HDR-ꟻLIP in `imgdiff` is its own issue.
8. **Measure and document**: the numbers below; D-022 gains the HDR paragraph (presets, paper white,
   encoding, the table format), `docs/demos/asteroids.md` an "HDR output" section with the preview
   captures, `PROFILE.md` the pass cost, the README the flags; CREDITS unchanged (OCIO and the ACES
   project are credited; `windows-sys` is already in the lock file, check `cargo run -p credits`).

**Render-graph passes added or changed.** Changed: `temporal/TAA resolve` and `post/display
transform` (the output image's format and the encoding; declarations unchanged), `app/overlay`
(shader constants only), `app/capture` (the copy's source format). Added: `post/hdr preview` (reads
the offscreen HDR transient, writes the swapchain; only in offscreen or preview modes) and, later,
`post/hdr metadata histogram` (a 256-bin PQ histogram of the shown image for MaxCLL/MaxFALL, the
`LuminanceMeter` pattern with a `HostRead` access). No barriers beyond the ones the graph derives
from the new image; no async-queue passes.

**Expected cost (an order of magnitude).** The display pass gains two `pow`s for PQ and a noise
fetch: the ACES 2.0 table costs +0.007 ms at 1440p in the resolve today (D-022) and the bench's
display pass 0.011 ms at 1600 × 900, so the HDR encode is a few thousandths of a millisecond,
below what the overlay resolves; the preview pass is another fullscreen triangle of the same
order. The 10-bit swapchain has the bandwidth of the 8-bit one; fp16 scRGB doubles the present
image (29 MB at 1440p, ~0.03 ms of bandwidth) and the compositor's conversion is outside Forge's
timers. The bake is 10 ms per preset of CPU at start, 40 ms if all four are baked eagerly; tables
are 2.2 MB each at 65³ RGBA16 (17 MB at 129³). The report's numbers should read "within 0.01 ms"
rather than a saving.

**What to measure and report.** The resolve and display pass in ms per encoding (sRGB, PQ, scRGB)
at 1600 × 900 and 1440p, three runs; the bake time per preset; the table against the per-pixel
transform per preset in PQ codes (p50/p99/p99.9/max on 400 000 colours and the six views) and ꟻLIP
on the preview; the SDR frame against the preview of the 1000-nit frame below paper white (ꟻLIP
means near #76's 0.003–0.012, the Infamous criterion); the capture batch at 0 px on every unchanged
line; validation and synchronization validation silent in every mode and across the H switch; the
banding ramp's distinct codes with and without dither; and, if the monitor has HDR, §5.7.

---

## What the numbers say

Encodings: scRGB 1.0 = 80 nits, 12.5 = 1000, 50 = 4000 (Microsoft); PQ 10-bit codes 497 / 520 /
594 / 769 / 923 of 1023 for 80 / 100 / 203 / 1000 / 4000 nits; one 10-bit PQ code is 0.9–1.0 % of
the luminance from 100 to 4000 nits, 1.9 % at 1 nit, 3.8 % at 0.1 nit, against 1.7 % for 8-bit sRGB
at code 128 on a 100-nit display (PQ was designed for 12 bits, Miller et al. 2013). Reference
whites: Windows' nominal SDR white 80 nits, adjustable by the user (`SDRWhiteLevel / 1000 × 80`);
BT.2408's 203 nits (58 % PQ); ACES 2.0's scene white 45.8 nits of 100 in SDR and 107 / 122 / 134
nits at 1000 / 2000 / 4000 nits, grey 10.0 / 14.5 / 15.7 / 16.8 (computed from the port's
constants). Presets: the Academy's Rec.2100 rows are 500 / 1000 / 2000 / 4000 nits with P3-D65 or
Rec.2020 limiting (plus 1000-nit HLG and 100-nit Rec.709-in-PQ); OCIO 2.4 (September 2024) lists
the same as builtins; Unreal ships 1000 and 2000 nit ACES 1.x paths; Unity presets 1000 / 2000 /
4000; HGiG's categories 600 / 1000 / 4000 MaxTML at 600 full-frame and 0.1 nit MinTML. The
transform's clamps: AP1 upper bound `8·r_hit` = 1024 / 3171 / 4096 / 5021 / 5946 at 100 / 500 /
1000 / 2000 / 4000 nits (OCIO's formula and `aces2.rs`), output clamp `peak/100`. Tables: 65³ over
20 stops today (0.31 stop per cell), 22–22.5 stops for HDR (0.34–0.35); fp16's step at PQ 0.75 is
4.9e-4 (half a code) against 1.5e-5 for 16-bit UNORM; 2.2 MB per 65³ RGBA16 preset. Costs from
Forge's own measurements: the ACES 2.0 table +0.007 ms at 1440p in the resolve, the per-pixel
transform +0.10 ms, the bake 10 ms; the display pass 0.011 ms at 1600 × 900; DLSS 0.43–0.46 ms
unchanged by the output format (D-022, D-024, `PROFILE.md`). Cauldron's HDR10 metadata: 1000 nits
max, MaxCLL 1000, MaxFALL 400, black 0.

---

## Checked and left out

- **A God of War HDR talk** — none found. Santa Monica's GDC 2019 set (nineteen talks) and
  "Rendering 'God of War Ragnarök'" (GDC 2023) are listed, none on HDR output. Covered by the other
  studios' talks.
- **"Gears 5" as a stand-alone HDR talk** — only the HDR section of Penty's Unreal Dev Days 2019
  presentation and Epic's interview; the PDF host was not reachable, so the entry quotes the
  interview.
- **The BT.2087 matrix coefficients** — the document and its "rounded to four decimal digits"
  statement were confirmed by search; the coefficients are from memory, with the recommendation to
  derive the matrix from primaries in code.
- **Dolby Vision, HDR10+ and HLG** — dynamic metadata is out of scope (Vulkan's `DOLBYVISION_EXT`
  is documented as legacy in the spec's own note); Windows presents PQ, so the Academy's HLG preset
  is not built.
- **AgX, the ACES 1.x fit and PBR Neutral in HDR** — none has an HDR output definition in the form
  Forge uses; all three fall back to the SDR-in-HDR row. An HDR AgX is a separate idea.
- **HDR-ꟻLIP in `imgdiff`** — described (§4) and deferred to its own issue; the LDR port exists.
- **Narkowicz's 1000-nit curve coefficients and Frostbite's slide content** (paper-white values,
  the LUT layout in PQ) — neither page was fetchable; only the search records' sentences are quoted,
  and the table in §3 marks Frostbite's paper white as "a parameter of the mapper" for that reason.
- **Two further NVIDIA forum threads** ("Windowed Vulkan HDR is presented through the SDR path when
  pEngineName is exactly 'DXVK'", "Problems with HDR and Nvidia Driver (on 3070)") — seen in
  search results, not load-bearing.
- **Auto HDR, `VK_EXT_surface_maintenance1`** — irrelevant to a game presenting HDR itself; not
  researched.

---

## Verification notes

Checked on 2026-09-26 with WebSearch and WebFetch (and `curl` to GitHub) only; no browser pane.
The session's egress proxy served `github.com` and `raw.githubusercontent.com` and refused every
other host tried (registry.khronos.org, docs.vulkan.org, learn.microsoft.com, gpuopen.com,
knarkowicz.wordpress.com, ea.com, draftdocs.acescentral.com, community.acescentral.com, hgig.org,
glowybits.com, blog.mousefingers.com: "blocked by the network egress proxy"). Verification
therefore has two grades, as in `terrain-genesis.md`.

- **Fetched and read (GitHub):** Vulkan-Docs `main` (`chapters/VK_KHR_surface/wsi.adoc` for the
  colour-space definitions, the primaries table and `VkHdrMetadataEXT`'s members; the
  `VK_EXT_swapchain_colorspace`, `VK_EXT_hdr_metadata` and `VK_AMD_display_native_hdr` appendices);
  MicrosoftDocs/win32 `high-dynamic-range.md` (ms.date 2022-10-10) and MicrosoftDocs/sdk-api
  `ns-dxgi1_6-dxgi_output_desc1.md` (2018-12-05) and `ns-wingdi-displayconfig_sdr_white_level.md`
  (2022-08-08), the sources of learn.microsoft.com; Unity-Technologies/Graphics `HDR-Output.md` and
  the HDR-Calibration-Sample README; GPUOpen Cauldron `FreeSyncHDR.cpp` and `ExtFreeSyncHDR.cpp`
  and the FreesyncPremiumProSample readme; aces-core (`README.md`, `CHANGELOG.md`,
  `lib/Lib.Academy.OutputTransform.ctl`) and aces-output (README, `d65/rec2100` listing);
  OpenColorIO `docs/releases/ocio_2_4.rst` and `src/OpenColorIO/transforms/builtins/ACES.cpp`; the
  OpenColorIO-Config-ACES v3.0.0 release page; NVIDIAGameWorks/Streamline
  `docs/ProgrammingGuideDLSS.md` (2.14.1) and `include/sl_dlss.h`; NVlabs/flip `README.md` (v1.7)
  and `src/cpp/FLIP.h`; Vulkan-Samples issue #638 and kvark/blade issue #158. Quotes from these are
  verbatim.
- **Confirmed through the search engine's record of the primary page** (title, authors, venue,
  date, and the sentences quoted, which are the search engine's extracts of the page named)
  (verified through search results): Miller, Nezamabadi & Daly 2013 (SMPTE journal record, DOI
  10.5594/j18290) and the ST 2084 constants (colour-science's documentation); BT.2087-0 (ITU
  listing and abstract); BT.2408-8 and -9 (ITU PDF listings and extracts); Lottes GDC 2016 (GDC
  Vault 1023512, the GPUOpen PDF listing); Wronski 2016 (bartwronski.com, with date); Hart GDC 2018
  (GDC Vault 1024803, developer.nvidia.com/hdr-gdc-2018); the NVIDIA forum thread of 2024-03-24;
  the Windows HDR Calibration app (Microsoft Support and the DirectX blog); Fry 2017 (GDC Vault
  1024466, ea.com, the SlideShare listing); Narkowicz 2016 (title, date, the scRGB and 1000-nit fit
  statements); Patry 2016 and 2017 (glowybits titles, dates and summaries); Patry 2021 (the
  Advances 2021 course page and its announcement); Malin 2018 (research.activision.com, Digital
  Dragons, the mousefingers page); Penty 2019 (the Dev Days listing and Epic's interview);
  Unreal's HDR page and cvar descriptions (dev.epicgames.com, docs.unrealengine.com 4.27, Unreal
  Directive); Unity's manual pages (the same text as the GitHub source); HGiG's PDF (definitions
  and categories from the search extracts); the ACESCentral thread 5734 (title, opener, the quoted
  sentences); the GPUOpen FreeSync articles (titles and the latency paragraph).
- **Weaker confirmations, stated plainly.** The BT.2087 coefficients are from memory. HGiG's
  definitions come from search extracts of the PDF and of summaries of it; the exact wording may
  differ. The Penty HDR sentence is from Epic's interview, not the presentation. Fry 2017's content
  beyond its abstract is not quoted. The ACESCentral thread's year (2025) is inferred from its
  context (Godot's HDR work). OpenColorIO-Config-ACES 3.0.0's "March 15" carried no year on the
  extract; it is 2025 by the OCIO 2.4/2.5 timeline. `VK_EXT_swapchain_colorspace`'s metadata block
  says "Last Modified Date 2019-04-26" while its history lists a revision 5 of 2024-03-16; the entry
  cites the latter.
- **Forge's own numbers** (formats and extensions in `swapchain.rs` and `instance.rs`; the table's
  size, shaper and format in `aces2.rs`; the passes in `display.rs`, `taa.rs`, `overlay.rs`,
  `lib.rs`; the costs in D-022, D-024 and `PROFILE.md`) are as of 2026-09-26. The tonescale table in
  §4 and the PQ codes, step sizes and shaper densities in §1 and §4 were computed for this note from
  the constants in `aces2.rs` and the ST 2084 constants; they are not measurements and the unit
  tests of §5 should reproduce them before they enter D-022.
- **Numbers to re-check before they enter a spec:** every cost estimate in the recommendation (the
  encode's "few thousandths of a millisecond", the preview pass, the fp16 present bandwidth) is an
  estimate to be replaced by the F1 overlay's numbers; the "10 ms per preset" bake is the SDR figure
  and may grow with the wider shaper; Windows' default SDR white on the owner's machine is whatever
  the slider says, not 80; Unreal's cvar list moves between 5.x releases.
