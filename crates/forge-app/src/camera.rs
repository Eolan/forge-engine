use glam::{Mat4, Quat, Vec3};
use winit::keyboard::KeyCode;

use crate::input::Input;

/// A free-flying camera: WASD/QE move, Shift fast, right mouse drag to look.
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
    /// Base speed (m/s); Shift multiplies by 5.
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
        let speed = if input.is_down(KeyCode::ShiftLeft) {
            self.speed * 5.0
        } else {
            self.speed
        };
        self.position += motion.normalize_or_zero() * speed * dt;
    }
}
