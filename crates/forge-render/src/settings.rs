//! The renderer's shared settings (D-069): one name, one range and one line of help for each,
//! whichever demo registers it, in the `r.` group. A demo registers those its frame has, with
//! its own default (the best look for its scene: ACES for the asteroids' black space, AgX's
//! punchy look elsewhere), reads them every frame, and the console sets them; a flag given on the
//! command line sets one as `--set` does.

use forge_core::settings::Spec;

use forge_gpu::DlssMode;

use crate::display::Tonemap;
use crate::fsr::FsrMode;
use crate::meshlet::SwRaster;

/// [`aa`]'s choices.
pub const AA: &[&str] = &["off", "dlaa", "dlss", "fsr"];
/// [`aa`]: none.
pub const AA_OFF: usize = 0;
/// [`aa`]: NVIDIA's DLAA.
pub const AA_DLAA: usize = 1;
/// [`aa`]: NVIDIA's DLSS, rendering smaller ([`dlss_mode`]).
pub const AA_DLSS: usize = 2;
/// [`aa`]: AMD's FSR 3.1 at the window's size (D-070), on any GPU.
pub const AA_FSR: usize = 3;

/// Anti-aliasing.
pub fn aa(default: usize) -> Spec<usize> {
    Spec::choice(
        "r.aa",
        AA,
        default,
        "Anti-aliasing: off, dlaa (NVIDIA's DLAA at the window's size), dlss (rendering smaller, \
         r.dlss.mode), or fsr (AMD's FSR 3.1 at the window's size, any GPU). DLAA and DLSS need \
         the Streamline SDK and an RTX GPU; without them, FSR. A demo that does not upscale shows \
         dlss as dlaa.",
    )
}

/// [`dlss_mode`]'s choices: DLSS's upscaling modes, after DLAA in [`DlssMode::ALL`].
pub const DLSS_MODES: &[&str] = &["quality", "balanced", "performance", "ultra-performance"];

/// How much smaller DLSS renders, with [`aa`] at `dlss`.
pub fn dlss_mode(default: DlssMode) -> Spec<usize> {
    Spec::choice(
        "r.dlss.mode",
        DLSS_MODES,
        dlss_mode_index(default),
        "How much smaller DLSS renders with r.aa dlss: quality, balanced, performance, \
         ultra-performance.",
    )
}

/// [`dlss_mode`]'s choice for `mode` (DLAA: quality's).
pub fn dlss_mode_index(mode: DlssMode) -> usize {
    DlssMode::ALL[1..]
        .iter()
        .position(|&m| m == mode)
        .unwrap_or(0)
}

/// The mode of [`dlss_mode`]'s choice `index`.
pub fn dlss_mode_of(index: usize) -> DlssMode {
    DlssMode::ALL[1..]
        .get(index)
        .copied()
        .unwrap_or(DlssMode::Quality)
}

/// [`fsr_mode`]'s choices: [`FsrMode::ALL`]'s names.
pub const FSR_MODES: &[&str] = &[
    "native",
    "quality",
    "balanced",
    "performance",
    "ultra-performance",
];

/// How much smaller FSR renders, with [`aa`] at `fsr` (D-070).
pub fn fsr_mode(default: FsrMode) -> Spec<usize> {
    Spec::choice(
        "r.fsr.mode",
        FSR_MODES,
        fsr_mode_index(default),
        "How much smaller FSR renders with r.aa fsr: native (its anti-aliasing at the window's \
         size), quality (1.5 times smaller), balanced (1.7), performance (2), ultra-performance \
         (3).",
    )
}

/// [`fsr_mode`]'s choice for `mode`.
pub fn fsr_mode_index(mode: FsrMode) -> usize {
    FsrMode::ALL.iter().position(|&m| m == mode).unwrap_or(0)
}

/// The mode of [`fsr_mode`]'s choice `index`.
pub fn fsr_mode_of(index: usize) -> FsrMode {
    FsrMode::ALL.get(index).copied().unwrap_or_default()
}

