//! why: crash-safe JSON persistence -- every store here (config,
//! preferences, notification settings, profiles) loads "unreadable" as
//! "never saved", so a bare fs::write torn by a crash/power cut silently
//! resets the user (back to first-launch, positions gone). Temp file +
//! rename is atomic on NTFS and ext4 within one directory.

use std::io;
use std::path::Path;

/// why: sibling temp name, same directory -- rename across volumes isn't atomic
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = Path::new(&tmp);
    std::fs::write(tmp, bytes)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_existing_content() {
        let dir = std::env::temp_dir().join(format!("eqlp-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!dir.join("config.json.tmp").exists(), "temp file cleaned up by rename");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
