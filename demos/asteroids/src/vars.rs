//! The ballad's settings (D-069): the renderer's shared ones (`forge_render::settings`) with the
//! ballad's own defaults (ACES, whose toe keeps space black; hard shadows; a 2 m occlusion), and
//! its own `asteroids.` ones. Registered from the state its flags gave it, applied when they
//! change, and set from the console (no key sets them since D-069's key map).

use forge_app::Context;
use forge_app::settings::{Settings, Spec, Var};
use forge_render::settings::{self as rs, AA_DLAA, AA_DLSS, AA_OFF, AA_TAA};
use forge_render::{CullFlags, DlssMode, SwRaster, Tonemap};

use crate::Ballad;

/// The handles of the ballad's settings.
#[derive(Clone)]
pub(crate) struct BalladVars {
    aa: Var<usize>,
    dlss_mode: Var<usize>,
    sharpen: Var<bool>,
    sharpen_stops: Var<f64>,
    tonemap: Var<usize>,
    bloom: Var<bool>,
    bloom_strength: Var<f64>,
    compensation: Var<f64>,
    ao: Var<bool>,
    ao_radius: Var<f64>,
    sun_disc: Var<bool>,
    lod_error: Var<f64>,
    sw_raster: Var<usize>,
    wireframe: Var<bool>,
    dust: Var<bool>,
    paused: Var<bool>,
    /// The settings that are a bit of the culling flags.
    bits: Vec<(Var<bool>, u32)>,
}

impl Ballad {
    /// The anti-aliasing on screen as `r.aa` and `r.dlss.mode` name it.
    fn aa_now(&self) -> (usize, DlssMode) {
        match &self.dlss {
            Some(dlss) if self.dlss_on && dlss.mode() == DlssMode::Dlaa => {
                (AA_DLAA, DlssMode::Quality)
            }
            Some(dlss) if self.dlss_on => (AA_DLSS, dlss.mode()),
            _ if !self.taa_enabled => (AA_OFF, DlssMode::Quality),
            _ => (AA_TAA, DlssMode::Quality),
        }
    }

    /// Registers the ballad's settings, from the state its flags gave it.
    pub(crate) fn register_vars(&mut self, s: &mut Settings) {
        let aa_default = if self.dlss.is_some() { AA_DLAA } else { AA_TAA };
        let (aa_now, mode_now) = self.aa_now();
        let flags = self.flags;
        let bits = [
            (rs::shadows(true), CullFlags::SHADOWS, true),
            (rs::translucency(true), CullFlags::TRANSLUCENCY, true),
            (rs::occlusion(true), CullFlags::OCCLUSION, true),
            (rs::cone(true), CullFlags::CONE, true),
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
                rs::show("culled", "What occlusion or cone culling rejected, in red."),
                CullFlags::SHOW_CULLED,
                false,
            ),
        ]
        .into_iter()
        .map(|(spec, b, default)| (s.add_from(spec, default, flags.has(b)), b))
        .collect();
        self.vars = Some(BalladVars {
            aa: s.add_from(rs::aa(aa_default), aa_default, aa_now),
            dlss_mode: s.add_from(
                rs::dlss_mode(DlssMode::Quality),
                rs::dlss_mode_index(DlssMode::Quality),
                rs::dlss_mode_index(mode_now),
            ),
            sharpen: s.add_from(rs::sharpen(true), true, self.taa.sharpen.is_some()),
            sharpen_stops: s.add_from_f32(
                rs::sharpen_stops(0.5),
                0.5,
                self.taa.sharpen.unwrap_or(self.args.rcas),
            ),
            tonemap: s.add_from(
                rs::tonemap(Tonemap::Aces),
                rs::tonemap_index(Tonemap::Aces),
                rs::tonemap_index(self.tonemap),
            ),
            bloom: s.add_from(rs::bloom(true), true, self.bloom_on),
            bloom_strength: s.add_from_f32(rs::bloom_strength(0.04), 0.04, self.taa.bloom_strength),
            compensation: s.add_from_f32(
                rs::exposure_compensation(0.0),
                0.0,
                self.exposure.compensation,
            ),
            ao: s.add_from(rs::ao(true), true, self.ao_on),
            ao_radius: s.add_from_f32(rs::ao_radius(2.0), 2.0, self.args.ao_radius),
            sun_disc: s.add_from(
                rs::sun_disc(false),
                false,
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
            dust: s.add_from(
                Spec::bool(
                    "asteroids.dust",
                    true,
                    "Sunlit dust between the rocks, on devices with ray queries.",
                ),
                true,
                self.dust_on,
            ),
            paused: s.add(Spec::bool(
                "asteroids.paused",
                false,
                "The flight along the belt stopped.",
            )),
            bits,
        });
        // Values from forge.cfg and --set reach the ballad at the first apply.
        self.vars_seen = 0;
    }

