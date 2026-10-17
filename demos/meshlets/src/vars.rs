//! The bench's settings (D-069): the renderer's shared ones (`forge_render::settings`) it has,
//! with its meshlets in their colours by default. Registered from the state its flags gave it,
//! applied when they change, and set from the console (no key sets them since D-069's key map),
//! `meshlets.freeze` (F before it) among them.

use forge_app::settings::{Settings, Spec, Var};
use forge_render::settings as rs;
use forge_render::{CullFlags, SwRaster, Tonemap};

use crate::Bench;

/// The handles of the bench's settings.
pub(crate) struct BenchVars {
    tonemap: Var<usize>,
    lod_error: Var<f64>,
    sw_raster: Var<usize>,
    wireframe: Var<bool>,
    /// The culling camera held where it stands (F before D-069).
    freeze: Var<bool>,
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
                rs::show_default("meshlets", true, "Each meshlet in a colour of its own."),
                CullFlags::MESHLET_COLORS,
                true,
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
                rs::show("wireframe", "The triangles' edges over the frame."),
                false,
                self.wireframe,
            ),
            freeze: s.add(Spec::bool(
                "meshlets.freeze",
                false,
                "The culling camera held where it stands: fly out and see what it culled; a \n                 resize ends it.",
            )),
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
        if s.changed_since(v.freeze, seen) && self.flags.has(CullFlags::FREEZE) != s.get(v.freeze) {
            // The camera is taken at the next update.
            self.flags.toggle(CullFlags::FREEZE);
            self.frozen = None;
        }
        for &(var, bit) in &v.bits {
            if s.changed_since(var, seen) && self.flags.has(bit) != s.get(var) {
                self.flags.toggle(bit);
            }
        }
        self.vars_seen = s.changes();
    }
}

impl Bench {
    /// `meshlets.freeze` off, as a resize ends the freeze.
    pub(crate) fn unfreeze_var(&self, s: &mut Settings) {
        if let Some(v) = &self.vars {
            s.set(v.freeze, false);
        }
    }
}
