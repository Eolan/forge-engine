use glam::{Mat4, Quat, Vec3};
use winit::keyboard::KeyCode;

use crate::input::Input;

/// A free-flying camera: WASD/QE move, Shift fast, the wheel sets the speed, right mouse drag to
/// look.
///
/// Right-handed, +Y up, −Z forward; reversed-Z infinite projection with Vulkan depth range.
#[derive(Clone, Debug)]
pub struct FlyCamera {
    /// World position (metres).
    pub position: Vec3,
    /// Yaw around +Y (radians).
    pub yaw: f32,
    /// Pitch around +X (radians), clamped.
    pub pitch: f32,
    /// Vertical field of view (radians).
    pub fov_y: f32,
    /// Near plane (metres).
    pub near: f32,
    /// Base speed (m/s), a quarter more or less a notch of the wheel; Shift multiplies by 5,
    /// Ctrl by 25.
    pub speed: f32,
    /// Radians per pixel of mouse motion.
    pub sensitivity: f32,
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 2.0, 10.0),
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 70_f32.to_radians(),
            near: 0.05,
            speed: 12.0,
            sensitivity: 0.0025,
        }
    }
}

impl FlyCamera {
    /// Orientation.
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }

    /// World → view.
    pub fn view(&self) -> Mat4 {
        Mat4::from_rotation_translation(self.rotation(), self.position).inverse()
    }

    /// Camera-relative → view: the view of this camera standing at the origin, its rotation
    /// alone. The renderer draws relative to the camera (D-004, issue #93): positions reach the
    /// GPU as integer cells and `f32` offsets, and the shaders subtract the camera's before
    /// this matrix applies, so no world-sized number meets an `f32` matrix.
    pub fn view_rotation(&self) -> Mat4 {
        Mat4::from_quat(self.rotation()).transpose()
    }

    /// View → clip, reversed-Z with an infinite far plane.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect, self.near)
    }

    /// Forward direction.
    pub fn forward(&self) -> Vec3 {
        self.rotation() * Vec3::NEG_Z
    }

    /// Applies one frame of input.
    pub fn update(&mut self, input: &Input, dt: f32) {
        if input.looking {
            self.yaw -= input.mouse_delta.0 * self.sensitivity;
            self.pitch = (self.pitch - input.mouse_delta.1 * self.sensitivity).clamp(-1.5, 1.5);
        }
        let rot = self.rotation();
        let forward = rot * Vec3::NEG_Z;
        let right = rot * Vec3::X;
        let mut motion = Vec3::ZERO;
        if input.is_down(KeyCode::KeyW) {
            motion += forward;
        }
        if input.is_down(KeyCode::KeyS) {
            motion -= forward;
        }
        if input.is_down(KeyCode::KeyD) {
            motion += right;
        }
        if input.is_down(KeyCode::KeyA) {
            motion -= right;
        }
        if input.is_down(KeyCode::KeyE) {
            motion += Vec3::Y;
        }
        if input.is_down(KeyCode::KeyQ) {
            motion -= Vec3::Y;
        }
        // The wheel sets the speed for good, a quarter faster or slower a notch (from a crawl to
        // look a doorway over to a flight across a planet's ground); Shift for five times the
        // speed, Ctrl for twenty-five, both for both.
        if input.wheel != 0.0 {
            self.speed = (self.speed * 1.25_f32.powf(input.wheel)).clamp(0.25, 400.0);
        }
        let mut speed = self.speed;
        if input.is_down(KeyCode::ShiftLeft) {
            speed *= 5.0;
        }
        if input.is_down(KeyCode::ControlLeft) {
            speed *= 25.0;
        }
        self.position += motion.normalize_or_zero() * speed * dt;
    }
}

