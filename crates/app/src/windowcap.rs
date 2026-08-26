//! why: the overlay is a negotiated capability, not an assumption -- see
//! FOUNDATION.md #4. Wayland can't do always-on-top (tao#1134, upstream)
//! or click-through either; detected once, features ask this, never
//! assume a floating window exists.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowCapability {
    /// why: always available, every platform -- the only guaranteed tier
    Docked,
    /// why: floating always-on-top window -- X11/Windows/macOS only
    Floating,
    /// why: floating + click passthrough to the game underneath -- X11/Windows/macOS only
    ClickThrough,
}

#[derive(Debug, Clone, Serialize)]
pub struct WindowCapabilityDto {
    pub capability: WindowCapability,
    /// why: plain-language reason when capped below ClickThrough, for the UI to show directly
    pub reason: Option<String>,
}

/// why: real runtime check, not a guess from target_os alone -- Linux's
/// own capability depends on which display server is actually running,
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
) -> WindowCapabilityDto {
    let is_wayland = session_type.is_some_and(|s| s.eq_ignore_ascii_case("wayland"))
        || (session_type.is_none_or(str::is_empty) && wayland_display.is_some());
    if is_wayland {
        WindowCapabilityDto {
            capability: WindowCapability::Docked,
            reason: Some(
                "Floating overlays need X11; this session is running under Wayland.".to_string(),
            ),
        }
    } else {
        WindowCapabilityDto {
            capability: WindowCapability::ClickThrough,
            reason: None,
        }
    }
}

fn detect_linux() -> WindowCapabilityDto {
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    classify_linux(session_type.as_deref(), wayland_display.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_wayland_session_type_caps_at_docked() {
        let dto = classify_linux(Some("wayland"), None);
        assert_eq!(dto.capability, WindowCapability::Docked);
        assert!(dto.reason.is_some());
    }

    #[test]
    fn explicit_x11_session_type_allows_click_through() {
        let dto = classify_linux(Some("x11"), None);
        assert_eq!(dto.capability, WindowCapability::ClickThrough);
        assert!(dto.reason.is_none());
    }

    /// why: some setups never export XDG_SESSION_TYPE at all -- WAYLAND_DISPLAY's
    /// own presence is the fallback signal every other real check falls back to
    #[test]
    fn missing_session_type_falls_back_to_wayland_display() {
        let dto = classify_linux(None, Some("wayland-0"));
        assert_eq!(dto.capability, WindowCapability::Docked);
    }

    #[test]
    fn missing_both_signals_assumes_x11() {
        let dto = classify_linux(None, None);
        assert_eq!(dto.capability, WindowCapability::ClickThrough);
    }
}
