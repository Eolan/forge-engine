//! The gallery's settings (D-069): the renderer's shared ones (`forge_render::settings`), each
//! registered with the default every demo shares and set to what the flags made of it, applied
//! to the gallery when they change, set from the console (no key sets them since D-069's key
//! map). The city, the island, the physics lab and the city plan are galleries: one place for
//! all four.

use forge_app::Context;
use forge_app::settings::{Settings, Spec, Var};
use forge_render::CullFlags;
use forge_render::settings::{self as rs, AA_DLAA, AA_DLSS, AA_FSR, AA_OFF};

use crate::Gallery;

/// The handles of the gallery's settings.
pub(crate) struct RenderVars {
    aa: Var<usize>,
    fsr_sharpen: Var<bool>,
    fsr_sharpness: Var<f64>,
    tonemap: Var<usize>,
    bloom: Var<bool>,
    bloom_strength: Var<f64>,
    local: Var<usize>,
    ao: Var<bool>,
    ao_radius: Var<f64>,
    ao_ignore_nearer: Var<f64>,
    exposure_brighten: Var<f64>,
    exposure_darken: Var<f64>,
    probes: Var<bool>,
    sky_light: Var<bool>,
    sun_disc: Var<bool>,
    lod_error: Var<f64>,
    sw_raster: Var<usize>,
    wireframe: Var<bool>,
    /// The settings that are a bit of the culling flags: shadows, reflections, culling, LOD and
    /// the debug views.
    bits: Vec<(Var<bool>, u32)>,
    /// The sun's shadows' denoiser (`r.shadows.denoiser`, F6 before D-069), where there is one.
    denoiser: Option<Var<usize>>,
    /// Its frames of history (`r.shadows.history`, #337), where there is one.
    shadow_history: Option<Var<i64>>,
    /// The catalogue's stars and the Moon's albedo at night (`r.sky.real`, the period key
    /// before D-069), where the night has them.
    sky_real: Option<Var<bool>>,
    /// The tank's water view (`r.show.liquid`, 1 to 3 before D-069), where there is a tank.
    liquid: Option<Var<usize>>,
}

