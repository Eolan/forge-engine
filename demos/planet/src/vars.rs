//! The planet's settings (D-069): the renderer's shared ones (`forge_render::settings`) with the
//! planet's defaults (TAA, AgX's punchy look), and `planet.paused`. Registered from the state
//! its flags gave it, applied when they change, and set by its keys.

use forge_app::Context;
use forge_app::settings::{Settings, Spec, Var};
use forge_render::settings::{self as rs, AA_OFF, AA_TAA};
use forge_render::{CullFlags, Tonemap};
use winit::keyboard::KeyCode;

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
                rs::show(
                    "culled",
                    "What occlusion or cone culling rejected, in red (X).",
                ),
                CullFlags::SHOW_CULLED,
                false,
            ),
        ]
        .into_iter()
        .map(|(spec, b, default)| (s.add_from(spec, default, flags.has(b)), b))
        .collect();
        let aa_now = if self.taa_enabled { AA_TAA } else { AA_OFF };
        self.vars = Some(PlanetVars {
            aa: s.add_from(rs::aa(AA_TAA), AA_TAA, aa_now),
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
                rs::show("wireframe", "The triangles' edges over the frame (Tab)."),
                false,
                self.wireframe,
            ),
            paused: s.add(Spec::bool(
                "planet.paused",
                false,
                "The scripted flight stopped where it is (P toggles it).",
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
            if aa > AA_TAA {
                tracing::info!("r.aa: the planet has no DLAA or DLSS yet, TAA instead");
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

    /// The keys bound to settings: they set them, and [`PlanetDemo::apply_vars`] does the
    /// rest. Returns whether the key was one of them.
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
        let toggle = |s: &mut Settings, var: Option<Var<bool>>| {
            if let Some(var) = var {
                let _ = s.toggle_var(var);
            }
        };
        match code {
            KeyCode::KeyP => toggle(s, Some(v.paused)),
            KeyCode::KeyN => toggle(s, Some(v.ao)),
            KeyCode::Tab => toggle(s, Some(v.wireframe)),
            KeyCode::KeyJ => toggle(s, bit_var(CullFlags::SHADOWS)),
            KeyCode::KeyO => toggle(s, bit_var(CullFlags::OCCLUSION)),
            KeyCode::KeyC => toggle(s, bit_var(CullFlags::CONE)),
            KeyCode::KeyM => toggle(s, bit_var(CullFlags::MESHLET_COLORS)),
            KeyCode::KeyK => toggle(s, bit_var(CullFlags::LOD_COLORS)),
            KeyCode::KeyX => toggle(s, bit_var(CullFlags::SHOW_CULLED)),
            KeyCode::KeyG => {
                let next = rs::tonemap_of(s.get(v.tonemap)).next();
                s.set(v.tonemap, rs::tonemap_index(next));
            }
            KeyCode::BracketLeft => s.set(v.lod_error, s.get(v.lod_error) * 0.5),
            KeyCode::BracketRight => s.set(v.lod_error, s.get(v.lod_error) * 2.0),
            KeyCode::Minus => s.set(v.compensation, s.get(v.compensation) - 0.5),
            KeyCode::Equal => s.set(v.compensation, s.get(v.compensation) + 0.5),
            // TAA on and off.
            KeyCode::KeyT => {
                let aa = if s.get(v.aa) == AA_OFF {
                    AA_TAA
                } else {
                    AA_OFF
                };
                s.set(v.aa, aa);
            }
            _ => return false,
        }
        true
    }
}
