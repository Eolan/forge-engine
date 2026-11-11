//! Relief inside a surface without geometry (D-107): the settings of the march the resolve makes
//! through a row's height map, and a CPU mirror of `shaders/parallax.slang` for the tests, term by
//! term (as [`crate::visibility`] mirrors the barycentrics).
//!
//! A height map's value is 1 on the mesh's surface and 0 on the relief's floor; the mesh is the
//! relief's top. Rays, depth and motion vectors stay the mesh's.

use glam::{Vec2, Vec3};

/// `RELIEF_STEPS_MOST` in `parallax.slang`: the most steps a march takes along the view.
pub const RELIEF_STEPS_MOST: u32 = 64;
/// `RELIEF_SHADOW_STEPS_MOST`: the most a march takes along a light.
pub const RELIEF_SHADOW_STEPS_MOST: u32 = 16;
const RELIEF_STEPS_LEAST: u32 = 4;
const RELIEF_FADE_LEVELS: f32 = 2.0;
const RELIEF_REFINE: u32 = 3;
/// `OPTION_RELIEF` in `meshlet.slang` (bit 4: 1 and 2 are the mirrors' and the screen reflections').
const OPTION_RELIEF: u32 = 16;
/// `OPTION_MICRO_BUMP`.
const OPTION_MICRO_BUMP: u32 = 4;
/// `OPTION_AO_MIN`.
const OPTION_AO_MIN: u32 = 8;

/// The switches of relief and micro-shadows (D-107), the frame's (`r.pom`, `r.micro.*`,
/// `r.ao.material`). With [`Self::enabled`] off, and the micro-shadows at their defaults, every row
/// shades as it did before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReliefSettings {
    /// Whether rows with a height map march it (`r.pom`).
    pub enabled: bool,
    /// The most steps along the view, at a grazing angle (`r.pom.steps`).
    pub steps: u32,
    /// Steps along the sun and the strongest lamp, 0: no self-shadow (`r.pom.shadow_steps`).
    pub shadow_steps: u32,
    /// The level of detail its relief starts to fade at (`r.pom.fade`): 0 is one texel a pixel.
    pub fade: f32,
    /// Every row's relief depth times this (`r.pom.scale`).
    pub scale: f32,
    /// How much of its micro-shadow an occlusion map casts on direct light, 0 to 1 (`r.micro.ao`).
    pub micro_ao: f32,
    /// A mapped normal's terminator softened against its surface's (`r.micro.bump`).
    pub micro_bump: bool,
    /// An occlusion map joins GTAO by their minimum, not their product (`r.ao.material`).
    pub ao_min: bool,
}

impl Default for ReliefSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            steps: 32,
            shadow_steps: 8,
            fade: 0.5,
            scale: 1.0,
            micro_ao: 0.0,
            // Off: the full capture batch showed it flattening sunlit bumps (the yard's pebbles lost
            // their lit rims, 10 % of the frame), a loss for the look (D-107, 2026-11-11).
            micro_bump: false,
            // On since 2026-11-11 (D-107's A/B): an occlusion map and GTAO by their minimum, which
            // no longer darkens twice a crevice both see.
            ao_min: true,
        }
    }
}

impl ReliefSettings {
    /// Its bits of `Frame::options`.
    pub(crate) fn option_bits(&self) -> u32 {
        (if self.enabled { OPTION_RELIEF } else { 0 })
            | (if self.micro_bump {
                OPTION_MICRO_BUMP
            } else {
                0
            })
            | (if self.ao_min { OPTION_AO_MIN } else { 0 })
    }

    /// `Frame::relief_steps`: the view's steps in its low byte, each light's in the next.
    pub(crate) fn step_word(&self) -> u32 {
        self.steps.clamp(RELIEF_STEPS_LEAST, RELIEF_STEPS_MOST)
            | self.shadow_steps.min(RELIEF_SHADOW_STEPS_MOST) << 8
    }
}

