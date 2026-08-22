//! why: verify uifiles::parse_ini against real files, including the 2
//!      known-corrupted ones, not just synthetic fixtures
//! input: path to the game's base install folder
//! output: every found file, section count, skipped-garbage count
//! run: cargo run -p eqlp-app --example uifiles_check -- <base_dir>

use eqlp_app::uifiles::{list_ui_files, parse_ini, ui_file_path};
use std::path::Path;

fn main() {
    let base_dir = std::env::args()
        .nth(1)
        .expect("usage: uifiles_check <base_dir>");
    let base_dir = Path::new(&base_dir);
    for f in list_ui_files(base_dir) {
        let path = ui_file_path(base_dir, &f.file).unwrap();
        match parse_ini(&path) {
            Ok(parsed) => {
                let flag = if parsed.skipped_garbage_lines > 10 {
                    " <-- LIKELY CORRUPTED"
                } else {
                    ""
                };
                println!(
                    "{:<45} {:<8} char={:<12} zone={:<10} backup={:<5} sections={:<4} skipped={}{}",
                    f.file,
                    f.kind,
                    f.character,
                    f.zone,
                    f.is_backup,
                    parsed.sections.len(),
                    parsed.skipped_garbage_lines,
                    flag
                );
            }
            Err(e) => println!("{:<45} ERROR: {e}", f.file),
        }
    }
}
