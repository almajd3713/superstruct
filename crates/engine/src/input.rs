//! Keyboard and mouse state, with an action-binding table so game code reads actions, not keys.

use std::collections::HashSet;

use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    MoveForward,
    MoveBack,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Sprint,
    ToggleFullscreen,
    CycleVsync,
    ReleaseCursor,
    Screenshot,
    ToggleOverlay,
    ToggleDebugLines,
    ToggleFly,
    FlyFaster,
    FlySlower,
    TeleportPort,
    FreezeCulling,
    TeleportSanctuary,
    /// Calls a lift at an opening.
    Interact,
    /// Stylization prototypes (P4.3.3).
    TogglePosterize,
    ToggleDither,
    ToggleOutlines,
    ToggleGrain,
    /// Altimeter and coordinates readout (P5.4.1).
    ToggleReadout,
    /// Drops a beacon at the player's feet (P5.4.2).
    DropBeacon,
    /// Photo mode (P5.4.3).
    TogglePhoto,
    FovWider,
    FovNarrower,
    ExposureUp,
    ExposureDown,
}

/// A physical key or mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Input {
    Key(KeyCode),
    Mouse(MouseButton),
}

#[derive(Debug, Clone)]
pub struct Bindings {
    table: Vec<(Input, Action)>,
}

impl Default for Bindings {
    fn default() -> Self {
        use Action as A;
        use KeyCode as K;
        let keys = [
            (K::KeyW, A::MoveForward),
            (K::KeyS, A::MoveBack),
            (K::KeyA, A::MoveLeft),
            (K::KeyD, A::MoveRight),
            (K::Space, A::MoveUp),
            (K::ControlLeft, A::MoveDown),
            (K::ShiftLeft, A::Sprint),
            (K::F11, A::ToggleFullscreen),
            (K::F2, A::CycleVsync),
            (K::Escape, A::ReleaseCursor),
            (K::F12, A::Screenshot),
            (K::F1, A::ToggleOverlay),
            (K::F3, A::ToggleDebugLines),
            (K::F4, A::ToggleFly),
            (K::Equal, A::FlyFaster),
            (K::Minus, A::FlySlower),
            (K::F5, A::TeleportPort),
            (K::F6, A::TeleportSanctuary),
            (K::F7, A::FreezeCulling),
            (K::KeyE, A::Interact),
            (K::F8, A::TogglePosterize),
            (K::F9, A::ToggleDither),
            (K::F10, A::ToggleOutlines),
            (K::KeyG, A::ToggleGrain),
            (K::Tab, A::ToggleReadout),
            (K::KeyB, A::DropBeacon),
            (K::KeyP, A::TogglePhoto),
            (K::PageUp, A::FovWider),
            (K::PageDown, A::FovNarrower),
            (K::Period, A::ExposureUp),
            (K::Comma, A::ExposureDown),
        ];
        Self {
            table: keys.into_iter().map(|(k, a)| (Input::Key(k), a)).collect(),
        }
    }
}

impl Bindings {
    pub fn rebind(&mut self, action: Action, input: Input) {
        self.table.retain(|(_, a)| *a != action);
        self.table.push((input, action));
    }

    fn actions_for(&self, input: Input) -> impl Iterator<Item = Action> + '_ {
        self.table
            .iter()
            .filter(move |(i, _)| *i == input)
            .map(|(_, a)| *a)
    }
}

#[derive(Debug, Default)]
pub struct InputState {
    bindings: Bindings,
    held: HashSet<Action>,
    pressed: HashSet<Action>,
    /// Raw mouse counts since the last [`InputState::end_frame`]; no acceleration or smoothing.
    mouse_delta: (f64, f64),
}

impl InputState {
    pub fn new(bindings: Bindings) -> Self {
        Self {
            bindings,
            ..Self::default()
        }
    }

    pub fn on_key(&mut self, code: KeyCode, state: ElementState, repeat: bool) {
        self.on_input(Input::Key(code), state, repeat);
    }

    pub fn on_mouse_button(&mut self, button: MouseButton, state: ElementState) {
        self.on_input(Input::Mouse(button), state, false);
    }

    fn on_input(&mut self, input: Input, state: ElementState, repeat: bool) {
        let actions: Vec<Action> = self.bindings.actions_for(input).collect();
        for action in actions {
            if state.is_pressed() {
                if !repeat && self.held.insert(action) {
                    self.pressed.insert(action);
                }
            } else {
                self.held.remove(&action);
            }
        }
    }

    /// Raw `DeviceEvent::MouseMotion` delta.
    pub fn on_mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }

    /// Drops everything held, for focus loss: key-up events are not delivered while unfocused.
    pub fn release_all(&mut self) {
        self.held.clear();
    }

    pub fn held(&self, action: Action) -> bool {
        self.held.contains(&action)
    }

    /// True for the single frame in which the action went down.
    pub fn pressed(&self, action: Action) -> bool {
        self.pressed.contains(&action)
    }

    pub fn mouse_delta(&self) -> (f64, f64) {
        self.mouse_delta
    }

    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.mouse_delta = (0.0, 0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_not_keys() {
        let mut input = InputState::new(Bindings::default());
        input.on_key(KeyCode::KeyW, ElementState::Pressed, false);
        assert!(input.held(Action::MoveForward) && input.pressed(Action::MoveForward));
        input.end_frame();
        assert!(input.held(Action::MoveForward) && !input.pressed(Action::MoveForward));
        input.on_key(KeyCode::KeyW, ElementState::Released, false);
        assert!(!input.held(Action::MoveForward));
    }

    #[test]
    fn rebinding_moves_the_action() {
        let mut bindings = Bindings::default();
        bindings.rebind(Action::MoveForward, Input::Key(KeyCode::ArrowUp));
        let mut input = InputState::new(bindings);
        input.on_key(KeyCode::KeyW, ElementState::Pressed, false);
        assert!(!input.held(Action::MoveForward));
        input.on_key(KeyCode::ArrowUp, ElementState::Pressed, false);
        assert!(input.held(Action::MoveForward));
    }

    #[test]
    fn mouse_delta_accumulates_until_frame_end() {
        let mut input = InputState::default();
        input.on_mouse_motion(1.0, 2.0);
        input.on_mouse_motion(3.0, -1.0);
        assert_eq!(input.mouse_delta(), (4.0, 1.0));
        input.end_frame();
        assert_eq!(input.mouse_delta(), (0.0, 0.0));
    }
}
