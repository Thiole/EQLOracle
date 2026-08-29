//! why: a downloaded AppImage installs itself as a real app on first
//! launch -- copied to `~/Applications/EQL-Oracle.AppImage` (the
//! AppImageLauncher/Gear Lever convention) with a menu entry and icon,
//! then execs the installed copy. Every later double-click of ANY
//! downloaded copy hands off the same way, so stale files in Downloads
//! never run their own old code or fight the in-app updater; the one
//! installed copy is the only one that ever really runs. The download
//! itself is left where the user put it -- never deleted.

#![cfg(target_os = "linux")]

use std::io;
use std::path::{Path, PathBuf};

const INSTALL_NAME: &str = "EQL-Oracle.AppImage";
/// why: sibling dotfile, not app-data -- must be readable before Tauri
/// (and its path resolver) exists, and must move with the install dir
const VERSION_MARKER: &str = ".EQL-Oracle.AppImage.version";
const DESKTOP_ID: &str = "com.eqlp.oracle";

/// why: called first thing in main(). Execs (never returns) when handing
/// off to the installed copy; returns to run this process normally when
/// this IS the installed copy, isn't an AppImage at all, or handoff fails.
/// `current_version` comes from the generated tauri context, not
/// CARGO_PKG_VERSION -- CI's --config override gives testing builds a
/// synthetic version only the context sees (see 3-release.yml).
pub fn install_or_handoff(current_version: &str) {
    // why: loop guard -- the exec'd copy must never re-enter this logic
    if std::env::var_os("EQLP_HANDOFF").is_some() {
        return;
    }
    // why: $APPIMAGE is the real outer file; current_exe() is the FUSE
    // mount and useless here -- see updater.rs's executable_path doc.
    // Absent means .deb/.rpm/local build: already properly installed.
    let Ok(appimage) = std::env::var("APPIMAGE") else {
        return;
    };
    let appimage = PathBuf::from(appimage);
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let install_dir = home.join("Applications");
    let install_path = install_dir.join(INSTALL_NAME);

    // why: canonicalize both -- a symlinked launch of the installed file
    // must count as installed, not trigger a copy-over-self
    let is_installed_copy = match (appimage.canonicalize(), install_path.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if is_installed_copy {
        // why: refreshed every launch -- the in-app updater rewrites the
        // AppImage in place, so the marker goes stale until here; the
        // desktop entry rewrite is idempotent and self-heals a deleted one
        write_desktop_integration(&install_path, current_version);
        let _ = std::fs::write(install_dir.join(VERSION_MARKER), current_version);
        return;
    }

    let marker = std::fs::read_to_string(install_dir.join(VERSION_MARKER)).ok();
    if should_replace(install_path.exists(), marker.as_deref(), current_version) {
        if let Err(e) = install_copy(&appimage, &install_dir, &install_path) {
            // why: never block launch on install trouble -- run the copy
            // the user actually clicked and try again next time
            eprintln!(
                "eqlp: self-install to {} failed ({e}); running from {}",
                install_path.display(),
                appimage.display()
            );
            return;
        }
        write_desktop_integration(&install_path, current_version);
        let _ = std::fs::write(install_dir.join(VERSION_MARKER), current_version);
    }

    use std::os::unix::process::CommandExt;
    // why: the AppImage runtime re-sets $APPIMAGE to its own path on exec,
    // so the installed copy sees itself correctly, not our stale value
    let err = std::process::Command::new(&install_path)
        .args(std::env::args_os().skip(1))
        .env("EQLP_HANDOFF", "1")
        .exec();
    eprintln!(
        "eqlp: handoff to {} failed ({err}); running this copy",
        install_path.display()
    );
}

/// why: pure decision so it's testable. Marker-missing with a file
/// present means a copy we didn't write (user-placed) -- don't clobber
/// it, just hand off; its own updater catches it up if it's old.
fn should_replace(install_exists: bool, installed_version: Option<&str>, current: &str) -> bool {
    if !install_exists {
        return true;
    }
    match installed_version {
        Some(v) => parse_version(current) > parse_version(v),
        None => false,
    }
}

/// why: numeric dot-parts only ("0.1.52" -> [0,1,52]) -- enough for this
/// app's real version scheme, no semver prerelease ordering
fn parse_version(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// why: copy to a temp name then rename -- a crash mid-copy must never
/// leave a truncated file at the install path
fn install_copy(src: &Path, dir: &Path, dest: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    let part = dir.join(".EQL-Oracle.AppImage.part");
    std::fs::copy(src, &part)?;
    std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&part, dest)?;
    Ok(())
}

fn data_home() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_DATA_HOME") {
        if !x.is_empty() {
            return Some(PathBuf::from(x));
        }
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))
}

