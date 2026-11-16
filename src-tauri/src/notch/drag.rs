//! Moving the notch. The UI reports the grip press; from then on this thread
//! follows the cursor (it is only read while dragging), keeps the notch on the
//! nearest edge of the monitor under the pointer, and springs it into place on
//! release, in step with the display's refresh.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};

use super::{dock, limits_for, monitors, perform, placement_of, plan, window, Notch, LABEL};
use crate::config::{Config, Motion, Screen};
use crate::spring::{self, Spring};
use crate::state::Core;
use crate::{notch, settings};

/// How far (logical px) another edge must be nearer before the notch hops to it.
const EDGE_STICKY: f64 = 24.0;
/// Released within this distance (logical px) of the middle: snap to the middle.
const CENTRE_SNAP: f64 = 24.0;
/// How far past the end of an edge the notch may be pulled.
const RUBBER: f64 = 14.0;

fn left_button_down() -> bool {
    unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 }
}

/// Waits for the next screen refresh (falls back to a short sleep without DWM).
fn next_frame(refresh_hz: u32) {
    if unsafe { DwmFlush() }.is_err() {
        std::thread::sleep(Duration::from_secs_f64(1.0 / refresh_hz.max(30) as f64));
    }
}

/// True when motion should be reduced: the setting, or Windows' "Animation effects" being off.
pub fn reduced_motion(config: &Config) -> bool {
    match config.appearance.motion {
        Motion::Reduced => true,
        Motion::Full => false,
        Motion::System => {
            let mut animations = windows::core::BOOL(1);
            let asked = unsafe {
                SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut animations as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
            };
            asked.is_ok() && !animations.as_bool()
        }
    }
}

/// Springs the dock point to `target` and leaves it there.
fn glide(app: &AppHandle, target: f64, tune: spring::Tune) {
    let notch_state = app.state::<Notch>();
    let (mut s, hz) = {
        let guard = notch_state.0.lock().unwrap();
        let Some(state) = guard.as_ref() else { return };
        (Spring::at(state.point, tune), state.monitor.refresh_hz)
    };
    s.target = target;
    let started = Instant::now();
    let mut last = Instant::now();
    while !s.settled(0.4) && started.elapsed() < Duration::from_millis(1500) {
        next_frame(hz);
        let dt = last.elapsed().as_secs_f64().min(1.0 / 30.0);
        last = Instant::now();
        s.step(dt);
        let mut guard = notch_state.0.lock().unwrap();
        let (Some(state), Some(window)) = (guard.as_mut(), window(app)) else { return };
        state.point = s.x;
        let planned = plan(state);
        drop(guard);
        perform(&window, planned);
    }
    let planned = notch_state.0.lock().unwrap().as_mut().and_then(|state| {
        state.point = target;
        plan(state)
    });
    if let Some(window) = window(app) {
        perform(&window, planned);
    }
}

/// Writes the notch's monitor, edge and position into the settings.
fn save_position(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    let notch_state = app.state::<Notch>();
    let old = core.config();
    let mut new = old.clone();
    {
        let guard = notch_state.0.lock().unwrap();
        let Some(state) = guard.as_ref() else { return };
        let (start, len) = dock::along_range(state.monitor.work, state.edge);
        new.notch.edge = state.edge;
        new.notch.position.set(state.edge, dock::to_fraction(state.point, start, len));
        // Dropped on another monitor than the settings point to: remember that monitor.
        let expected = notch::resolve_monitor(&old).map(|m| m.id);
        if expected.as_deref() != Some(state.monitor.id.as_str()) {
            new.notch.screen = Screen::Monitor;
            new.notch.monitor_id = state.monitor.id.clone();
        }
    }
    let _ = settings::commit(app, old, new);
}

/// Starts following the cursor until the left button is released.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || follow(&app));
}

fn follow(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    let config = core.config();
    let notch_state = app.state::<Notch>();
    if config.notch.locked {
        return;
    }
    let reduced = reduced_motion(&config);

    // Where on the notch the pointer grabbed it, along the edge.
    let (mut follow, mut grab, hz) = {
        let mut guard = notch_state.0.lock().unwrap();
        let Some(state) = guard.as_mut() else { return };
        if state.dragging {
            return;
        }
        state.dragging = true;
        let (x, y) = monitors::cursor();
        let along = if state.edge.is_vertical() { y } else { x } as f64;
        (Spring::at(state.point, if reduced { spring::REDUCED } else { spring::DRAG }), along - state.point, state.monitor.refresh_hz)
    };

    let mut last = Instant::now();
    while left_button_down() {
        next_frame(hz);
        let dt = last.elapsed().as_secs_f64().min(1.0 / 30.0);
        last = Instant::now();
        let (x, y) = monitors::cursor();
        let Some(monitor) = monitors::at_point(x, y) else { continue };

        let mut guard = notch_state.0.lock().unwrap();
        let (Some(state), Some(window)) = (guard.as_mut(), window(app)) else { return };
        let sticky = (EDGE_STICKY * monitor.scale) as i32;
        let same_monitor = monitor.id == state.monitor.id;
        let edge = dock::nearest_edge(monitor.work, x, y, state.edge, if same_monitor { sticky } else { 0 });
        let along = if edge.is_vertical() { y } else { x } as f64;

        if !same_monitor || edge != state.edge {
            // Hop to the other edge / monitor. The UI lays out for it and sends a new
            // shape; until then `apply` leaves the window where it is.
            state.monitor = monitor;
            state.edge = edge;
            state.point = along;
            follow = Spring::at(along, follow.tune);
            grab = 0.0;
            let place = placement_of(state);
            drop(guard);
            let _ = app.emit_to(LABEL, "notch-dock", place);
            continue;
        }
        let Some(shape) = state.shape.filter(|s| s.edge == state.edge) else { continue };
        let (lo, hi) = limits_for(state, &shape);
        follow.target = dock::rubber_band(along - grab, lo, hi, RUBBER * state.monitor.scale);
        follow.step(dt);
        state.point = follow.x;
        let planned = plan(state);
        drop(guard);
        perform(&window, planned);
    }

    // Released: snap to the middle if close, come back inside the limits, and land.
    let target = {
        let guard = notch_state.0.lock().unwrap();
        let Some(state) = guard.as_ref() else { return };
        let (start, len) = dock::along_range(state.monitor.work, state.edge);
        let snapped = dock::snap_to_centre(state.point, start, len, CENTRE_SNAP * state.monitor.scale);
        match state.shape.filter(|s| s.edge == state.edge) {
            Some(shape) => {
                let (lo, hi) = limits_for(state, &shape);
                dock::clamp(snapped, lo, hi)
            }
            None => snapped,
        }
    };
    let _ = app.emit_to(LABEL, "notch-drop", ());
    glide(app, target, spring::with_feel(spring::SNAPPY, config.appearance.spring_feel, reduced));
    if let Some(state) = notch_state.0.lock().unwrap().as_mut() {
        state.dragging = false;
    }
    save_position(app);
}

/// Double-click on the grip, or "Recentre" in settings: spring to the middle of the edge.
pub fn recentre(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let core = app.state::<Arc<Core>>();
        let config = core.config();
        let target = {
            let notch_state = app.state::<Notch>();
            let guard = notch_state.0.lock().unwrap();
            let Some(state) = guard.as_ref() else { return };
            let (start, len) = dock::along_range(state.monitor.work, state.edge);
            dock::from_fraction(0.5, start, len)
        };
        glide(&app, target, spring::with_feel(spring::BOUNCY, config.appearance.spring_feel, reduced_motion(&config)));
        save_position(&app);
    });
}
