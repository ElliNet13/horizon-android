//! Touch input on phones: connects the touch layer (`horizon_client::touch`'s on-screen controls and
//! `crate::touch_ui`'s menu gestures) to the game's input paths.
//!
//! The wiring is deliberately thin, so it stays easy to carry over upstream changes:
//! - in the world a finger drives [`TouchControls`], whose [`PadState`] is merged with the pad's in
//!   `poll_pads`, and whose look drag becomes mouse motion;
//! - on a screen a finger becomes the mouse events the agent control already synthesizes
//!   (`CursorMoved`, `MouseInput`, `MouseWheel`), so every screen works unchanged.
//!
//! Active on Android, or anywhere with `HORIZON_TOUCH` set (so a desktop touchscreen can test it).

use super::event_loop::EventLoopCtl;
use super::{Game, Screen};
use crate::touch_ui::{Action, Gesture, TouchUi};
use horizon_client::gamepad::PadState;
use horizon_client::hud::HudBatch;
use horizon_client_render::touch::TouchControls;
use winit::dpi::PhysicalPosition;
use winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
};
use winit::keyboard::KeyCode;

/// Degrees one pixel of mouse motion turns the camera at vanilla's default sensitivity.
const DEGREES_PER_PIXEL: f32 = 0.15;
/// The saved on-screen layout, in the game folder.
const LAYOUT_FILE: &str = "horizon-touch.json";

/// Whether this run takes touch input (and draws the touch layer).
pub(in crate::game) fn touch_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED
        .get_or_init(|| cfg!(target_os = "android") || std::env::var_os("HORIZON_TOUCH").is_some())
}

/// The touch layer's state, kept by the game.
#[derive(Default)]
pub(in crate::game) struct TouchInput {
    controls: TouchControls,
    ui: TouchUi,
    loaded: bool,
}

