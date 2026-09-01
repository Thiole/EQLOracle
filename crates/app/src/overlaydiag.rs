//! why: Windows overlay reports arrive as "nothing shows" with no data.
//! This reads back what the OS actually thinks of each overlay window
//! (style bits, layered attributes, visibility, cloaking) so a report
//! becomes facts. Read by the Debug tab and by `--overlay-probe`.

use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
pub struct MonitorDto {
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// why: Option fields -- each read can fail independently; a failed
/// read is reported as absent, never guessed
#[derive(Debug, Clone, Serialize)]
pub struct OverlayWindowDiag {
    pub label: String,
    pub tauri_visible: Option<bool>,
    /// why: physical pixels, straight from the OS -- comparable to rect
    pub outer_x: Option<i32>,
    pub outer_y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub win32: Option<Win32Diag>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Win32Diag {
    pub ex_style: u32,
    /// why: decoded so a pasted report is readable without a bit table
    pub ex_flags: Vec<String>,
    pub visible: bool,
    pub iconic: bool,
    /// why: None on a LAYERED window means SetLayeredWindowAttributes
    /// was never applied -- exactly the "running but invisible" state
    pub layered_alpha: Option<u8>,
    /// why: DWM cloaking hides a window that Win32 still calls visible
    pub cloaked: Option<u32>,
    pub rect: [i32; 4],
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayDiagnosticsDto {
    pub version: String,
    pub platform: String,
    pub capability: crate::windowcap::WindowCapabilityDto,
    pub monitors: Vec<MonitorDto>,
    pub overlays: Vec<OverlayWindowDiag>,
}

pub fn collect(app: &AppHandle) -> OverlayDiagnosticsDto {
    let monitors = app
        .available_monitors()
        .map(|ms| {
            ms.iter()
                .map(|m| MonitorDto {
                    name: m.name().cloned(),
                    x: m.position().x,
                    y: m.position().y,
                    width: m.size().width,
                    height: m.size().height,
                    scale: m.scale_factor(),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut overlays: Vec<OverlayWindowDiag> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("overlay-"))
        .map(|(label, w)| OverlayWindowDiag {
            label,
            tauri_visible: w.is_visible().ok(),
            outer_x: w.outer_position().ok().map(|p| p.x),
            outer_y: w.outer_position().ok().map(|p| p.y),
            width: w.outer_size().ok().map(|s| s.width),
            height: w.outer_size().ok().map(|s| s.height),
            win32: win32_diag(&w),
        })
        .collect();
    overlays.sort_by(|a, b| a.label.cmp(&b.label));
    OverlayDiagnosticsDto {
        version: app.package_info().version.to_string(),
        platform: std::env::consts::OS.to_string(),
        capability: crate::windowcap::detect(),
        monitors,
        overlays,
    }
}

#[cfg(target_os = "windows")]
fn win32_diag(window: &tauri::WebviewWindow) -> Option<Win32Diag> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetLayeredWindowAttributes, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible,
        GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_EX_TRANSPARENT,
    };
    let hwnd = window.hwnd().ok()?.0 as isize;
    unsafe {
        let ex = GetWindowLongPtrW(hwnd as _, GWL_EXSTYLE) as u32;
        let mut ex_flags = Vec::new();
        for (bit, name) in [
            (WS_EX_LAYERED, "LAYERED"),
            (WS_EX_TRANSPARENT, "TRANSPARENT"),
            (WS_EX_TOOLWINDOW, "TOOLWINDOW"),
            (WS_EX_APPWINDOW, "APPWINDOW"),
            (WS_EX_NOACTIVATE, "NOACTIVATE"),
            (WS_EX_TOPMOST, "TOPMOST"),
        ] {
            if ex & bit != 0 {
                ex_flags.push(name.to_string());
            }
        }
        let mut key = 0u32;
        let mut alpha = 0u8;
        let mut flags = 0u32;
        let layered_alpha =
            (GetLayeredWindowAttributes(hwnd as _, &mut key, &mut alpha, &mut flags) != 0)
                .then_some(alpha);
        let mut cloak = 0u32;
        let cloaked = (DwmGetWindowAttribute(
            hwnd as _,
            DWMWA_CLOAKED as u32,
            &mut cloak as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        ) == 0)
            .then_some(cloak);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let _ = GetWindowRect(hwnd as _, &mut rect);
        Some(Win32Diag {
            ex_style: ex,
            ex_flags,
            visible: IsWindowVisible(hwnd as _) != 0,
            iconic: IsIconic(hwnd as _) != 0,
            layered_alpha,
            cloaked,
            rect: [rect.left, rect.top, rect.right, rect.bottom],
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn win32_diag(_window: &tauri::WebviewWindow) -> Option<Win32Diag> {
    None
}
