//! why: the overlay is a negotiated capability, not an assumption -- see
//! FOUNDATION.md #4. Wayland's own native protocol can't do always-on-top
//! (tao#1134, upstream) or click-through. main.rs forces GDK_BACKEND to
//! prefer X11, so on a Wayland desktop this app's own window runs through
//! XWayland instead -- the near-universal X11-compatibility layer
//! GNOME/KDE/etc. ship by default for exactly this kind of app. That's
//! still just a second, ordinary window the compositor draws on top;
//! nothing about it touches the game process, its memory, or its render
//! pipeline -- categorically different from an injected/hooked overlay.
//! Detected once, features ask this, never assume a floating window exists.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowCapability {
    /// why: always available, every platform -- the only guaranteed tier
    Docked,
    /// why: floating always-on-top window -- X11 (real or via XWayland), Windows, macOS
    Floating,
    /// why: floating + click passthrough to the game underneath -- X11 (real or via XWayland), Windows, macOS
    ClickThrough,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowCapabilityDto {
    pub capability: WindowCapability,
    /// why: plain-language reason when capped below ClickThrough, for the UI to show directly
    pub reason: Option<String>,
}

/// why: real runtime check, not a guess from target_os alone -- Linux's
/// own capability depends on which display server is actually reachable,
/// not just "this is Linux"
pub fn detect() -> WindowCapabilityDto {
    if !cfg!(target_os = "linux") {
        // why: Windows/macOS always have both -- no known upstream gap there
        return WindowCapabilityDto {
            capability: WindowCapability::ClickThrough,
            reason: None,
        };
    }
    detect_linux()
}

/// why: split out so the real decision logic is unit-testable without
/// mutating real process env vars (see tests below)
fn classify_linux(
    session_type: Option<&str>,
    wayland_display: Option<&str>,
    x11_display: Option<&str>,
) -> WindowCapabilityDto {
    let is_wayland_session = session_type.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
        || (session_type.is_none_or(str::is_empty) && wayland_display.is_some());
    if !is_wayland_session {
        return WindowCapabilityDto {
            capability: WindowCapability::ClickThrough,
            reason: None,
        };
    }
    // why: a Wayland session almost always still has a real DISPLAY --
    // XWayland running underneath (GNOME/KDE both enable it by default).
    // main.rs's own GDK_BACKEND override sends this app's window through
    // that X11 layer instead of the native Wayland one, so a reachable
    // DISPLAY here really does mean ClickThrough works, not a guess.
    if x11_display.is_some_and(|d| !d.is_empty()) {
        return WindowCapabilityDto {
            capability: WindowCapability::ClickThrough,
            reason: None,
        };
    }
    WindowCapabilityDto {
        capability: WindowCapability::Docked,
        reason: Some(
            "Floating overlays need X11 or XWayland; this session has neither reachable."
                .to_string(),
        ),
    }
}

fn detect_linux() -> WindowCapabilityDto {
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let x11_display = std::env::var("DISPLAY").ok();
    classify_linux(
        session_type.as_deref(),
        wayland_display.as_deref(),
        x11_display.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x11_session_type_allows_click_through() {
        let dto = classify_linux(Some("x11"), None, Some(":0"));
        assert_eq!(dto.capability, WindowCapability::ClickThrough);
        assert!(dto.reason.is_none());
    }

    /// why: the real, common case -- a Wayland session with XWayland
    /// still running underneath (GNOME/KDE default), which is what
    /// GDK_BACKEND=x11,wayland actually lets this app use
    #[test]
    fn wayland_session_with_a_reachable_display_allows_click_through() {
        let dto = classify_linux(Some("wayland"), Some("wayland-0"), Some(":0"));
        assert_eq!(dto.capability, WindowCapability::ClickThrough);
        assert!(dto.reason.is_none());
    }

    /// why: the genuinely rare case -- a Wayland-only compositor with no
    /// XWayland at all, nothing this app can do about that
    #[test]
    fn wayland_session_with_no_reachable_display_caps_at_docked() {
        let dto = classify_linux(Some("wayland"), Some("wayland-0"), None);
        assert_eq!(dto.capability, WindowCapability::Docked);
        assert!(dto.reason.is_some());
    }

    /// why: some setups never export XDG_SESSION_TYPE at all -- WAYLAND_DISPLAY's
    /// own presence is the fallback signal every other real check falls back to
    #[test]
    fn missing_session_type_falls_back_to_wayland_display() {
        let dto = classify_linux(None, Some("wayland-0"), None);
        assert_eq!(dto.capability, WindowCapability::Docked);
    }

    #[test]
    fn missing_every_signal_assumes_x11() {
        let dto = classify_linux(None, None, None);
        assert_eq!(dto.capability, WindowCapability::ClickThrough);
    }
}
