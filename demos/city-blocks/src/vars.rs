//! The gallery's settings (D-069): the renderer's shared ones (`forge_render::settings`), each
//! registered with the default every demo shares and set to what the flags made of it, applied
//! to the gallery when they change, and set by its keys. The city, the island, the physics lab
//! and the city plan are galleries: one place for all four.

use forge_app::Context;
use forge_app::settings::{Settings, Source, Spec, Var, VarType};
use forge_render::CullFlags;
use forge_render::settings::{self as rs, AA_DLAA, AA_OFF, AA_TAA};
use winit::keyboard::KeyCode;

use crate::Gallery;

/// The handles of the gallery's settings.
pub(crate) struct RenderVars {
    aa: Var<usize>,
    sharpen: Var<bool>,
    sharpen_stops: Var<f64>,
    tonemap: Var<usize>,
    bloom: Var<bool>,
    bloom_strength: Var<f64>,
    local: Var<usize>,
    ao: Var<bool>,
    ao_radius: Var<f64>,
    probes: Var<bool>,
    sky_light: Var<bool>,
    sun_disc: Var<bool>,
    lod_error: Var<f64>,
    sw_raster: Var<usize>,
    wireframe: Var<bool>,
    /// The settings that are a bit of the culling flags: shadows, reflections, culling, LOD and
    /// the debug views.
    bits: Vec<(Var<bool>, u32)>,
}

/// Registers `spec` and, where the flags (or the scene) made its value another than the shared
/// default, sets it so, as a flag on the command line: a value from `forge.cfg` or `--set`
/// stays otherwise, and reaches the gallery at the first apply.
fn add<T: VarType + PartialEq>(s: &mut Settings, spec: Spec<T>, default: T, now: T) -> Var<T> {
    let var = s.add(spec);
    if now != default {
        s.set_by(var, now, Source::CommandLine);
    }
    var
}

/// [`add`] for a value the gallery keeps in `f32`, compared there: a default written in decimal
/// (0.04) is the flag's own.
fn add_f32(s: &mut Settings, spec: Spec<f64>, default: f64, now: f32) -> Var<f64> {
    let var = s.add(spec);
    if default as f32 != now {
        s.set_by(var, f64::from(now), Source::CommandLine);
    }
    var
}