/// [`fsr_sharpen`]'s default in every demo: on (D-070).
pub const FSR_SHARPEN: bool = true;
/// [`fsr_sharpness`]'s default in every demo. At 0.5 a still edge is nearly an ideal pixel's
/// (MTF50 0.58 against 0.52 unsharpened) and the city shimmers less than under DLAA; at 0.75 it
/// shimmers more (D-070).
pub const FSR_SHARPNESS: f64 = 0.5;

/// FSR's image sharpened by AMD's RCAS after the accumulation (D-070).
pub fn fsr_sharpen(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.fsr.sharpen",
        default,
        "FSR's image sharpened by AMD's RCAS (D-070); r.fsr.sharpness sets how much.",
    )
}

/// How strongly RCAS sharpens FSR's image, on AMD's scale.
pub fn fsr_sharpness(default: f64) -> Spec<f64> {
    Spec::float(
        "r.fsr.sharpness",
        default,
        0.0..=1.0,
        "How strongly RCAS sharpens FSR's image, AMD's scale: 0 is two stops below its strongest, \
         1 its strongest.",
    )
}

/// [`tonemap`]'s choices, in [`Tonemap::ALL`]'s order, then the analytic reference.
pub const TONEMAPS: &[&str] = &[
    "agx",
    "agx-punchy",
    "aces",
    "neutral",
    "aces2",
    "aces-narkowicz",
    "aces2-analytic",
];

/// The tone curve.
pub fn tonemap(default: Tonemap) -> Spec<usize> {
    Spec::choice(
        "r.tonemap",
        TONEMAPS,
        tonemap_index(default),
        "The tone curve: agx, agx-punchy, aces, neutral, aces2, aces-narkowicz (Falcor's) \
         (aces2-analytic, the reference); toggle steps through them.",
    )
}

/// [`tonemap`]'s choice for `t`.
pub fn tonemap_index(t: Tonemap) -> usize {
    TONEMAPS.iter().position(|&n| n == t.name()).unwrap_or(0)
}

/// The curve of [`tonemap`]'s choice `index`.
pub fn tonemap_of(index: usize) -> Tonemap {
    TONEMAPS
        .get(index)
        .and_then(|n| n.parse().ok())
        .unwrap_or_default()
}

/// Bloom.
pub fn bloom(default: bool) -> Spec<bool> {
    Spec::bool("r.bloom", default, "Bloom; r.bloom.strength sets how much.")
}

/// Bloom's share of the shown image (`--bloom`).
pub fn bloom_strength(default: f64) -> Spec<f64> {
    Spec::float(
        "r.bloom.strength",
        default,
        0.0..=1.0,
        "Bloom's share of the shown image.",
    )
}

/// The metered exposure's compensation.
pub fn exposure_compensation(default: f64) -> Spec<f64> {
    Spec::float(
        "r.exposure.compensation",
        default,
        -8.0..=8.0,
        "The metered exposure's compensation in stops: positive brighter.",
    )
}

/// A night's exposure floor (`AutoExposure::night_floor`, research/home-lighting.md §8.2, D-080).
pub fn exposure_night_floor(default: f64) -> Spec<f64> {
    Spec::float(
        "r.exposure.night_floor",
        default,
        -6.0..=8.0,
        "At night, the least EV100 the exposure takes: 1 shows a lit village as a photographer \
         would, its lamps and windows not blown out; -6 keys the night on the moonlight alone.",
    )
}

/// [`local_exposure`]'s choices.
pub const LOCAL_EXPOSURE: &[&str] = &["off", "on", "view"];

/// Local exposure (D-067).
pub fn local_exposure(default: usize) -> Spec<usize> {
    Spec::choice(
        "r.exposure.local",
        LOCAL_EXPOSURE,
        default,
        "Local exposure (D-067): a dark room beside a sunlit doorway lifted; view shows its gain \
         in false colours.",
    )
}

/// Ambient occlusion (GTAO).
pub fn ao(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.ao",
        default,
        "Ambient occlusion of the sky's light (GTAO).",
    )
}

/// How far the occlusion reaches.
pub fn ao_radius(default: f64) -> Spec<f64> {
    Spec::float(
        "r.ao.radius",
        default,
        0.05..=16.0,
        "How far the ambient occlusion reaches, metres.",
    )
}