impl Game<'_> {
    /// Android's Back button is the Escape key: closes a screen, or opens the pause menu.
    /// True when `event` was that button (pressed or released), which nothing else should see.
    pub(in crate::game) fn android_back(&mut self, event: &WindowEvent) -> bool {
        #[cfg(target_os = "android")]
        if let WindowEvent::KeyboardInput { event: key, .. } = event
            && key.logical_key == winit::keyboard::Key::Named(winit::keyboard::NamedKey::GoBack)
        {
            if key.state == ElementState::Pressed && !key.repeat {
                let _ = self.key_pressed(KeyCode::Escape, KeyCode::Escape);
            }
            return true;
        }
        let _ = event;
        false
    }

    fn touch_in_world(&self) -> bool {
        self.screen == Screen::Playing && self.title.is_none() && !self.dialog_open()
    }

    fn touch_surface(&self) -> Option<[u32; 2]> {
        let size = self.window.as_ref()?.inner_size();
        (size.width > 0 && size.height > 0).then_some([size.width, size.height])
    }

    /// One finger event from the window.
    pub(in crate::game) fn touch_event(&mut self, event_loop: &dyn EventLoopCtl, touch: &Touch) {
        let Some([w, h]) = self.touch_surface() else {
            return;
        };
        if !self.touch.loaded {
            self.touch.loaded = true;
            let path = Self::game_dir().join(LAYOUT_FILE);
            self.touch.controls.settings = horizon_client_render::touch::Settings::load(&path);
        }
        self.touch.controls.viewport(w, h);
        let id = touch.id;
        let point = [touch.location.x as f32, touch.location.y as f32];
        if self.touch_in_world() {
            self.touch_world(id, point, touch.phase, [w, h]);
        } else {
            // A finger that began in the world must not stay held on a screen.
            self.touch.controls.clear();
            self.touch_screen(event_loop, id, point, touch.phase, [w, h]);
        }
    }

    fn touch_world(&mut self, id: u64, point: [f32; 2], phase: TouchPhase, [w, h]: [u32; 2]) {
        let started = phase == TouchPhase::Started;
        let ended = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        let norm = [point[0] / w as f32, point[1] / h as f32];
        let delta = self.touch.controls.update(id, norm, started, ended);
        if delta != [0.0; 2] {
            let [dx, dy] = horizon_client_render::touch::aim::camera_delta(
                delta,
                w,
                h,
                &self.touch.controls.settings,
            );
            self.mouse.add_motion(
                f64::from(dx / DEGREES_PER_PIXEL),
                f64::from(dy / DEGREES_PER_PIXEL),
            );
        }
        let c = &mut self.touch.controls;
        let chat = std::mem::take(&mut c.chat_requested);
        let debug = std::mem::take(&mut c.debug_requested);
        // Not wired yet: the layout editor and the perspective toggle.
        c.settings_requested = false;
        c.perspective_requested = false;
        if chat {
            self.open_chat_method("");
        }
        if debug {
            self.toggle_debug_options();
        }
    }

    fn touch_screen(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        id: u64,
        point: [f32; 2],
        phase: TouchPhase,
        [w, h]: [u32; 2],
    ) {
        let scale = HudBatch::new([w, h], self.settings.gui_scale_inputs()).scale;
        let gui = [point[0] / scale, point[1] / scale];
        let device_id = DeviceId::dummy();
        let at = PhysicalPosition::new(f64::from(point[0]), f64::from(point[1]));
        if phase == TouchPhase::Started {
            let (gw, gh) = (w as f32 / scale, h as f32 / scale);
            match self.touch.ui.action(gw, gh, gui, true) {
                Some(Action::Key(key)) => {
                    let _ = self.key_pressed(key, key);
                    return;
                }
                Some(Action::VirtualMouse) => {
                    self.touch.ui.virtual_mouse = !self.touch.ui.virtual_mouse;
                    return;
                }
                // The auxiliary keys, on-screen keyboard and mouse buttons are not wired yet.
                Some(_) => return,
                None => {}
            }
        }
        // A finger on a slider drives the pointer like a held mouse button.
        if self.touch.ui.direct_pointer(id, gui, phase) {
            self.touch_mouse(event_loop, device_id, at, Some(phase));
            return;
        }
        match self.touch.ui.gesture(id, point, phase) {
            Gesture::Tap => {
                self.touch_mouse(event_loop, device_id, at, Some(TouchPhase::Started));
                self.touch_mouse(event_loop, device_id, at, Some(TouchPhase::Ended));
            }
            Gesture::Scroll(lines) => {
                self.touch_mouse(event_loop, device_id, at, None);
                self.handle_window_event_inner(
                    event_loop,
                    WindowEvent::MouseWheel {
                        device_id,
                        delta: MouseScrollDelta::LineDelta(0.0, -lines),
                        phase: TouchPhase::Moved,
                    },
                );
            }
            Gesture::None => {}
        }
    }

    /// The cursor at `at`, then the left button for a finger that lands or lifts.
    fn touch_mouse(
        &mut self,
        event_loop: &dyn EventLoopCtl,
        device_id: DeviceId,
        at: PhysicalPosition<f64>,
        phase: Option<TouchPhase>,
    ) {
        self.handle_window_event_inner(
            event_loop,
            WindowEvent::CursorMoved {
                device_id,
                position: at,
            },
        );
        let state = match phase {
            Some(TouchPhase::Started) => ElementState::Pressed,
            Some(TouchPhase::Ended | TouchPhase::Cancelled) => ElementState::Released,
            _ => return,
        };
        self.handle_window_event_inner(
            event_loop,
            WindowEvent::MouseInput {
                device_id,
                state,
                button: MouseButton::Left,
            },
        );
    }

    /// The touch controls as a pad's state, for `poll_pads` to merge.
    pub(in crate::game) fn touch_pad_state(&mut self) -> PadState {
        if !touch_enabled() || !self.touch_in_world() {
            return PadState::default();
        }
        self.touch.controls.poll_state()
    }

    /// Draws the touch layer over everything else.
    pub(in crate::game) fn draw_touch(&mut self, batch: &mut HudBatch) {
        if !touch_enabled() {
            return;
        }
        if self.touch_in_world() {
            self.touch.controls.draw(batch, &self.gui.atlas);
        } else {
            self.touch.ui.draw(batch, &self.gui.atlas, true);
        }
    }
}

/// Back, for screens: the Escape key.
#[allow(dead_code)]
const BACK: KeyCode = KeyCode::Escape;
