use std::collections::HashSet;

use winit::event::MouseButton;
use winit::keyboard::KeyCode;

/// Keyboard and mouse state for the current frame.
#[derive(Default, Debug)]
pub struct Input {
    keys: HashSet<KeyCode>,
    /// The mouse buttons held, and those pressed and released since the last frame.
    buttons: HashSet<MouseButton>,
    pressed: HashSet<MouseButton>,
    released: HashSet<MouseButton>,
    /// Accumulated raw mouse motion since the last frame.
    pub mouse_delta: (f32, f32),
    /// The wheel's notches since the last frame, up positive.
    pub wheel: f32,
    /// The mouse turns the view: the right button held (cursor grabbed), or the game's mouse
    /// look on (`app.mouse_look`, the cursor captured; the right button then the game's).
    pub looking: bool,
    /// The game's mouse look is on (`app.mouse_look`): the view turns with the mouse without a
    /// button held, and every button is the game's (fire, aim).
    pub mouse_look: bool,
    /// The window has lost the keyboard's focus (Alt+Tab, another window clicked): the cursor is
    /// the system's again and the mouse's motion is not the game's.
    pub(crate) unfocused: bool,
    /// The game's menu is open (#352): the cursor is the system's, the view keeps still, and the
    /// keys and buttons are the menu's until it closes.
    pub(crate) paused: bool,
}

impl Input {
    /// Whether `code` is held down.
    pub fn is_down(&self, code: KeyCode) -> bool {
        self.keys.contains(&code)
    }

    /// Whether mouse button `button` is held down.
    pub fn button_down(&self, button: MouseButton) -> bool {
        self.buttons.contains(&button)
    }

    /// Whether mouse button `button` went down since the last frame (a shot fired once per
    /// press).
    pub fn button_pressed(&self, button: MouseButton) -> bool {
        self.pressed.contains(&button)
    }

    /// Whether mouse button `button` went up since the last frame.
    pub fn button_released(&self, button: MouseButton) -> bool {
        self.released.contains(&button)
    }

    pub(crate) fn set_key(&mut self, code: KeyCode, pressed: bool) {
        if pressed {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
        }
    }

    pub(crate) fn set_button(&mut self, button: MouseButton, down: bool) {
        if down {
            if self.buttons.insert(button) {
                self.pressed.insert(button);
            }
        } else if self.buttons.remove(&button) {
            self.released.insert(button);
        }
    }

    /// Lets every key and button go: one held when the console opened must not stay down for
    /// the demo.
    pub(crate) fn release_all(&mut self) {
        self.keys.clear();
        self.released.extend(self.buttons.drain());
    }

    pub(crate) fn end_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
        self.wheel = 0.0;
        self.pressed.clear();
        self.released.clear();
    }
}

/// The name a binding gives mouse button `button` (`bind mouse1 …`, D-069): mouse1 the left,
/// mouse2 the right, mouse3 the middle, mouse4 and mouse5 the side buttons (back, forward).
pub fn button_name(button: MouseButton) -> Option<&'static str> {
    Some(match button {
        MouseButton::Left => "mouse1",
        MouseButton::Right => "mouse2",
        MouseButton::Middle => "mouse3",
        MouseButton::Back => "mouse4",
        MouseButton::Forward => "mouse5",
        MouseButton::Other(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_button_is_pressed_for_one_frame_held_until_released() {
        let mut input = Input::default();
        input.set_button(MouseButton::Left, true);
        assert!(input.button_pressed(MouseButton::Left) && input.button_down(MouseButton::Left));
        // A second down event (a repeat) is not a second press.
        input.end_frame();
        input.set_button(MouseButton::Left, true);
        assert!(!input.button_pressed(MouseButton::Left) && input.button_down(MouseButton::Left));
        input.set_button(MouseButton::Left, false);
        assert!(input.button_released(MouseButton::Left) && !input.button_down(MouseButton::Left));
        input.end_frame();
        assert!(!input.button_released(MouseButton::Left));
        // Released by the console's opening, as keys are.
        input.set_button(MouseButton::Right, true);
        input.release_all();
        assert!(
            !input.button_down(MouseButton::Right) && input.button_released(MouseButton::Right)
        );
        assert_eq!(button_name(MouseButton::Left), Some("mouse1"));
        assert_eq!(button_name(MouseButton::Right), Some("mouse2"));
    }
}