/// The textures' mip bias, stops (`MeshletRenderer::texture_lod_bias`): below 0 a sharper level
/// (NVIDIA's DLSS guide: log2(render / output) − 1, so −1 under DLAA, its jitter averaging the
/// finer texels).
pub fn texture_lod_bias(default: f64) -> Spec<f64> {
    Spec::float(
        "r.textures.lod_bias",
        default,
        -3.0..=3.0,
        "The textures' mip bias, stops: below 0 sharper (DLSS asks for -1 under DLAA).",
    )
}

/// A debug view of the standard rows' shading inputs (D-086).
pub fn show_input(default: i64) -> Spec<i64> {
    Spec::int(
        "r.show.input",
        default,
        0..=6,
        "A shading input instead of the light: 1 albedo, 2 roughness, 3 metalness, 4 normal, \
         5 the highlight's widening (Toksvig red, geometry green), 6 F0; 0 none.",
    )
}

/// How many frames the sun's shadows' denoiser steadies over (#337, SIGMA's stabilisation).
pub fn shadow_history(default: i64) -> Spec<i64> {
    Spec::int(
        "r.shadows.history",
        default,
        0..=7,
        "Frames the sun's shadows' denoiser (SIGMA) steadies over: fewer keep a moving caster's \
         shadow from trailing behind it, more steady a still penumbra.",
    )
}

/// What the ambient occlusion leaves out near the camera (#337).
pub fn ao_ignore_nearer(default: f64) -> Spec<f64> {
    Spec::float(
        "r.ao.ignore_nearer",
        default,
        0.0..=2.0,
        "The ambient occlusion leaves out what stands nearer the camera than this, metres (0: \
         nothing): a first-person weapon or hands then cast no dark halo on the wall behind them.",
    )
}

/// How fast the automatic exposure brightens the image when the scene darkens (#336).
pub fn exposure_speed_brighten(default: f64) -> Spec<f64> {
    Spec::float(
        "r.exposure.speed_brighten",
        default,
        0.05..=20.0,
        "How fast the automatic exposure brightens the image when the scene gets darker (walking \
         into a dark room), per second: most of the way in about 2 / speed seconds.",
    )
}

/// How fast the automatic exposure darkens the image when the scene brightens (#336).
pub fn exposure_speed_darken(default: f64) -> Spec<f64> {
    Spec::float(
        "r.exposure.speed_darken",
        default,
        0.05..=20.0,
        "How fast the automatic exposure darkens the image when the scene gets brighter (walking \
         out into the sun), per second: most of the way in about 2 / speed seconds.",
    )
}

/// The probes' diffuse light.
pub fn probes(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.probes",
        default,
        "The light probes' diffuse light, bounces included.",
    )
}

/// The sky's light.
pub fn sky_light(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.sky_light",
        default,
        "The sky's light on the surfaces, besides the sun's.",
    )
}

/// The sun's ray-traced shadows.
pub fn shadows(default: bool) -> Spec<bool> {
    Spec::bool("r.shadows", default, "The sun's ray-traced shadows.")
}

/// The sky's reflection in glass and at grazing angles.
pub fn sky_reflections(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.reflections.sky",
        default,
        "The sky's reflection in glass, water and at grazing angles.",
    )
}

/// D-088's references for the diffuse light round each pixel: measures, far from real time.
pub fn gi_reference(default: i64) -> Spec<i64> {
    Spec::int(
        "r.gi.reference",
        default,
        0..=4,
        "The diffuse light from 32 rays a pixel (D-088, slow): 1 to 2 m and the probes past them, \
         2 one bounce traced and the probes after, 3 two bounces, 4 a path without the probes; \
         0 the probes.",
    )
}

/// GGX without its multiple-scattering compensation (D-086).
pub fn single_scatter(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.specular.single_scatter",
        default,
        "GGX without its multiple-scattering compensation, as Falcor's (rough metals darker).",
    )
}

/// The mirror rays against the scene.
pub fn ray_reflections(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.reflections.rays",
        default,
        "Mirror rays against the scene in the smooth surfaces.",
    )
}

