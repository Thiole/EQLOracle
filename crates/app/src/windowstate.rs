//! why: the main window opens where it was left, at the size it was left.
//! Tracked in memory on every move/resize and written once on close, so a
//! drag never touches the disk. Reported real: every launch reset the
//! window to tauri.conf's 1040x720 at the OS default position.

use crate::preferences::{self, WindowState};
use crate::state::LockRecover;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// why: Windows parks a minimized window at -32000; saving that would
/// reopen it off-screen forever. Any sane coordinate is far above this.
const MINIMIZED_SENTINEL: f64 = -30_000.0;

/// why: the live geometry, only when it is worth keeping -- a minimized
/// or maximized window reports the wrong thing to restore to
pub fn capture(w: &WebviewWindow) -> Option<WindowState> {
    if w.is_minimized().unwrap_or(false) {
        return None;
    }
    let maximized = w.is_maximized().unwrap_or(false);
    let scale = w.scale_factor().ok()?;
    let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
    let size = w.inner_size().ok()?.to_logical::<f64>(scale);
    if pos.x <= MINIMIZED_SENTINEL || pos.y <= MINIMIZED_SENTINEL {
        return None;
    }
    if size.width < 1.0 || size.height < 1.0 {
        return None;
    }
    Some(WindowState {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized,
    })
}

/// why: one tracked value per app -- every Moved/Resized updates it, and
/// only a real un-maximized geometry overwrites x/y/w/h, so un-maximizing
/// after a restart lands on the size the user actually chose
pub fn track(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let Some(now) = capture(&w) else { return };
    let handle = app.state::<Tracked>();
    let mut tracked = handle.0.lock_recover();
    match tracked.as_mut() {
        Some(prev) if now.maximized => prev.maximized = true,
        _ => *tracked = Some(now),
    }
}

/// why: written once, on the way out -- a move event fires per pixel
pub fn save(app: &AppHandle) {
    let handle = app.state::<Tracked>();
    let tracked = *handle.0.lock_recover();
    let Some(state) = tracked else { return };
    let mut prefs = preferences::load(app);
    if prefs.main_window == Some(state) {
        return;
    }
    prefs.main_window = Some(state);
    let _ = preferences::save(app, &prefs);
}

/// why: size first, then position -- setting size can nudge a window on
/// some window managers, so position lands last and wins. A saved point
/// that no longer falls on any monitor is dropped, not restored: an
/// unplugged second display would otherwise reopen the app off-screen
/// (the same guard the overlay windows already use).
pub fn restore(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let Some(state) = preferences::load(app).main_window else {
        return;
    };
    let handle = app.state::<Tracked>();
    *handle.0.lock_recover() = Some(state);
    let Ok(scale) = w.scale_factor() else { return };
    let size = PhysicalSize::new(
        (state.width * scale).round() as u32,
        (state.height * scale).round() as u32,
    );
    let _ = w.set_size(size);
    if crate::commands::position_on_some_monitor(app, state.x, state.y) {
        let _ = w.set_position(PhysicalPosition::new(
            (state.x * scale).round() as i32,
            (state.y * scale).round() as i32,
        ));
    }
    if state.maximized {
        let _ = w.maximize();
    }
}

/// why: the in-memory half -- managed state so the event handler and the
/// close hook share one value without a global
#[derive(Default)]
pub struct Tracked(pub std::sync::Mutex<Option<WindowState>>);

#[cfg(test)]
mod tests {
    use super::*;

    /// why: the two readings that must never be written -- Windows'
    /// minimized sentinel and a degenerate size
    #[test]
    fn a_minimized_or_degenerate_geometry_is_never_worth_saving() {
        let sane = WindowState {
            x: 100.0,
            y: 50.0,
            width: 1200.0,
            height: 800.0,
            maximized: false,
        };
        assert!(sane.x > MINIMIZED_SENTINEL && sane.width >= 1.0);
        let minimized = WindowState {
            x: -32_000.0,
            y: -32_000.0,
            ..sane
        };
        assert!(minimized.x <= MINIMIZED_SENTINEL, "the sentinel is caught");
        let collapsed = WindowState {
            width: 0.0,
            height: 0.0,
            ..sane
        };
        assert!(collapsed.width < 1.0, "a zero size is caught");
    }

    /// why: a maximized reading must not overwrite the size to
    /// un-maximize back to -- only the flag
    #[test]
    fn maximizing_keeps_the_restored_size_underneath() {
        let mut tracked = Some(WindowState {
            x: 100.0,
            y: 50.0,
            width: 1200.0,
            height: 800.0,
            maximized: false,
        });
        let maxed = WindowState {
            x: 0.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
            maximized: true,
        };
        match tracked.as_mut() {
            Some(prev) if maxed.maximized => prev.maximized = true,
            _ => tracked = Some(maxed),
        }
        let t = tracked.expect("still tracked");
        assert!(t.maximized, "the flag is taken");
        assert_eq!((t.width, t.height), (1200.0, 800.0), "the size is not");
    }
}