/// A third-person camera's arm (#315): the camera kept [`Self::length`] back from a pivot (the
/// character's head) along a direction, pulled in at once to the first thing between them with a
/// sphere of [`Self::radius`] round the camera kept clear, and let out again smoothly once it
/// clears, so it never stands behind a trunk or inside a wall.
///
/// The sweep is five rays from the pivot: the arm's own and four aimed a radius round its end
/// across it, near enough a sphere's for a camera; the caller casts them against what the camera
/// must keep out of (the physics world's rays, or the ray-traced scene's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringArm {
    /// How far back the camera stands when nothing is in the way, metres.
    pub length: f32,
    /// The clearance kept round the camera, metres: more than the near plane's reach.
    pub radius: f32,
    /// How fast the arm lets out again once its way clears, per second: it closes this share
    /// of what is left of the way each second's `1 − e^(−rate)` (4: 98 % in a second).
    pub return_rate: f32,
    /// How far back the camera stands now, metres.
    reach: f32,
}

impl SpringArm {
    /// An arm `length` metres long keeping `radius` clear, let out at a rate of 4 a second.
    pub fn new(length: f32, radius: f32) -> Self {
        Self {
            length,
            radius,
            return_rate: 4.0,
            reach: length,
        }
    }

    /// How far back the camera stands now, metres.
    pub fn reach(&self) -> f32 {
        self.reach
    }

    /// The camera's place this frame: from `pivot`, back along `back` (towards the camera, any
    /// length but zero) as far as the way is clear, after `dt` seconds. `ray(origin, direction,
    /// max)` gives the distance along the unit `direction` to the first hit within `max`
    /// metres, or none.
    pub fn update(
        &mut self,
        pivot: Vec3,
        back: Vec3,
        dt: f32,
        ray: impl Fn(Vec3, Vec3, f32) -> Option<f32>,
    ) -> Vec3 {
        let back = back.normalize_or(Vec3::Z);
        let (u, v) = back.any_orthonormal_pair();
        let end = back * (self.length + self.radius);
        let mut clear = self.length;
        for across in [Vec3::ZERO, u, -u, v, -v] {
            let to = end + across * self.radius;
            let reach = to.length();
            let direction = to / reach;
            if let Some(hit) = ray(pivot, direction, reach) {
                // Back along the arm, less the clearance.
                clear = clear.min((hit * direction.dot(back) - self.radius).max(0.0));
            }
        }
        if clear < self.reach {
            // In at once: the camera never passes through what is in the way.
            self.reach = clear;
        } else {
            let share = 1.0 - (-self.return_rate * dt.max(0.0)).exp();
            self.reach += (clear - self.reach) * share;
        }
        pivot + back * self.reach
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall across +z at `at` metres: the distance along `direction` from `origin` to it.
    fn wall(at: f32) -> impl Fn(Vec3, Vec3, f32) -> Option<f32> {
        move |origin: Vec3, direction: Vec3, max: f32| {
            if direction.z <= 1e-6 {
                return None;
            }
            let t = (at - origin.z) / direction.z;
            (t >= 0.0 && t <= max).then_some(t)
        }
    }

    #[test]
    fn the_arm_pulls_in_at_once_and_lets_out_smoothly() {
        let mut arm = SpringArm::new(4.0, 0.3);
        let pivot = Vec3::new(0.0, 1.7, 0.0);
        // A wall 2 m behind: the camera at once 0.3 m short of it.
        let camera = arm.update(pivot, Vec3::Z, 1.0 / 60.0, wall(2.0));
        assert!((camera.z - 1.7).abs() < 1e-4, "{camera}");
        // The wall gone: out again, not at once, and nearly all the way after a second.
        let camera = arm.update(pivot, Vec3::Z, 1.0 / 60.0, |_, _, _| None);
        assert!(camera.z > 1.7 && camera.z < 2.0, "{camera}");
        for _ in 0..59 {
            arm.update(pivot, Vec3::Z, 1.0 / 60.0, |_, _, _| None);
        }
        assert!(arm.reach() > 3.9 && arm.reach() <= 4.0, "{}", arm.reach());
        // Nothing in the way from the start: the whole length.
        let mut free = SpringArm::new(4.0, 0.3);
        let camera = free.update(pivot, Vec3::new(0.0, 0.0, 2.0), 0.0, |_, _, _| None);
        assert!(
            camera.abs_diff_eq(Vec3::new(0.0, 1.7, 4.0), 1e-5),
            "{camera}"
        );
    }
}
