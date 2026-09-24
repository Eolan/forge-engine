//! The visibility buffer's attribute reconstruction, mirrored from `shaders/meshlet.slang`
//! (`barycentrics`) so the derivation is tested on the CPU and documented in one place.
//!
//! Every drawn pixel holds `visible_slot << 7 | triangle`: the slot indexes the frame's
//! visible-cluster list (instance, cluster), the triangle the cluster's primitive. Shading
//! runs once per pixel in compute: it fetches the three vertices, projects them with the
//! frame's (jittered) projection and reconstructs the perspective-correct barycentrics of the
//! pixel centre analytically, with their screen-space derivatives (Schied & Dachsbacher
//! 2015; the form Hable 2021 uses). No `ddx`/`ddy` is involved, so shading needs no quads and
//! no helper lanes, and derivatives stay exact at triangle edges.
//!
//! The derivation: for a triangle with clip-space vertices `c_i = (x_i, y_i, z_i, w_i)`, the
//! screen-linear barycentrics `b_i(p)` of a point `p` in NDC are affine, and so are
//! `b_i(p) / w_i`. Their sum is `1 / w(p)`; the perspective-correct coordinates are
//! `lambda_i = w(p) · b_i(p) / w_i`. With `b(n_0) = (1, 0, 0)` at vertex 0 and the gradients
//! of `b_i` from the NDC edge vectors, the whole thing is a few multiply-adds per pixel; the
//! derivative of `lambda_i` along `x` is `w · (∂(b_i/w_i)/∂x − lambda_i · ∂(1/w)/∂x)`.

use glam::{Vec2, Vec3, Vec4};

/// Bits of the triangle index inside a visibility id (clusters have at most 124 triangles).
pub const TRIANGLE_BITS: u32 = 7;
/// A pixel nothing was drawn to.
pub const EMPTY: u32 = u32::MAX;

/// Perspective-correct barycentrics of a pixel and their change per pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Barycentrics {
    /// The coordinates at the pixel centre; they sum to one inside the triangle.
    pub lambda: Vec3,
    /// Change of `lambda` one pixel to the right.
    pub ddx: Vec3,
    /// Change of `lambda` one pixel down.
    pub ddy: Vec3,
}

/// `barycentrics` of `shaders/meshlet.slang`: `c0..c2` are the clip-space vertices, `ndc` the
/// pixel centre (x right, y up) and `size` the target in pixels.
pub fn barycentrics(c0: Vec4, c1: Vec4, c2: Vec4, ndc: Vec2, size: Vec2) -> Barycentrics {
    let inv_w = Vec3::new(1.0 / c0.w, 1.0 / c1.w, 1.0 / c2.w);
    let n0 = c0.truncate().truncate() * inv_w.x;
    let n1 = c1.truncate().truncate() * inv_w.y;
    let n2 = c2.truncate().truncate() * inv_w.z;
    let (e0, e1) = (n2 - n1, n0 - n1);
    let inv_det = 1.0 / (e0.x * e1.y - e0.y * e1.x);
    let dbdx = Vec3::new(n1.y - n2.y, n2.y - n0.y, n0.y - n1.y) * inv_det * inv_w;
    let dbdy = Vec3::new(n2.x - n1.x, n0.x - n2.x, n1.x - n0.x) * inv_det * inv_w;
    let ddx_sum = dbdx.x + dbdx.y + dbdx.z;
    let ddy_sum = dbdy.x + dbdy.y + dbdy.z;
    let delta = ndc - n0;
    let interp_inv_w = inv_w.x + delta.x * ddx_sum + delta.y * ddy_sum;
    let interp_w = 1.0 / interp_inv_w;
    let lambda = Vec3::new(
        interp_w * (inv_w.x + delta.x * dbdx.x + delta.y * dbdy.x),
        interp_w * (delta.x * dbdx.y + delta.y * dbdy.y),
        interp_w * (delta.x * dbdx.z + delta.y * dbdy.z),
    );
    Barycentrics {
        lambda,
        ddx: interp_w * (dbdx - ddx_sum * lambda) * (2.0 / size.x),
        ddy: interp_w * (dbdy - ddy_sum * lambda) * (-2.0 / size.y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    fn clip(view_proj: Mat4, p: Vec3) -> Vec4 {
        view_proj * p.extend(1.0)
    }

    #[test]
    fn the_barycentrics_of_a_projected_point_are_recovered() {
        let projection =
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.1, 16.0 / 9.0, 0.05);
        let view =
            glam::camera::rh::view::look_at_mat4(Vec3::new(0.5, 1.0, 3.0), Vec3::ZERO, Vec3::Y);
        let view_proj = projection * view;
        let tri = [
            Vec3::new(-1.0, -0.5, -2.0),
            Vec3::new(1.2, -0.3, -4.0),
            Vec3::new(0.1, 1.4, -3.0),
        ];
        let c = [
            clip(view_proj, tri[0]),
            clip(view_proj, tri[1]),
            clip(view_proj, tri[2]),
        ];
        let size = Vec2::new(1600.0, 900.0);
        for &(a, b) in &[(0.2_f32, 0.3_f32), (0.6, 0.1), (0.05, 0.9), (1.0, 0.0)] {
            let want = Vec3::new(a, b, 1.0 - a - b);
            // The world point with those barycentrics, projected: its NDC is the pixel.
            let world = tri[0] * want.x + tri[1] * want.y + tri[2] * want.z;
            let p = clip(view_proj, world);
            let ndc = Vec2::new(p.x / p.w, p.y / p.w);
            let got = barycentrics(c[0], c[1], c[2], ndc, size);
            assert!(
                got.lambda.abs_diff_eq(want, 2e-5),
                "{want} vs {}",
                got.lambda
            );
        }
    }

    #[test]
    fn the_derivatives_match_finite_differences() {
        let projection =
            glam::camera::rh::proj::directx::perspective_infinite_reverse(1.1, 16.0 / 9.0, 0.05);
        let view_proj = projection;
        let c = [
            clip(view_proj, Vec3::new(-1.0, -0.5, -2.0)),
            clip(view_proj, Vec3::new(1.2, -0.3, -4.0)),
            clip(view_proj, Vec3::new(0.1, 1.4, -3.0)),
        ];
        let size = Vec2::new(1600.0, 900.0);
        let ndc = Vec2::new(0.05, 0.1);
        let at = |n: Vec2| barycentrics(c[0], c[1], c[2], n, size).lambda;
        let got = barycentrics(c[0], c[1], c[2], ndc, size);
        let step_x = Vec2::new(2.0 / size.x, 0.0);
        let step_y = Vec2::new(0.0, -2.0 / size.y);
        let fd_x = at(ndc + step_x) - at(ndc);
        let fd_y = at(ndc + step_y) - at(ndc);
        assert!(got.ddx.abs_diff_eq(fd_x, 1e-5), "{} vs {fd_x}", got.ddx);
        assert!(got.ddy.abs_diff_eq(fd_y, 1e-5), "{} vs {fd_y}", got.ddy);
        assert!(
            (got.ddx.x + got.ddx.y + got.ddx.z).abs() < 1e-6,
            "the sum stays one"
        );
    }
}