    /// Applies the settings that changed since the last call; DLSS's mode needs the device.
    pub(crate) fn apply_vars(&mut self, ctx: &Context) {
        let s = &ctx.settings;
        let seen = self.vars_seen;
        if seen == s.changes() {
            return;
        }
        let Some(v) = self.vars.clone() else {
            return;
        };
        let changed = |var: Var<bool>| s.changed_since(var, seen);
        if s.changed_since(v.aa, seen)
            || s.changed_since(v.dlss_mode, seen)
            || changed(v.sharpen)
            || s.changed_since(v.sharpen_stops, seen)
        {
            let aa = s.get(v.aa);
            let mode = if aa == AA_DLSS {
                rs::dlss_mode_of(s.get(v.dlss_mode))
            } else {
                DlssMode::Dlaa
            };
            self.taa_enabled = aa != AA_OFF;
            self.taa.sharpen = s.get(v.sharpen).then_some(s.get(v.sharpen_stops) as f32);
            let wanted = matches!(aa, AA_DLAA | AA_DLSS);
            if wanted && self.dlss.is_none() {
                tracing::warn!(
                    "DLSS is not available (--features dlss, the Streamline SDK, an RTX GPU): TAA"
                );
            }
            let before = self.aa_now();
            ctx.device.wait_idle();
            let configured: anyhow::Result<()> = match &mut self.dlss {
                Some(dlss) if wanted => {
                    self.dlss_on = true;
                    dlss.configure(mode, ctx.extent()).map_err(Into::into)
                }
                _ => {
                    self.dlss_on = false;
                    Ok(())
                }
            };
            let applied: anyhow::Result<()> =
                configured.and_then(|()| self.apply_upscaler(ctx.extent()));
            match applied {
                Ok(()) if self.aa_now() != before => tracing::info!(
                    upscaler = self.upscaler_label(),
                    render = ?self.render_extent,
                    "anti-aliasing switched"
                ),
                Ok(()) => {}
                Err(error) => tracing::error!(%error, "cannot switch the anti-aliasing"),
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
        if s.changed_since(v.compensation, seen) {
            self.exposure.compensation = s.get(v.compensation) as f32;
        }
        if changed(v.ao) {
            self.ao_on = s.get(v.ao);
        }
        if s.changed_since(v.ao_radius, seen) {
            self.args.ao_radius = s.get(v.ao_radius) as f32;
        }
        if changed(v.sun_disc) {
            self.renderer.sun_angular_radius = if s.get(v.sun_disc) {
                forge_render::starfield::SUN_ANGULAR_RADIUS_1AU
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
        if changed(v.dust) {
            self.dust_on = s.get(v.dust);
        }
        if changed(v.paused) {
            self.paused = s.get(v.paused);
        }
        for &(var, bit) in &v.bits {
            if changed(var) && self.flags.has(bit) != s.get(var) {
                self.flags.toggle(bit);
            }
        }
        self.vars_seen = s.changes();
    }

    /// TAA → DLAA → Quality → Balanced → Performance → Ultra Performance → TAA (the U key and
    /// `--cycle-upscaler`), through the settings.
    pub(crate) fn cycle_upscaler(&mut self, ctx: &mut Context) {
        let Some(v) = &self.vars else {
            return;
        };
        if self.dlss.is_none() {
            tracing::warn!(
                "DLSS is not available (--features dlss, the Streamline SDK, an RTX GPU)"
            );
            return;
        }
        let s = &mut ctx.settings;
        let next = match (s.get(v.aa), rs::dlss_mode_of(s.get(v.dlss_mode))) {
            (AA_DLAA, _) => (AA_DLSS, DlssMode::Quality),
            (AA_DLSS, mode) => {
                let i = DlssMode::ALL.iter().position(|&m| m == mode).unwrap_or(0);
                match DlssMode::ALL.get(i + 1) {
                    Some(&m) => (AA_DLSS, m),
                    None => (AA_TAA, DlssMode::Quality),
                }
            }
            _ => (AA_DLAA, DlssMode::Quality),
        };
        s.set(v.aa, next.0);
        s.set(v.dlss_mode, rs::dlss_mode_index(next.1));
    }
}