impl Gallery {
    /// Registers the gallery's settings, from the state its flags gave it.
    pub(crate) fn register_vars(&mut self, s: &mut Settings) {
        let dlaa = self.dlaa.is_some();
        let aa_default = if dlaa { AA_DLAA } else { AA_TAA };
        let aa_now = if !self.taa.enabled {
            AA_OFF
        } else if self.dlaa_on {
            AA_DLAA
        } else {
            AA_TAA
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
                rs::show("meshlets", "Each meshlet in a colour of its own (M)."),
                CullFlags::MESHLET_COLORS,
                false,
            ),
            (
                rs::show("lod", "Each cluster coloured by its level of detail (K)."),
                CullFlags::LOD_COLORS,
                false,
            ),
            (
                rs::show("raster", "The software rasteriser's pixels in green (H)."),
                CullFlags::SHOW_RASTER,
                false,
            ),
            (
                rs::show("ao", "The ambient occlusion alone, as a grey surface (V)."),
                CullFlags::SHOW_AO,
                false,
            ),
            (
                rs::show(
                    "gi",
                    "The light the sky and the probes send a white surface (U).",
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
        ]
        .into_iter()
        .map(|(spec, b, default)| (add(s, spec, default, bit(b)), b))
        .collect();
        self.vars = Some(RenderVars {
            aa: add(s, rs::aa(aa_default), aa_default, aa_now),
            sharpen: add(s, rs::sharpen(true), true, self.taa.sharpen.is_some()),
            sharpen_stops: add_f32(
                s,
                rs::sharpen_stops(0.5),
                0.5,
                self.taa.sharpen.unwrap_or(self.args.rcas),
            ),
            tonemap: add(
                s,
                rs::tonemap(Tonemap::AgXPunchy),
                rs::tonemap_index(Tonemap::AgXPunchy),
                rs::tonemap_index(self.tonemap),
            ),
            bloom: add(s, rs::bloom(true), true, self.bloom_on),
            bloom_strength: add_f32(s, rs::bloom_strength(0.04), 0.04, self.taa.bloom_strength),
            local: add(s, rs::local_exposure(0), 0, local_now),
            ao: add(s, rs::ao(true), true, self.ao_on),
            ao_radius: add_f32(s, rs::ao_radius(1.5), 1.5, self.args.ao_radius),
            probes: add(s, rs::probes(true), true, self.probes_on),
            sky_light: add(s, rs::sky_light(true), true, self.sky_light),
            sun_disc: add(
                s,
                rs::sun_disc(true),
                true,
                self.renderer.sun_angular_radius > 0.0,
            ),
            lod_error: add_f32(s, rs::lod_error(1.0), 1.0, self.args.lod_error),
            sw_raster: add(
                s,
                rs::sw_raster(SwRaster::Auto),
                rs::sw_raster_index(SwRaster::Auto),
                rs::sw_raster_index(self.args.sw_raster),
            ),
            wireframe: add(
                s,
                rs::show("wireframe", "The triangles' edges over the frame (Tab)."),
                false,
                self.wireframe,
            ),
            bits,
        });
        // Values from forge.cfg and --set reach the gallery at the first apply.
        self.vars_seen = 0;
    }

    /// Applies the settings that changed since the last call to the gallery's state.
    pub(crate) fn apply_vars(&mut self, s: &Settings) {
        let Some(v) = &self.vars else {
            return;
        };
        let seen = self.vars_seen;
        if seen == s.changes() {
            return;
        }
        let changed = |var: Var<bool>| s.changed_since(var, seen);
        if s.changed_since(v.aa, seen)
            || changed(v.sharpen)
            || s.changed_since(v.sharpen_stops, seen)
        {
            let aa = s.get(v.aa);
            self.dlaa_on = aa == AA_DLAA && self.dlaa.is_some();
            self.taa.enabled = aa != AA_OFF;
            self.taa.sharpen = s.get(v.sharpen).then_some(s.get(v.sharpen_stops) as f32);
            self.taa.reset_history();
            if aa == AA_DLAA && self.dlaa.is_none() {
                tracing::info!(
                    "r.aa dlaa: DLAA does not run here (the Streamline SDK and an RTX GPU), TAA instead"
                );
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
        self.vars_seen = s.changes();
    }

    /// The keys bound to settings: they set them, and [`Gallery::apply_vars`] does the rest.
    /// Returns whether the key was one of them.
    pub(crate) fn vars_key(&mut self, ctx: &mut Context, code: KeyCode) -> bool {
        let Some(v) = &self.vars else {
            return false;
        };
        let s = &mut ctx.settings;
        let bit_var = |b: u32| {
            v.bits
                .iter()
                .find(|(_, bit)| *bit == b)
                .map(|&(var, _)| var)
        };
        let toggled = match code {
            KeyCode::KeyN => Some(s.toggle_var(v.ao)),
            KeyCode::KeyP => Some(s.toggle_var(v.probes)),
            KeyCode::KeyI => Some(s.toggle_var(v.sky_light)),
            KeyCode::KeyB => Some(s.toggle_var(v.bloom)),
            KeyCode::KeyX => Some(s.toggle_var(v.local)),
            KeyCode::KeyZ => Some(s.toggle_var(v.sun_disc)),
            KeyCode::KeyR => Some(s.toggle_var(v.sw_raster)),
            KeyCode::Tab => Some(s.toggle_var(v.wireframe)),
            KeyCode::KeyJ => bit_var(CullFlags::SHADOWS).map(|x| s.toggle_var(x)),
            KeyCode::KeyF => bit_var(CullFlags::SKY_REFLECTIONS).map(|x| s.toggle_var(x)),
            KeyCode::KeyY => bit_var(CullFlags::RAY_REFLECTIONS).map(|x| s.toggle_var(x)),
            KeyCode::KeyO => bit_var(CullFlags::OCCLUSION).map(|x| s.toggle_var(x)),
            KeyCode::KeyL => bit_var(CullFlags::LOD).map(|x| s.toggle_var(x)),
            KeyCode::KeyM => bit_var(CullFlags::MESHLET_COLORS).map(|x| s.toggle_var(x)),
            KeyCode::KeyK => bit_var(CullFlags::LOD_COLORS).map(|x| s.toggle_var(x)),
            KeyCode::KeyH => bit_var(CullFlags::SHOW_RASTER).map(|x| s.toggle_var(x)),
            KeyCode::KeyV => bit_var(CullFlags::SHOW_AO).map(|x| s.toggle_var(x)),
            KeyCode::KeyU => bit_var(CullFlags::SHOW_GI).map(|x| s.toggle_var(x)),
            KeyCode::KeyG => {
                let next = rs::tonemap_of(s.get(v.tonemap)).next();
                s.set(v.tonemap, rs::tonemap_index(next));
                Some(Ok(String::new()))
            }
            KeyCode::BracketLeft | KeyCode::BracketRight => {
                let e = s.get(v.lod_error);
                let e = if code == KeyCode::BracketLeft {
                    e * 0.5
                } else {
                    e * 2.0
                };
                s.set(v.lod_error, e);
                Some(Ok(String::new()))
            }
            // DLAA where it runs, TAA sharpened, plain, off (D-045).
            KeyCode::KeyT => {
                let next = match (s.get(v.aa), s.get(v.sharpen)) {
                    (AA_DLAA, _) => (AA_TAA, true),
                    (AA_TAA, true) => (AA_TAA, false),
                    (AA_TAA, false) => (AA_OFF, true),
                    _ if self.dlaa.is_some() => (AA_DLAA, true),
                    _ => (AA_TAA, true),
                };
                s.set(v.aa, next.0);
                s.set(v.sharpen, next.1);
                let name = match next {
                    (AA_DLAA, _) => "DLAA",
                    (AA_TAA, true) => "TAA, sharpened",
                    (AA_TAA, false) => "TAA",
                    _ => "none",
                };
                tracing::info!(anti_aliasing = name);
                Some(Ok(String::new()))
            }
            _ => None,
        };
        toggled.is_some()
    }
}

use forge_render::{SwRaster, Tonemap};
