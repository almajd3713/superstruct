//! Window helpers.

use winit::window::{Fullscreen, Window};

/// Borderless fullscreen on the window's current monitor, or back to windowed.
pub fn toggle_fullscreen(window: &Window) {
    let next = match window.fullscreen() {
        Some(_) => None,
        None => Some(Fullscreen::Borderless(None)),
    };
    window.set_fullscreen(next);
}
