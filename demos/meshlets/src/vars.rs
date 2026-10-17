//! The bench's settings (D-069): the renderer's shared ones (`forge_render::settings`) it has,
//! with its meshlets in their colours by default. Registered from the state its flags gave it,
//! applied when they change, and set by its keys (F, freezing the culling camera, stays a key).

use forge_app::Context;
use forge_app::settings::{Settings, Var};
use forge_render::settings as rs;
use forge_render::{CullFlags, SwRaster, Tonemap};
use winit::keyboard::KeyCode;

use crate::Bench;

/// The handles of the bench's settings.
pub(crate) struct BenchVars {
    tonemap: Var<usize>,
    lod_error: Var<f64>,
    sw_raster: Var<usize>,
    wireframe: Var<bool>,
    /// The settings that are a bit of the culling flags.
    bits: Vec<(Var<bool>, u32)>,
}

impl Bench {
    /// Registers the bench's settings, from the state its flags gave it.
    pub(crate) fn register_vars(&mut self, s: &mut Settings) {
        let flags = self.flags;
        let bits = [
            (rs::cone(true), CullFlags::CONE, true),
            (rs::frustum(true), CullFlags::FRUSTUM, true),
            (rs::occlusion(true), CullFlags::OCCLUSION, true),
            (rs::lod(true), CullFlags::LOD, true),
            (
                rs::show_default("meshlets", true, "Each meshlet in a colour of its own (M)."),
                CullFlags::MESHLET_COLORS,
                true,
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
        ]
        .into_iter()
        .map(|(spec, b, default)| (s.add_from(spec, default, flags.has(b)), b))
        .collect();
        self.vars = Some(BenchVars {
            tonemap: s.add_from(
                rs::tonemap(Tonemap::AgXPunchy),
                rs::tonemap_index(Tonemap::AgXPunchy),
                rs::tonemap_index(self.tonemap),
            ),
            lod_error: s.add_from_f32(rs::lod_error(1.0), 1.0, self.args.lod_error),
            sw_raster: s.add_from(
                rs::sw_raster(SwRaster::Auto),
                rs::sw_raster_index(SwRaster::Auto),
                rs::sw_raster_index(self.args.sw_raster),
            ),
            wireframe: s.add_from(
                rs::show("wireframe", "The triangles' edges over the frame (Tab)."),
                false,
                self.wireframe,
            ),
            bits,
        });
        // Values from forge.cfg and --set reach the bench at the first apply.
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
        if s.changed_since(v.tonemap, seen) {
            self.tonemap = rs::tonemap_of(s.get(v.tonemap));
        }
        if s.changed_since(v.lod_error, seen) {
            self.args.lod_error = s.get(v.lod_error) as f32;
        }
        if s.changed_since(v.sw_raster, seen) {
            self.args.sw_raster = rs::sw_raster_of(s.get(v.sw_raster));
        }
        if s.changed_since(v.wireframe, seen) {
            self.wireframe = s.get(v.wireframe);
        }
        for &(var, bit) in &v.bits {
            if s.changed_since(var, seen) && self.flags.has(bit) != s.get(var) {
                self.flags.toggle(bit);
            }
        }
        self.vars_seen = s.changes();
    }

    /// The keys bound to settings: they set them, and [`Bench::apply_vars`] does the rest.
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
        let toggle = |s: &mut Settings, var: Option<Var<bool>>| {
            if let Some(var) = var {
                let _ = s.toggle_var(var);
            }
        };
        match code {
            KeyCode::KeyC => toggle(s, bit_var(CullFlags::CONE)),
            KeyCode::KeyV => toggle(s, bit_var(CullFlags::FRUSTUM)),
            KeyCode::KeyO => toggle(s, bit_var(CullFlags::OCCLUSION)),
            KeyCode::KeyL => toggle(s, bit_var(CullFlags::LOD)),
            KeyCode::KeyM => toggle(s, bit_var(CullFlags::MESHLET_COLORS)),
            KeyCode::KeyK => toggle(s, bit_var(CullFlags::LOD_COLORS)),
            KeyCode::KeyH => toggle(s, bit_var(CullFlags::SHOW_RASTER)),
            KeyCode::Tab => toggle(s, Some(v.wireframe)),
            KeyCode::KeyR => {
                let _ = s.toggle_var(v.sw_raster);
            }
            KeyCode::KeyG => {
                let next = rs::tonemap_of(s.get(v.tonemap)).next();
                s.set(v.tonemap, rs::tonemap_index(next));
            }
            KeyCode::BracketLeft => s.set(v.lod_error, s.get(v.lod_error) * 0.5),
            KeyCode::BracketRight => s.set(v.lod_error, s.get(v.lod_error) * 2.0),
            _ => return false,
        }
        true
    }
}