/// The mirrors' motion vectors moved towards their reflections.
pub fn reflection_motion(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.reflections.motion",
        default,
        "The smooth surfaces' motion vectors moved towards what they reflect, so the upscaler \n         follows the reflections; off, the surfaces.",
    )
}

/// The sun's disc at its true size.
pub fn sun_disc(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.sun.disc",
        default,
        "The sun as a disc of its true size (soft shadows); off, a point.",
    )
}

/// Occlusion culling.
pub fn occlusion(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.cull.occlusion",
        default,
        "Two-pass occlusion culling against the depth pyramid; off, the same \
         pixels.",
    )
}

/// Normal-cone culling.
pub fn cone(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.cull.cone",
        default,
        "Back-facing clusters culled by their normal cones; off, the same pixels.",
    )
}

/// Translucent surfaces' light through them.
pub fn translucency(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.translucency",
        default,
        "Light through thin translucent surfaces.",
    )
}

/// Level of detail.
pub fn lod(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.lod",
        default,
        "Level of detail by the clusters' projected error; off, the finest.",
    )
}

/// The error in pixels a cluster may show.
pub fn lod_error(default: f64) -> Spec<f64> {
    Spec::float(
        "r.lod.error",
        default,
        0.125..=16.0,
        "The error in pixels a cluster may show before a finer one is drawn.",
    )
}

/// [`sw_raster`]'s choices.
pub const SW_RASTER: &[&str] = &["off", "auto", "on"];

/// The software rasteriser.
pub fn sw_raster(default: SwRaster) -> Spec<usize> {
    Spec::choice(
        "r.sw_raster",
        SW_RASTER,
        sw_raster_index(default),
        "Small triangles rasterised in compute: off, auto (when enough dense triangles repay \
         it), on.",
    )
}

/// [`sw_raster`]'s choice for `mode`.
pub fn sw_raster_index(mode: SwRaster) -> usize {
    match mode {
        SwRaster::Off => 0,
        SwRaster::Auto => 1,
        SwRaster::On => 2,
    }
}

/// The mode of [`sw_raster`]'s choice `index`.
pub fn sw_raster_of(index: usize) -> SwRaster {
    match index {
        0 => SwRaster::Off,
        2 => SwRaster::On,
        _ => SwRaster::Auto,
    }
}

/// A debug view: `r.show.NAME`, off by default.
pub fn show(name: &'static str, help: &str) -> Spec<bool> {
    show_default(name, false, help)
}

/// A debug view on by default: the meshlets bench shows its meshlets' colours.
pub fn show_default(name: &'static str, default: bool, help: &str) -> Spec<bool> {
    Spec::bool(&format!("r.show.{name}"), default, help)
}

/// Frustum culling.
pub fn frustum(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.cull.frustum",
        default,
        "Instances and clusters outside the view culled; \
         off, the same pixels.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_map_both_ways() {
        for t in Tonemap::ALL.into_iter().chain([Tonemap::Aces2Analytic]) {
            assert_eq!(tonemap_of(tonemap_index(t)), t, "{}", t.name());
        }
        for mode in [SwRaster::Off, SwRaster::Auto, SwRaster::On] {
            assert_eq!(sw_raster_of(sw_raster_index(mode)), mode);
        }
        for mode in &DlssMode::ALL[1..] {
            assert_eq!(dlss_mode_of(dlss_mode_index(*mode)), *mode);
        }
        assert_eq!(DLSS_MODES.len(), DlssMode::ALL.len() - 1);
        for (index, mode) in FsrMode::ALL.into_iter().enumerate() {
            assert_eq!(FSR_MODES[index], mode.name());
            assert_eq!(fsr_mode_of(fsr_mode_index(mode)), mode);
        }
    }

    #[test]
    fn the_names_register() {
        let mut s = forge_core::settings::Settings::new();
        s.add(aa(AA_FSR));
        s.add(tonemap(Tonemap::AgXPunchy));
        s.add(show("ao", "the occlusion"));
        assert_eq!(
            s.info("r.tonemap").map(|v| v.value_text()).as_deref(),
            Some("agx-punchy")
        );
        assert!(s.info("r.show.ao").is_some());
    }
}