/// why: menu entry + icon, best-effort -- a failure here still leaves a
/// working installed AppImage, just without desktop integration
fn write_desktop_integration(install_path: &Path, version: &str) {
    let Some(data) = data_home() else {
        return;
    };
    let icon_dir = data.join("icons/hicolor/128x128/apps");
    if std::fs::create_dir_all(&icon_dir).is_ok() {
        let _ = std::fs::write(
            icon_dir.join(format!("{DESKTOP_ID}.png")),
            include_bytes!("../icons/128x128.png"),
        );
    }
    let apps_dir = data.join("applications");
    if std::fs::create_dir_all(&apps_dir).is_ok() {
        let _ = std::fs::write(
            apps_dir.join(format!("{DESKTOP_ID}.desktop")),
            desktop_entry(install_path, version),
        );
        // why: refreshes the menu cache where present; harmless where not
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&apps_dir)
            .output();
    }
}

/// why: Exec is double-quoted per the desktop-entry spec's quoting rules
/// so a space in $HOME can't split the path into arguments
fn desktop_entry(install_path: &Path, version: &str) -> String {
    let exec = install_path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=EQL Oracle\n\
         Comment=EverQuest log companion\n\
         Exec=\"{exec}\"\n\
         Icon={DESKTOP_ID}\n\
         Terminal=false\n\
         Categories=Game;Utility;\n\
         StartupWMClass=eqlp-app\n\
         X-AppImage-Version={version}\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parse_orders_real_release_numbers() {
        assert!(parse_version("0.1.52") > parse_version("0.1.9"));
        assert!(parse_version("0.2.0") > parse_version("0.1.52"));
        assert_eq!(parse_version("0.1.52"), parse_version("0.1.52"));
    }

    #[test]
    fn replace_only_on_missing_install_or_strictly_newer_download() {
        assert!(should_replace(false, None, "0.1.52"));
        assert!(should_replace(true, Some("0.1.51"), "0.1.52"));
        assert!(!should_replace(true, Some("0.1.52"), "0.1.52"));
        // why: launching an OLD download must not downgrade the install
        assert!(!should_replace(true, Some("0.1.53"), "0.1.52"));
        // why: unknown provenance -- hand off, never clobber
        assert!(!should_replace(true, None, "0.1.52"));
    }

    #[test]
    fn desktop_entry_quotes_exec_and_names_the_icon() {
        let entry = desktop_entry(
            Path::new("/home/a user/Applications/EQL-Oracle.AppImage"),
            "0.2.0",
        );
        assert!(entry.contains("Exec=\"/home/a user/Applications/EQL-Oracle.AppImage\"\n"));
        assert!(entry.contains("Icon=com.eqlp.oracle\n"));
        assert!(entry.contains("Name=EQL Oracle\n"));
    }

    #[test]
    fn install_copy_lands_executable_with_no_part_file_left() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("eqlp-selfinstall-{}", std::process::id()));
        let dir = base.join("Applications");
        std::fs::create_dir_all(&base).unwrap();
        let src = base.join("download.AppImage");
        std::fs::write(&src, b"payload").unwrap();
        let dest = dir.join(INSTALL_NAME);
        install_copy(&src, &dir, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"payload");
        assert_ne!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o111,
            0
        );
        assert!(!dir.join(".EQL-Oracle.AppImage.part").exists());
        std::fs::remove_dir_all(&base).ok();
    }
}
