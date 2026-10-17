//! The renderer's shared settings (D-069): one name, one range and one line of help for each,
//! whichever demo registers it, in the `r.` group. A demo registers those its frame has, with
//! its own default (the best look for its scene: ACES for the asteroids' black space, AgX's
//! punchy look elsewhere), reads them every frame, and its keys set them; a flag given on the
//! command line sets one as `--set` does.

use forge_core::settings::Spec;

use crate::display::Tonemap;
use crate::meshlet::SwRaster;

/// [`aa`]'s choices.
pub const AA: &[&str] = &["off", "taa", "dlaa"];
/// [`aa`]: none.
pub const AA_OFF: usize = 0;
/// [`aa`]: temporal anti-aliasing.
pub const AA_TAA: usize = 1;
/// [`aa`]: NVIDIA's DLAA.
pub const AA_DLAA: usize = 2;

/// Anti-aliasing.
pub fn aa(default: usize) -> Spec<usize> {
    Spec::choice(
        "r.aa",
        AA,
        default,
        "Anti-aliasing: off, taa, or dlaa (NVIDIA's DLAA, where the Streamline SDK and an RTX GPU \
         are; else TAA). T steps through them.",
    )
}

/// TAA's image sharpened (FidelityFX RCAS, D-045).
pub fn sharpen(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.taa.sharpen",
        default,
        "TAA's image sharpened by FidelityFX RCAS (D-045); r.taa.sharpen_stops sets how much.",
    )
}

/// How strongly: stops below the strongest (`--rcas`).
pub fn sharpen_stops(default: f64) -> Spec<f64> {
    Spec::float(
        "r.taa.sharpen_stops",
        default,
        0.0..=8.0,
        "How strongly TAA's image is sharpened: stops below RCAS's strongest, each half as strong.",
    )
}

/// [`tonemap`]'s choices, in [`Tonemap::ALL`]'s order, then the analytic reference.
pub const TONEMAPS: &[&str] = &[
    "agx",
    "agx-punchy",
    "aces",
    "neutral",
    "aces2",
    "aces2-analytic",
];

/// The tone curve.
pub fn tonemap(default: Tonemap) -> Spec<usize> {
    Spec::choice(
        "r.tonemap",
        TONEMAPS,
        tonemap_index(default),
        "The tone curve: agx, agx-punchy, aces, neutral, aces2 (aces2-analytic, the reference). G \
         steps through them.",
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
    Spec::bool(
        "r.bloom",
        default,
        "Bloom (B toggles it); r.bloom.strength sets how much.",
    )
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

/// [`local_exposure`]'s choices.
pub const LOCAL_EXPOSURE: &[&str] = &["off", "on", "view"];

/// Local exposure (D-067).
pub fn local_exposure(default: usize) -> Spec<usize> {
    Spec::choice(
        "r.exposure.local",
        LOCAL_EXPOSURE,
        default,
        "Local exposure (D-067): a dark room beside a sunlit doorway lifted; view shows its gain \
         in false colours. X steps through them.",
    )
}

/// Ambient occlusion (GTAO).
pub fn ao(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.ao",
        default,
        "Ambient occlusion of the sky's light (GTAO; N toggles it).",
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

/// The probes' diffuse light.
pub fn probes(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.probes",
        default,
        "The light probes' diffuse light, bounces included (P toggles them).",
    )
}

/// The sky's light.
pub fn sky_light(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.sky_light",
        default,
        "The sky's light on the surfaces, besides the sun's (I toggles it).",
    )
}

/// The sun's ray-traced shadows.
pub fn shadows(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.shadows",
        default,
        "The sun's ray-traced shadows (J toggles them).",
    )
}

/// The sky's reflection in glass and at grazing angles.
pub fn sky_reflections(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.reflections.sky",
        default,
        "The sky's reflection in glass, water and at grazing angles (F toggles it).",
    )
}

/// The mirror rays against the scene.
pub fn ray_reflections(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.reflections.rays",
        default,
        "Mirror rays against the scene in the smooth surfaces (Y toggles them).",
    )
}

/// The sun's disc at its true size.
pub fn sun_disc(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.sun.disc",
        default,
        "The sun as a disc of its true size (soft shadows); off, a point (Z toggles it).",
    )
}

/// Occlusion culling.
pub fn occlusion(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.cull.occlusion",
        default,
        "Two-pass occlusion culling against the depth pyramid (O toggles it); off, the same \
         pixels.",
    )
}

/// Level of detail.
pub fn lod(default: bool) -> Spec<bool> {
    Spec::bool(
        "r.lod",
        default,
        "Level of detail by the clusters' projected error (L toggles it); off, the finest.",
    )
}

/// The error in pixels a cluster may show.
pub fn lod_error(default: f64) -> Spec<f64> {
    Spec::float(
        "r.lod.error",
        default,
        0.125..=16.0,
        "The error in pixels a cluster may show before a finer one is drawn ([ and ] halve and \
         double it).",
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
         it), on. R steps through them.",
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
    Spec::bool(&format!("r.show.{name}"), false, help)
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
    }

    #[test]
    fn the_names_register() {
        let mut s = forge_core::settings::Settings::new();
        s.add(aa(AA_TAA));
        s.add(tonemap(Tonemap::AgXPunchy));
        s.add(show("ao", "the occlusion"));
        assert_eq!(
            s.info("r.tonemap").map(|v| v.value_text()).as_deref(),
            Some("agx-punchy")
        );
        assert!(s.info("r.show.ao").is_some());
    }
}
