use std::collections::HashSet;

use winit::keyboard::KeyCode;

/// Keyboard and mouse state for the current frame.
#[derive(Default, Debug)]
pub struct Input {
    keys: HashSet<KeyCode>,
    /// Accumulated raw mouse motion since the last frame.
    pub mouse_delta: (f32, f32),
    /// Right mouse button held (cursor grabbed).
    pub looking: bool,
}

impl Input {
    /// Whether `code` is held down.
    pub fn is_down(&self, code: KeyCode) -> bool {
        self.keys.contains(&code)
    }

    pub(crate) fn set_key(&mut self, code: KeyCode, pressed: bool) {
        if pressed {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
        }
    }

    pub(crate) fn end_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
    }
}