impl Gallery {
    /// Registers the gallery's settings, from the state its flags gave it.
    pub(crate) fn register_vars(&mut self, s: &mut Settings) {
        // DLAA where it runs, else FSR (D-070).
        let aa_default = if self.dlaa.is_some() { AA_DLAA } else { AA_FSR };
        let aa_now = if self.dlaa_on {
            AA_DLAA
        } else if self.fsr_on {
            AA_FSR
        } else {
            AA_OFF
        };
        let local_now = match (self.local_settings.enabled, self.local_settings.view) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        };
        let flags = self.flags;
        let bit = |b: u32| flags.has(b);
        let bits = [
            (rs::shadows(true), CullFlags::SHADOWS, true),
            (rs::sky_reflections(true), CullFlags::SKY_REFLECTIONS, true),
            (rs::ray_reflections(true), CullFlags::RAY_REFLECTIONS, true),
            (rs::occlusion(true), CullFlags::OCCLUSION, true),
            (rs::lod(true), CullFlags::LOD, true),
            (
                rs::show("meshlets", "Each meshlet in a colour of its own."),
                CullFlags::MESHLET_COLORS,
                false,
            ),
            (
                rs::show("lod", "Each cluster coloured by its level of detail."),
                CullFlags::LOD_COLORS,
                false,
            ),
            (
                rs::show("raster", "The software rasteriser's pixels in green."),
                CullFlags::SHOW_RASTER,
                false,
            ),
            (
                rs::show("ao", "The ambient occlusion alone, as a grey surface."),
                CullFlags::SHOW_AO,
                false,
            ),
            (
                rs::show(
                    "gi",
                    "The light the sky and the probes send a white surface.",
                ),
                CullFlags::SHOW_GI,
                false,
            ),
            (
                rs::show("culled", "What occlusion or cone culling rejected, in red."),
                CullFlags::SHOW_CULLED,
                false,
            ),
            (
                rs::show(
                    "regions",
                    "Each surface tinted by the probe region lighting it (D-068).",
                ),
                CullFlags::SHOW_REGIONS,
                false,
            ),
            (
                rs::show(
                    "ladder",
                    "The meshes instances cross to on their ladders, in blue (D-075).",
                ),
                CullFlags::SHOW_LADDER,
                false,
            ),
        ]
        .into_iter()
        .map(|(spec, b, default)| (s.add_from(spec, default, bit(b)), b))
        .collect();
        // What only some galleries have: the shadows' denoisers, the real night sky, the tank.
        let denoiser = (!self.sun_shadows.is_empty()).then(|| {
            let now = match self.sun_shadow_on {
                Some(index) => {
                    let name = self.sun_shadows[index].kind().name();
                    DENOISERS
                        .iter()
                        .position(|d| d.eq_ignore_ascii_case(name))
                        .unwrap_or(0)
                }
                None => DENOISERS.len() - 1,
            };
            s.add_from(
                Spec::choice(
                    "r.shadows.denoiser",
                    DENOISERS,
                    0,
                    "The sun's soft shadows' denoiser: NVIDIA's SIGMA where NRD's library is, \
                     AMD's FidelityFX, or none (one ray a frame and the anti-aliasing's average).",
                ),
                0,
                now,
            )
        });
        let sky_real = self.night.as_ref().filter(|n| n.has_real()).map(|night| {
            s.add_from(
                Spec::bool(
                    "r.sky.real",
                    false,
                    "At night, the catalogue's stars and the Moon's albedo (D-046).",
                ),
                false,
                night.show_real,
            )
        });
        let liquid = self.liquid.is_some().then(|| {
            let now = match self.liquid_mode {
                forge_render::LiquidMode::Look => 0,
                forge_render::LiquidMode::Speed => 1,
                forge_render::LiquidMode::Landing => 2,
            };
            s.add_from(
                Spec::choice(
                    "r.show.liquid",
                    LIQUID_VIEWS,
                    0,
                    "The tank's water as it looks, coloured by its speed, or by where its rays \
                     land (#156).",
                ),
                0,
                now,
            )
        });
        self.vars = Some(RenderVars {
            aa: s.add_from(rs::aa(aa_default), aa_default, aa_now),
            fsr_sharpen: s.add(rs::fsr_sharpen(rs::FSR_SHARPEN)),
            fsr_sharpness: s.add(rs::fsr_sharpness(rs::FSR_SHARPNESS)),
            tonemap: s.add_from(
                rs::tonemap(Tonemap::AgXPunchy),
                rs::tonemap_index(Tonemap::AgXPunchy),
                rs::tonemap_index(self.tonemap),
            ),
            bloom: s.add_from(rs::bloom(true), true, self.bloom_on),
            bloom_strength: s.add_from_f32(rs::bloom_strength(0.04), 0.04, self.taa.bloom_strength),
            local: s.add_from(rs::local_exposure(0), 0, local_now),
            ao: s.add_from(rs::ao(true), true, self.ao_on),
            ao_radius: s.add_from_f32(rs::ao_radius(1.5), 1.5, self.args.ao_radius),
            ao_ignore_nearer: s.add(rs::ao_ignore_nearer(0.0)),
            exposure_brighten: s.add_from_f32(
                rs::exposure_speed_brighten(0.8),
                0.8,
                self.auto_exposure.speed_brighten,
            ),
            exposure_darken: s.add_from_f32(
                rs::exposure_speed_darken(1.5),
                1.5,
                self.auto_exposure.speed_darken,
            ),
            probes: s.add_from(rs::probes(true), true, self.probes_on),
            sky_light: s.add_from(rs::sky_light(true), true, self.sky_light),
            sun_disc: s.add_from(
                rs::sun_disc(true),
                true,
                self.renderer.sun_angular_radius > 0.0,
            ),
            lod_error: s.add_from_f32(rs::lod_error(1.0), 1.0, self.args.lod_error),
            sw_raster: s.add_from(
                rs::sw_raster(SwRaster::Auto),
                rs::sw_raster_index(SwRaster::Auto),
                rs::sw_raster_index(self.args.sw_raster),
            ),
            wireframe: s.add_from(
                rs::show("wireframe", "The triangles' edges over the frame."),
                false,
                self.wireframe,
            ),
            bits,
            denoiser,
            shadow_history: (!self.sun_shadows.is_empty()).then(|| {
                let history = forge_render::sun_shadow::SHADOW_HISTORY as i64;
                s.add(rs::shadow_history(history))
            }),
            sky_real,
            liquid,
        });
        // FSR sharpened as registered (D-070): the first apply only runs for what forge.cfg and
        // --set changed.
        if let Some(fsr) = self.fsr.as_mut() {
            fsr.sharpness = rs::FSR_SHARPEN.then_some(rs::FSR_SHARPNESS as f32);
        }
        // Values from forge.cfg and --set reach the gallery at the first apply.
        self.vars_seen = 0;
    }

    /// Applies the settings that changed since the last call to the gallery's state.
    pub(crate) fn apply_vars(&mut self, ctx: &Context) {
        let s = &ctx.settings;
        let Some(v) = &self.vars else {
            return;
        };
        let seen = self.vars_seen;
        if seen == s.changes() {
            return;
        }
        let changed = |var: Var<bool>| s.changed_since(var, seen);
        if s.changed_since(v.aa, seen) {
            let aa = s.get(v.aa);
            let dlaa_asked = matches!(aa, AA_DLAA | AA_DLSS);
            self.dlaa_on = dlaa_asked && self.dlaa.is_some();
            // FSR where it is asked for, and in DLAA's place where the device has no DLSS (D-070):
            // made the first time it is wanted.
            let fsr_wanted = aa == AA_FSR || dlaa_asked && self.dlaa.is_none();
            if fsr_wanted && self.fsr.is_none() {
                match forge_render::FsrUpscaler::new(
                    &ctx.device,
                    &ctx.shaders,
                    forge_render::FsrMode::NativeAa,
                    ctx.extent(),
                ) {
                    Ok(fsr) => self.fsr = Some(fsr),
                    Err(error) => tracing::error!(%error, "r.aa: no FSR, no anti-aliasing"),
                }
            }
            self.fsr_on = fsr_wanted && self.fsr.is_some();
            if let Some(fsr) = self.fsr.as_mut().filter(|_| fsr_wanted) {
                fsr.reset_history();
            }
            if dlaa_asked && self.dlaa.is_none() {
                tracing::info!(
                    "r.aa dlaa: DLAA does not run here (the Streamline SDK and an RTX GPU), FSR instead"
                );
            }
            // The jitter only where something resolves it.
            self.taa.enabled = self.dlaa_on || self.fsr_on;
            self.taa.reset_history();
        }
        if s.changed_since(v.aa, seen)
            || changed(v.fsr_sharpen)
            || s.changed_since(v.fsr_sharpness, seen)
        {
            // RCAS after FSR (D-070), on whichever FSR the anti-aliasing made.
            let sharpness = s
                .get(v.fsr_sharpen)
                .then_some(s.get(v.fsr_sharpness) as f32);
            if let Some(fsr) = self.fsr.as_mut() {
                fsr.sharpness = sharpness;
            }
        }
        if s.changed_since(v.tonemap, seen) {
            self.tonemap = rs::tonemap_of(s.get(v.tonemap));
        }
        if changed(v.bloom) || s.changed_since(v.bloom_strength, seen) {
            let strength = s.get(v.bloom_strength) as f32;
            self.taa.bloom_strength = strength;
            self.bloom_on = s.get(v.bloom) && strength > 0.0;
        }
        if s.changed_since(v.local, seen) {
            let local = s.get(v.local);
            self.local_settings.enabled = local > 0;
            self.local_settings.view = local == 2;
            let line = self.local_settings.line(self.local_middle());
            tracing::info!(%line, "local exposure");
        }
        if changed(v.ao) {
            self.ao_on = s.get(v.ao);
        }
        if s.changed_since(v.ao_radius, seen) {
            self.args.ao_radius = s.get(v.ao_radius) as f32;
        }
        if s.changed_since(v.ao_ignore_nearer, seen) {
            self.ao_ignore_nearer = s.get(v.ao_ignore_nearer) as f32;
        }
        if s.changed_since(v.exposure_brighten, seen) {
            self.auto_exposure.speed_brighten = s.get(v.exposure_brighten) as f32;
        }
        if s.changed_since(v.exposure_darken, seen) {
            self.auto_exposure.speed_darken = s.get(v.exposure_darken) as f32;
        }
        if changed(v.probes) {
            self.probes_on = s.get(v.probes);
        }
        if changed(v.sky_light) {
            self.sky_light = s.get(v.sky_light);
        }
        if changed(v.sun_disc) {
            self.renderer.sun_angular_radius = if s.get(v.sun_disc) {
                forge_render::starfield::SUN_ANGULAR_RADIUS_1AU * self.sun_scale
            } else {
                0.0
            };
        }
        if s.changed_since(v.lod_error, seen) {
            self.args.lod_error = s.get(v.lod_error) as f32;
        }
        if s.changed_since(v.sw_raster, seen) {
            self.args.sw_raster = rs::sw_raster_of(s.get(v.sw_raster));
        }
        if changed(v.wireframe) {
            self.wireframe = s.get(v.wireframe);
        }
        for &(var, bit) in &v.bits {
            if changed(var) && self.flags.has(bit) != s.get(var) {
                self.flags.toggle(bit);
            }
        }
        if let Some(var) = v.shadow_history
            && s.changed_since(var, seen)
        {
            let history = s.get(var) as u32;
            for denoiser in &mut self.sun_shadows {
                denoiser.history = history;
            }
        }
        if let Some(var) = v.denoiser
            && s.changed_since(var, seen)
        {
            let wanted = DENOISERS[s.get(var)];
            let index = self
                .sun_shadows
                .iter()
                .position(|d| d.kind().name().eq_ignore_ascii_case(wanted));
            match index {
                Some(index) => {
                    self.sun_shadows[index].reset_history();
                    self.sun_shadow_on = Some(index);
                }
                None if wanted == "none" => self.sun_shadow_on = None,
                None => tracing::info!("r.shadows.denoiser {wanted}: not here (no NRD?)"),
            }
        }
        if let Some(var) = v.sky_real
            && changed(var)
            && let Some(night) = self.night.as_mut()
        {
            night.show_real = s.get(var);
        }
        if let Some(var) = v.liquid
            && s.changed_since(var, seen)
        {
            self.liquid_mode = match s.get(var) {
                0 => forge_render::LiquidMode::Look,
                1 => forge_render::LiquidMode::Speed,
                _ => forge_render::LiquidMode::Landing,
            };
        }
        self.vars_seen = s.changes();
    }
}

/// `r.shadows.denoiser`'s choices: NVIDIA's SIGMA (where NRD's library is), AMD's FidelityFX,
/// none (the resolve's own ray a frame).
const DENOISERS: &[&str] = &["sigma", "ffx", "none"];
/// `r.show.liquid`'s choices: the tank's water as it looks, coloured by its speed, or by where
/// its rays land (#156).
const LIQUID_VIEWS: &[&str] = &["look", "speed", "landing"];

use forge_render::{SwRaster, Tonemap};