/// `relief_fade`: 1 whole to 0 gone by the level of detail a pixel reads, from `start` over
/// [`RELIEF_FADE_LEVELS`].
pub fn relief_fade(level: f32, start: f32) -> f32 {
    let t = ((level - start) / RELIEF_FADE_LEVELS).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// `relief_steps`: a step per texel crossed, never more than the angle's share of `most`.
pub fn relief_steps(span: f32, ndv: f32, most: u32) -> u32 {
    let angle = most - ((most - RELIEF_STEPS_LEAST) as f32 * ndv.clamp(0.0, 1.0)) as u32;
    (span.ceil() as u32 + 1).min(angle.max(RELIEF_STEPS_LEAST))
}

/// `relief_march`: how far down the relief the view ray first meets it, 0 to 1, for a height
/// function already over its top (`relief_height`). `across`: the map's coordinates' change from
/// the surface to the floor along the view.
pub fn relief_march(height: &impl Fn(Vec2) -> f32, at: Vec2, across: Vec2, steps: u32) -> f32 {
    let mut g0 = height(at) - 1.0;
    if g0 >= 0.0 {
        return 0.0;
    }
    let mut s0 = 0.0_f32;
    let mut s1 = 1.0_f32;
    let mut g1 = 0.0_f32;
    let stride = 1.0 / steps.max(1) as f32;
    for i in 1..=steps {
        let s = i as f32 * stride;
        let g = s + height(at + across * s) - 1.0;
        if g >= 0.0 {
            s1 = s;
            g1 = g;
            break;
        }
        s0 = s;
        g0 = g;
    }
    for _ in 0..RELIEF_REFINE {
        let s = s0 + (s1 - s0) * -g0 / (g1 - g0).max(1e-5);
        let g = s + height(at + across * s) - 1.0;
        if g >= 0.0 {
            s1 = s;
            g1 = g;
        } else {
            s0 = s;
            g0 = g;
        }
    }
    s0 + (s1 - s0) * -g0 / (g1 - g0).max(1e-5)
}

/// What [`relief_light_share`] reads of a light and the relief.
#[derive(Clone, Copy, Debug)]
pub struct LightClimb {
    /// The map's coordinates at the point, and how far down the relief it lies, 0 to 1.
    pub hit: Vec2,
    /// That depth, 0 to 1.
    pub sunk: f32,
    /// How far the coordinates move climbing the whole relief towards the light.
    pub climb: Vec2,
    /// The light's cosine on the surface.
    pub ndl: f32,
    /// The relief's depth, metres.
    pub depth: f32,
    /// The light's angular radius.
    pub tan_cone: f32,
    /// The least radius of its cone, metres.
    pub margin: f32,
}

/// `relief_light_share`: the share of a light that reaches a point of the relief, by how far the
/// ground stands inside the ray at the deepest of its fixed steps, over one width for the ray.
pub fn relief_light_share(height: &impl Fn(Vec2) -> f32, c: LightClimb, steps: u32) -> f32 {
    if c.sunk <= 0.0 || steps == 0 {
        return 1.0;
    }
    let across_ray = c.depth * (1.0 - c.ndl * c.ndl).clamp(0.0, 1.0).sqrt();
    let width = 2.0 * (c.sunk * c.depth / c.ndl * c.tan_cone).max(c.margin);
    let stride = 1.0 / steps as f32;
    let mut inside = 0.0_f32;
    for i in 0..steps {
        let climbed = c.sunk * (i as f32 + 0.5) * stride;
        inside = inside.max((c.sunk - climbed) - (1.0 - height(c.hit + c.climb * climbed)));
        if inside * across_ray >= width {
            return 0.0;
        }
    }
    1.0 - (inside * across_ray / width).clamp(0.0, 1.0)
}

/// `micro_shadow`: the share of a direct light an occlusion map's crevice keeps at the cosine `ndl`.
pub fn micro_shadow(ndl: f32, ao: f32) -> f32 {
    (ndl.abs() + 2.0 * ao * ao - 1.0).clamp(0.0, 1.0)
}

/// `bump_terminator`: the share of a direct light a mapped normal may take at its surface's
/// terminator.
pub fn bump_terminator(geometric: Vec3, shading: Vec3, light: Vec3) -> f32 {
    let ns = shading.dot(light);
    let gs = geometric.dot(shading);
    if ns <= 0.0 || gs <= 0.0 {
        return 1.0;
    }
    let g = (geometric.dot(light) / (ns * gs)).clamp(0.0, 1.0);
    g * (1.0 + g - g * g)
}

/// The gradients of a surface's u and v along it (`relief_surface`): the cotangent frame over its
/// determinant. `None` edge-on.
pub fn surface_gradients(
    n: Vec3,
    dpdx: Vec3,
    dpdy: Vec3,
    duvdx: Vec2,
    duvdy: Vec2,
) -> Option<(Vec3, Vec3)> {
    let dp1 = n.cross(dpdx);
    let dp2 = dpdy.cross(n);
    let det = dpdx.dot(dp2);
    if det.abs() < 1e-24 {
        return None;
    }
    Some((
        (dp2 * duvdx.x + dp1 * duvdy.x) / det,
        (dp2 * duvdx.y + dp1 * duvdy.y) / det,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_goes_from_whole_to_gone() {
        assert_eq!(relief_fade(0.0, 0.5), 1.0);
        assert_eq!(relief_fade(0.5, 0.5), 1.0);
        assert_eq!(relief_fade(2.5, 0.5), 0.0);
        assert!(relief_fade(1.5, 0.5) > 0.0 && relief_fade(1.5, 0.5) < 1.0);
        let mut last = 1.0;
        for k in 0..64 {
            let f = relief_fade(k as f32 * 0.1, 0.5);
            assert!(f <= last, "the fade rises at {k}");
            last = f;
        }
    }

    #[test]
    fn the_steps_follow_the_texels_and_the_angle() {
        // No more than the texels crossed.
        assert_eq!(relief_steps(1.5, 0.2, 32), 3);
        // A grazing view may take them all, a straight one few.
        assert_eq!(relief_steps(500.0, 0.0, 32), 32);
        assert_eq!(relief_steps(500.0, 1.0, 32), RELIEF_STEPS_LEAST);
    }

    #[test]
    fn the_settings_pack_their_words() {
        let s = ReliefSettings::default();
        assert_eq!(s.step_word() & 0xFF, 32);
        assert_eq!((s.step_word() >> 8) & 0xFF, 8);
        assert_eq!(s.option_bits(), OPTION_RELIEF | OPTION_AO_MIN);
        let off = ReliefSettings {
            enabled: false,
            micro_bump: true,
            ao_min: false,
            ..s
        };
        assert_eq!(off.option_bits(), OPTION_MICRO_BUMP);
    }

    #[test]
    fn a_flat_top_is_met_on_the_surface_and_a_floor_at_its_depth() {
        let flat = |_: Vec2| 1.0;
        assert_eq!(relief_march(&flat, Vec2::ZERO, Vec2::X, 16), 0.0);
        let floor = |_: Vec2| 0.0;
        assert!((relief_march(&floor, Vec2::ZERO, Vec2::X, 16) - 1.0).abs() < 1e-4);
        // A floor at half the depth.
        let half = |_: Vec2| 0.5;
        assert!((relief_march(&half, Vec2::ZERO, Vec2::X, 16) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn a_slope_is_met_where_the_closed_form_says() {
        // h(u) = 1 − k u along the view's way: g(s) = s (1 − a k) − u₀ k, so s = u₀ k / (1 − a k).
        let k = 2.0;
        let slope = |p: Vec2| (1.0 - k * p.x).clamp(0.0, 1.0);
        for (u0, a) in [(0.1, 0.1), (0.2, -0.2), (0.05, 0.3)] {
            let exact = u0 * k / (1.0 - a * k);
            let got = relief_march(&slope, Vec2::new(u0, 0.0), Vec2::new(a, 0.0), 12);
            assert!(
                (got - exact).abs() < 1e-3,
                "{u0} {a}: {got} against {exact}"
            );
        }
    }

    #[test]
    fn a_step_in_the_relief_is_not_walked_through() {
        // A pit under the view's start, its far wall rising to the surface further along it.
        let wall = |p: Vec2| if p.x < 0.5 { 0.2 } else { 1.0 };
        let at = Vec2::new(0.4, 0.0);
        let got = relief_march(&wall, at, Vec2::new(0.4, 0.0), 32);
        // The ray stands at u = 0.4 + 0.4 s, reaching the wall at s = 0.25, above the pit's floor.
        assert!((got - 0.25).abs() < 0.04, "{got}");
    }

    #[test]
    fn the_gradients_of_a_plane_are_the_uvs_per_metre() {
        // z = 0, u = 2 x, v = −y (mirrored): a pixel covers 1 cm.
        let n = Vec3::Z;
        let (dpdx, dpdy) = (Vec3::X * 0.01, Vec3::Y * 0.01);
        let (duvdx, duvdy) = (Vec2::new(0.02, 0.0), Vec2::new(0.0, -0.01));
        let (gu, gv) = surface_gradients(n, dpdx, dpdy, duvdx, duvdy).unwrap();
        assert!((gu - Vec3::X * 2.0).length() < 1e-4, "{gu}");
        assert!((gv + Vec3::Y).length() < 1e-4, "{gv}");
        // Looking down +x along a sloping ray, the view ray goes on towards +x as it sinks: u grows.
        let v = Vec3::new(-1.0, 0.0, 1.0).normalize();
        let across = -gu.dot(v) / n.dot(v);
        assert!((across - 2.0).abs() < 1e-4, "{across}");
        // Edge-on: nothing to march.
        assert!(surface_gradients(n, Vec3::ZERO, dpdy, duvdx, duvdy).is_none());
    }

    #[test]
    fn a_wall_between_the_point_and_the_light_hides_it() {
        let wall = |p: Vec2| if p.x > 0.3 { 1.0 } else { 0.0 };
        let c = LightClimb {
            hit: Vec2::new(0.1, 0.0),
            sunk: 1.0,
            climb: Vec2::new(0.5, 0.0),
            ndl: 0.6,
            depth: 0.05,
            tan_cone: 0.01,
            margin: 1e-4,
        };
        assert_eq!(relief_light_share(&wall, c, 16), 0.0);
        // Towards the other side nothing stands in the way.
        let open = LightClimb {
            climb: Vec2::new(-0.5, 0.0),
            ..c
        };
        assert_eq!(relief_light_share(&wall, open, 16), 1.0);
        // On the top, nothing can stand over it.
        let top = LightClimb { sunk: 0.0, ..c };
        assert_eq!(relief_light_share(&wall, top, 16), 1.0);
        // No steps: no self-shadow.
        assert_eq!(relief_light_share(&wall, c, 0), 1.0);
    }

    #[test]
    fn a_wider_light_softens_the_same_edge() {
        let ramp = |p: Vec2| (p.x * 4.0).clamp(0.0, 1.0);
        let c = LightClimb {
            hit: Vec2::new(0.02, 0.0),
            sunk: 0.92,
            climb: Vec2::new(0.25, 0.0),
            ndl: 0.7,
            depth: 0.02,
            tan_cone: 0.005,
            margin: 1e-5,
        };
        let sharp = relief_light_share(&ramp, c, 16);
        let wide = relief_light_share(&ramp, LightClimb { tan_cone: 0.5, ..c }, 16);
        assert!(wide > sharp, "{wide} against {sharp}");
    }

    #[test]
    fn micro_shadows_vanish_without_their_causes() {
        // No crevice: nothing, at any angle.
        for ndl in [-1.0, -0.2, 0.0, 0.3, 1.0] {
            assert_eq!(micro_shadow(ndl, 1.0), 1.0);
        }
        // A closed crevice takes no direct light from anywhere; a half-open one darkens a grazing
        // light more than one from overhead.
        assert_eq!(micro_shadow(0.0, 0.0), 0.0);
        assert_eq!(micro_shadow(1.0, 0.0), 0.0);
        assert!(micro_shadow(0.3, 0.8) < micro_shadow(0.9, 0.8));
        // Agreeing normals have no terminator of their own.
        let n = Vec3::new(0.2, 0.9, 0.1).normalize();
        let l = Vec3::new(0.5, 0.1, 0.7).normalize();
        assert_eq!(bump_terminator(n, n, l), 1.0);
        // A bump leaning into a light its surface turns from takes none of it.
        let up = Vec3::Y;
        let leaning = Vec3::new(0.9, 0.3, 0.0).normalize();
        let under = Vec3::new(1.0, -0.2, 0.0).normalize();
        assert_eq!(bump_terminator(up, leaning, under), 0.0);
    }

    /// The mock-up (`RELIEF_SHEET=/absolute/path.png cargo test -p forge-render relief_sheet`): a
    /// brick floor drawn on the CPU by the mirror above, nothing of the engine run. Rows: a grazing
    /// view, a steeper one. Columns: the normal map alone; the march; its shadow along the sun; and
    /// its occlusion's micro-shadow.
    #[test]
    fn relief_sheet() {
        let Ok(path) = std::env::var("RELIEF_SHEET") else {
            return;
        };
        let (w, h) = (480_u32, 300_u32);
        let mut sheet = image::RgbImage::new(w * 4, h * 2);
        let height = |p: Vec2| brick_height(p);
        let depth = 0.03_f32;
        let metres_per_uv = 1.2_f32;
        let sun = Vec3::new(0.75, 0.42, 0.25).normalize();
        for (row, camera) in [Vec3::new(-0.2, 0.42, 2.1), Vec3::new(-0.1, 1.3, 1.2)]
            .into_iter()
            .enumerate()
        {
            let target = Vec3::new(0.0, 0.0, 0.2);
            let forward = (target - camera).normalize();
            let right = forward.cross(Vec3::Y).normalize();
            let up = right.cross(forward);
            for column in 0..4_u32 {
                for y in 0..h {
                    for x in 0..w {
                        let ndc = Vec2::new(
                            (x as f32 + 0.5) / w as f32 * 2.0 - 1.0,
                            1.0 - (y as f32 + 0.5) / h as f32 * 2.0,
                        ) * Vec2::new(0.64, 0.40);
                        let ray = (forward + right * ndc.x + up * ndc.y).normalize();
                        let c = shade(camera, ray, column, &height, depth, metres_per_uv, sun);
                        sheet.put_pixel(column * w + x, row as u32 * h + y, c);
                    }
                }
            }
        }
        sheet.save(path).unwrap();
    }

    fn brick_height(p: Vec2) -> f32 {
        const ACROSS: f32 = 3.0;
        const ROWS: f32 = 6.0;
        const MORTAR: f32 = 0.09;
        let v = p.y * ROWS;
        let row = v.floor();
        let shift = if (row as i32).rem_euclid(2) == 0 {
            0.0
        } else {
            0.5
        };
        let x = p.x * ACROSS + shift;
        let (fx, fy) = (x - x.floor(), v - row);
        let edge = (fx.min(1.0 - fx) * 2.0 * ROWS / ACROSS * 0.5).min(fy.min(1.0 - fy));
        let t = ((edge - MORTAR * 0.4) / (MORTAR * 0.6)).clamp(0.0, 1.0);
        let inset = t * t * (3.0 - 2.0 * t);
        let grain = (p.x * 97.0).sin() * (p.y * 131.0).cos() * 0.5 + 0.5;
        (inset * 0.92 + 0.08 * grain).clamp(0.0, 1.0)
    }

    fn brick_albedo(p: Vec2, h: f32) -> Vec3 {
        let tone =
            ((p.y * 6.0).floor() * 7.3 + ((p.x * 3.0).floor() * 3.1).sin()).sin() * 0.5 + 0.5;
        let clay = Vec3::new(0.30, 0.10, 0.06).lerp(Vec3::new(0.40, 0.19, 0.11), tone);
        Vec3::new(0.40, 0.38, 0.35).lerp(clay, (h * 1.4).clamp(0.0, 1.0))
    }

    /// One pixel of a floor at y = 0, its relief `depth` metres below it. Columns as
    /// [`relief_sheet`]'s.
    fn shade(
        camera: Vec3,
        ray: Vec3,
        column: u32,
        height: &impl Fn(Vec2) -> f32,
        depth: f32,
        metres_per_uv: f32,
        sun: Vec3,
    ) -> image::Rgb<u8> {
        if ray.y >= -1e-4 {
            return image::Rgb([150, 170, 195]);
        }
        let t = -camera.y / ray.y;
        let point = camera + ray * t;
        let uv = Vec2::new(point.x, point.z) / metres_per_uv;
        let view = -ray;
        let n = Vec3::Y;
        let gu = Vec3::X / metres_per_uv;
        let gv = Vec3::Z / metres_per_uv;
        let ndv = view.dot(n).max(0.1);
        let across = -Vec2::new(gu.dot(view), gv.dot(view)) / ndv * depth;
        let texels = 1024.0_f32;
        let steps = relief_steps(across.length() * texels, ndv, 32);
        let sunk = if column == 0 {
            0.0
        } else {
            relief_march(height, uv, across, steps)
        };
        let at = uv + across * sunk;
        let h = height(at);
        // The normal from the height's slope, metres over metres.
        let e = 1.0 / texels;
        let dx = (height(at + Vec2::X * e) - height(at - Vec2::X * e)) / (2.0 * e);
        let dz = (height(at + Vec2::Y * e) - height(at - Vec2::Y * e)) / (2.0 * e);
        let slope = Vec2::new(dx, dz) * depth / metres_per_uv;
        let normal = Vec3::new(-slope.x, 1.0, -slope.y).normalize();
        let ndl = sun.dot(n);
        let climb = Vec2::new(gu.dot(sun), gv.dot(sun)) * (depth / ndl.max(0.1));
        let share = if column >= 2 {
            relief_light_share(
                height,
                LightClimb {
                    hit: at,
                    sunk: if column == 0 { 1.0 - h } else { sunk },
                    climb,
                    ndl,
                    depth,
                    tan_cone: 0.0047,
                    margin: 0.75 * metres_per_uv / texels,
                },
                8,
            )
        } else {
            1.0
        };
        let ao = (0.25 + 0.75 * h).clamp(0.0, 1.0);
        let micro = if column == 3 {
            micro_shadow(normal.dot(sun), ao)
        } else {
            1.0
        };
        let albedo = brick_albedo(at, h);
        let sky_ao = if column == 3 { ao } else { 1.0 };
        let light = normal.dot(sun).max(0.0) * share * micro * 2.4 + 0.42 * sky_ao;
        let c = albedo * light;
        let encode = |v: f32| ((v / (1.0 + v)).clamp(0.0, 1.0).sqrt() * 255.0) as u8;
        image::Rgb([encode(c.x), encode(c.y), encode(c.z)])
    }
}
