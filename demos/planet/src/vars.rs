//! The planet's settings (D-069): the renderer's shared ones (`forge_render::settings`) with the
//! planet's defaults (DLAA where it runs, else TAA; AgX's punchy look), and `planet.paused`.
//! Registered from the state its flags gave it, applied when they change, and set from the
//! console (no key sets them since D-069's key map).

use forge_app::settings::{Settings, Spec, Var};
use forge_render::settings::{self as rs, AA_DLAA, AA_DLSS, AA_FSR, AA_OFF, AA_TAA};
use forge_render::{CullFlags, Tonemap};

use crate::PlanetDemo;

/// The handles of the planet's settings.
pub(crate) struct PlanetVars {
    aa: Var<usize>,
    sharpen: Var<bool>,
    sharpen_stops: Var<f64>,
    tonemap: Var<usize>,
    bloom: Var<bool>,
    bloom_strength: Var<f64>,
    compensation: Var<f64>,
    ao: Var<bool>,
    lod_error: Var<f64>,
    wireframe: Var<bool>,
    paused: Var<bool>,
    /// The settings that are a bit of the culling flags.
    bits: Vec<(Var<bool>, u32)>,
}

impl PlanetDemo {
    /// Registers the planet's settings, from the state its flags gave it.
    pub(crate) fn register_vars(&mut self, s: &mut Settings) {
        let flags = self.flags;
        let bits = [
            (rs::shadows(true), CullFlags::SHADOWS, true),
            (rs::occlusion(true), CullFlags::OCCLUSION, true),
            (rs::cone(true), CullFlags::CONE, true),
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
                rs::show("culled", "What occlusion or cone culling rejected, in red."),
                CullFlags::SHOW_CULLED,
                false,
            ),
        ]
        .into_iter()
        .map(|(spec, b, default)| (s.add_from(spec, default, flags.has(b)), b))
        .collect();
        let aa_default = if self.dlaa.is_some() { AA_DLAA } else { AA_TAA };
        let aa_now = if !self.taa_enabled {
            AA_OFF
        } else if self.fsr_on {
            AA_FSR
        } else if self.dlaa_on {
            AA_DLAA
        } else {
            AA_TAA
        };
        self.vars = Some(PlanetVars {
            aa: s.add_from(rs::aa(aa_default), aa_default, aa_now),
            sharpen: s.add_from(rs::sharpen(true), true, self.taa.sharpen.is_some()),
            sharpen_stops: s.add_from_f32(
                rs::sharpen_stops(0.5),
                0.5,
                self.taa.sharpen.unwrap_or(self.args.rcas),
            ),
            tonemap: s.add_from(
                rs::tonemap(Tonemap::AgXPunchy),
                rs::tonemap_index(Tonemap::AgXPunchy),
                rs::tonemap_index(self.tonemap),
            ),
            bloom: s.add_from(rs::bloom(true), true, self.args.bloom > 0.0),
            bloom_strength: s.add_from_f32(rs::bloom_strength(0.04), 0.04, self.taa.bloom_strength),
            compensation: s.add_from_f32(
                rs::exposure_compensation(0.0),
                0.0,
                self.exposure.compensation,
            ),
            ao: s.add_from(rs::ao(true), true, self.ao_on),
            lod_error: s.add_from_f32(rs::lod_error(1.0), 1.0, self.args.lod_error),
            wireframe: s.add_from(
                rs::show("wireframe", "The triangles' edges over the frame."),
                false,
                self.wireframe,
            ),
            paused: s.add(Spec::bool(
                "planet.paused",
                false,
                "The scripted flight stopped where it is.",
            )),
            bits,
        });
        // Values from forge.cfg and --set reach the planet at the first apply.
        self.vars_seen = 0;
    }

    /// Applies the settings that changed since the last call.
    pub(crate) fn apply_vars(&mut self, s: &Settings) {
        let seen = self.vars_seen;
        if seen == s.changes() {
            return;
        }
        let Some(v) = &self.vars else {
            return;
        };
        let changed = |var: Var<bool>| s.changed_since(var, seen);
        if s.changed_since(v.aa, seen)
            || changed(v.sharpen)
            || s.changed_since(v.sharpen_stops, seen)
        {
            let aa = s.get(v.aa);
            let wanted = matches!(aa, AA_DLAA | AA_DLSS);
            if wanted && self.dlaa.is_none() {
                tracing::info!(
                    "r.aa: DLAA does not run here (the Streamline SDK and an RTX GPU, or a \
                     scripted run without --set r.aa=dlaa): TAA instead"
                );
            }
            // DLSS's smaller modes are the asteroids' alone: the planet shows dlss as DLAA.
            self.dlaa_on = wanted && self.dlaa.is_some();
            self.fsr_on = aa == AA_FSR && self.fsr.is_some();
            if let Some((fsr, _)) = self.fsr.as_mut().filter(|_| aa == AA_FSR) {
                fsr.reset_history();
            }
            if aa == AA_FSR && self.fsr.is_none() {
                tracing::info!(
                    "r.aa fsr: FSR is made when a run starts with --set r.aa=fsr; TAA instead"
                );
            }
            self.taa_enabled = aa != AA_OFF;
            self.taa.sharpen = s.get(v.sharpen).then_some(s.get(v.sharpen_stops) as f32);
            self.taa.reset_history();
        }
        if s.changed_since(v.tonemap, seen) {
            self.tonemap = rs::tonemap_of(s.get(v.tonemap));
        }
        if changed(v.bloom) || s.changed_since(v.bloom_strength, seen) {
            let strength = s.get(v.bloom_strength) as f32;
            self.taa.bloom_strength = strength;
            self.args.bloom = if s.get(v.bloom) { strength } else { 0.0 };
        }
        if s.changed_since(v.compensation, seen) {
            self.exposure.compensation = s.get(v.compensation) as f32;
        }
        if changed(v.ao) {
            self.ao_on = s.get(v.ao);
        }
        if s.changed_since(v.lod_error, seen) {
            self.args.lod_error = s.get(v.lod_error) as f32;
        }
        if changed(v.wireframe) {
            self.wireframe = s.get(v.wireframe);
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
}
