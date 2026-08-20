//! Parses the classic EQ ASCII zone-map format (`<install>/maps/*.txt`,
//! and community "map packs" in subfolders like `maps/Brewall/`) -- the
//! same format tools like EQMap/ShowEQ/MQ2 have used for decades:
//!
//! ```text
//! L x1, y1, z1, x2, y2, z2, r, g, b     -- a wall/boundary line segment
//! P x, y, z, r, g, b, size, Label_Text  -- a labeled point (NPC/POI)
//! ```
//!
//! Real 3D data, not a flat 2D minimap format pretending to be 3D --
//! confirmed against the reference install's own files (Befallen's Z
//! ranges -90.6 to +26.1, a multi-level dungeon).
//!
//! A zone's map is split across one base file (`<zone>.txt`) plus
//! optionally-numbered siblings (`<zone>_1.txt`, `<zone>_2.txt`, ...) --
//! confirmed these are *not* alternate layers to pick between: some hold
//! only supplementary points (`befallen_1.txt`: one `to_West_Commonlands`
//! exit marker), but some carry real wall geometry too
//! (`neighborhood_1.txt`: 1,634 `L` lines + 42 `P`). `load_zone_map`
//! always merges every matching file into one picture.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct MapPoint3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// One wall/boundary segment, in the map file's own coordinate order --
/// NOT the same order `/loc` prints in (a real, verified mapping, not an
/// assumption -- see `Ingest::last_loc`'s own doc).
#[derive(Debug, Clone, Copy)]
pub struct MapLine {
    pub a: MapPoint3,
    pub b: MapPoint3,
    pub r: u8,
    pub g: u8,
    pub b_: u8,
}

/// One labeled point -- an NPC, a zone-line exit, a point of interest.
#[derive(Debug, Clone)]
pub struct MapMarker {
    pub pos: MapPoint3,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub size: u8,
    pub label: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedZoneMap {
    pub lines: Vec<MapLine>,
    pub markers: Vec<MapMarker>,
}

/// Parses either a bare integer (`482`) or a decimal (`-123.6500`) --
/// real files mix both freely within the same line (confirmed:
/// `"-123.6500, 482, -87.3500"` in the reference install's own
/// `befallen.txt`), so a parser that only accepts one shape silently
/// drops real geometry.
fn parse_f32(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// One `L`/`P` line's own comma-separated fields, trimmed -- real files
/// use inconsistent spacing after commas (single *and* double spaces
/// seen in the same file), so this always trims rather than assuming a
/// fixed-width split.
fn fields(rest: &str) -> Vec<&str> {
    rest.split(',').map(str::trim).collect()
}

/// The line-format parser itself. Unrecognized lines (a stray blank, a
/// tag this format doesn't define) are skipped rather than treated as an
/// error -- these files are hand/tool-authored over decades by many
/// different community map-pack maintainers, not a format this app
/// controls.
pub fn parse_map_text(text: &str) -> ParsedZoneMap {
    let mut out = ParsedZoneMap::default();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("L ") {
            let f = fields(rest);
            if f.len() < 9 {
                continue;
            }
            let (Some(x1), Some(y1), Some(z1), Some(x2), Some(y2), Some(z2)) = (
                parse_f32(f[0]),
                parse_f32(f[1]),
                parse_f32(f[2]),
                parse_f32(f[3]),
                parse_f32(f[4]),
                parse_f32(f[5]),
            ) else {
                continue;
            };
            let (Ok(r), Ok(g), Ok(b_)) =
                (f[6].parse::<u8>(), f[7].parse::<u8>(), f[8].parse::<u8>())
            else {
                continue;
            };
            out.lines.push(MapLine {
                a: MapPoint3 { x: x1, y: y1, z: z1 },
                b: MapPoint3 { x: x2, y: y2, z: z2 },
                r,
                g,
                b_,
            });
        } else if let Some(rest) = line.strip_prefix("P ") {
            let f = fields(rest);
            if f.len() < 8 {
                continue;
            }
            let (Some(x), Some(y), Some(z)) = (parse_f32(f[0]), parse_f32(f[1]), parse_f32(f[2]))
            else {
                continue;
            };
            let (Ok(r), Ok(g), Ok(b), Ok(size)) = (
                f[3].parse::<u8>(),
                f[4].parse::<u8>(),
                f[5].parse::<u8>(),
                f[6].parse::<u8>(),
            ) else {
                continue;
            };
            // The label is everything after the 7th comma, rejoined --
            // not assumed to be exactly one field, in case a label ever
            // legitimately contains a comma.
            let label = f[7..].join(", ");
            out.markers.push(MapMarker {
                pos: MapPoint3 { x, y, z },
                r,
                g,
                b,
                size,
                label,
            });
        }
    }
    out
}

/// `maps/`, resolved under the game's base install folder -- the one
/// stored path every existing feature (log tailing, inventory dumps)
/// already reads external files from (`AppConfig::base_dir`).
fn maps_root(base_dir: &Path) -> PathBuf {
    base_dir.join("maps")
}

/// A safe, single-path-component name -- rejects anything with a
/// directory separator or `..`, the same discipline
/// `inventory::dump_path` uses for a filename lifted off a log line, even
/// though `pack`/`zone` here come from the frontend rather than the log:
/// neither is a source this app fully trusts to never send a stray `..`.
fn safe_component(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\');
    ok.then_some(name)
}

/// Every subfolder of `maps/` -- each one a community map pack (e.g.
/// `Brewall`). Empty is valid: base game maps only, no pack installed.
pub fn list_map_packs(base_dir: &Path) -> Vec<String> {
    let root = maps_root(base_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut packs: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .collect();
    packs.sort();
    packs
}

/// A `<zone>_<digits>.txt` sibling's own zone stem -- `"befallen_1"` ->
/// `"befallen"`, `"befallen"` -> `"befallen"` (no suffix to strip).
fn zone_stem(file_stem: &str) -> &str {
    match file_stem.rsplit_once('_') {
        Some((base, suffix)) if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => file_stem,
    }
}

/// Every distinct zone with at least one map file in `base_dir/maps` (or
/// `base_dir/maps/<pack>` when `pack` is given) -- numbered siblings
/// collapsed to their own zone's single entry, sorted.
pub fn list_zone_names(base_dir: &Path, pack: Option<&str>) -> Vec<String> {
    let mut root = maps_root(base_dir);
    if let Some(p) = pack.and_then(safe_component) {
        root.push(p);
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut zones: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| name.ends_with(".txt") && !name.ends_with(".txt:crc"))
        .filter_map(|name| {
            let stem = name.strip_suffix(".txt")?;
            Some(zone_stem(stem).to_string())
        })
        .collect();
    zones.sort();
    zones.dedup();
    zones
}

/// Every zone with at least one map file anywhere under `base_dir/maps` --
/// base game plus every community pack, deduped -- for the Maps module's
/// zone-first picker. `list_zone_versions` is what then tells the
/// frontend which source(s) actually cover a chosen zone; this replaces
/// making the user pick a pack before they can even see whether their
/// zone has a map at all.
pub fn list_all_zone_names(base_dir: &Path) -> Vec<String> {
    let mut zones = list_zone_names(base_dir, None);
    for pack in list_map_packs(base_dir) {
        zones.extend(list_zone_names(base_dir, Some(&pack)));
    }
    zones.sort();
    zones.dedup();
    zones
}

/// Every distinct source that has at least one map file for `zone` --
/// `None` for the base game, `Some(pack)` for each community pack that
/// also covers it (e.g. Befallen: base game + Brewall, two different
/// renderings of the same zone). Base game sorts first when present;
/// packs alphabetically after (`list_map_packs` is already sorted).
/// Drives the "available versions" picker once a zone is chosen.
pub fn list_zone_versions(base_dir: &Path, zone: &str) -> Vec<Option<String>> {
    let mut out = Vec::new();
    if list_zone_names(base_dir, None).iter().any(|z| z == zone) {
        out.push(None);
    }
    for pack in list_map_packs(base_dir) {
        if list_zone_names(base_dir, Some(&pack)).iter().any(|z| z == zone) {
            out.push(Some(pack));
        }
    }
    out
}

/// Reads and merges every `<zone>*.txt` file for `zone` (base file plus
/// every numbered sibling) into one `ParsedZoneMap` -- see this module's
/// own doc for why merging, not picking one, is correct here. `Ok` with
/// empty vecs if `zone` has files but they're all unparseable garbage;
/// `Err` only if the directory itself can't be listed (missing/
/// unreadable `maps/` folder).
pub fn load_zone_map(base_dir: &Path, pack: Option<&str>, zone: &str) -> std::io::Result<ParsedZoneMap> {
    let zone = safe_component(zone)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("not a plain zone name: {zone}")))?;
    let mut root = maps_root(base_dir);
    if let Some(p) = pack.and_then(safe_component) {
        root.push(p);
    }
    let entries = std::fs::read_dir(&root)?;
    let mut out = ParsedZoneMap::default();
    for entry in entries.filter_map(Result::ok) {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".txt") else {
            continue;
        };
        if zone_stem(stem) != zone {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let parsed = parse_map_text(&text);
        out.lines.extend(parsed.lines);
        out.markers.extend(parsed.markers);
    }
    Ok(out)
}

// ---------------------------------------------------------------- DTOs

#[derive(Debug, Clone, Copy, Serialize)]
pub struct MapLineDto {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub color: [u8; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct MapMarkerDto {
    pub pos: [f32; 3],
    pub color: [u8; 3],
    pub size: u8,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MapFileDto {
    pub lines: Vec<MapLineDto>,
    pub markers: Vec<MapMarkerDto>,
}

impl From<ParsedZoneMap> for MapFileDto {
    fn from(parsed: ParsedZoneMap) -> Self {
        MapFileDto {
            lines: parsed
                .lines
                .into_iter()
                .map(|l| MapLineDto {
                    a: [l.a.x, l.a.y, l.a.z],
                    b: [l.b.x, l.b.y, l.b.z],
                    color: [l.r, l.g, l.b_],
                })
                .collect(),
            markers: parsed
                .markers
                .into_iter()
                .map(|m| MapMarkerDto {
                    pos: [m.pos.x, m.pos.y, m.pos.z],
                    color: [m.r, m.g, m.b],
                    size: m.size,
                    label: m.label,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real lines, `befallen.txt` -- includes the mixed-decimal/bare-
    /// integer case ("482" with no decimal point alongside "-123.6500"
    /// with one) that a naive float parser could silently drop.
    #[test]
    fn parses_real_wall_lines_including_bare_integer_coordinates() {
        let text = "\
L 143.2200, 998.3000, -90.6200, 153.0600, 990.1100, -80.7100, 128, 128, 128
L -115.0100, 493.1600, -87.4200, -123.6500, 482, -87.3500, 128, 128, 128
";
        let parsed = parse_map_text(text);
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].a.x, 143.22);
        assert_eq!(parsed.lines[0].b.z, -80.71);
        assert_eq!(parsed.lines[0].r, 128);
        assert_eq!(parsed.lines[1].b.y, 482.0, "bare integer, no decimal point");
    }

    /// Real line, `abysmal.txt` -- a labeled merchant marker, double-
    /// spaced fields (real files aren't consistently single-space-after-
    /// comma).
    #[test]
    fn parses_a_real_labeled_marker() {
        let text = "P 195.0000, 210.0000, 94.8135,  0, 0, 0,  3,  Gruppip_(Wizard_Spells)";
        let parsed = parse_map_text(text);
        assert_eq!(parsed.markers.len(), 1);
        let m = &parsed.markers[0];
        assert_eq!(m.pos.x, 195.0);
        assert_eq!(m.pos.z, 94.8135);
        assert_eq!(m.size, 3);
        assert_eq!(m.label, "Gruppip_(Wizard_Spells)");
    }

    #[test]
    fn zone_stem_strips_only_a_trailing_numeric_suffix() {
        assert_eq!(zone_stem("befallen_1"), "befallen");
        assert_eq!(zone_stem("befallen"), "befallen");
        // A real zone whose own name ends in digits must not be mistaken
        // for a numbered sibling of a shorter base name.
        assert_eq!(zone_stem("povalor"), "povalor");
    }

    #[test]
    fn unparseable_lines_are_skipped_not_errored() {
        let text = "not a map line at all\nL too, few, fields\n";
        let parsed = parse_map_text(text);
        assert!(parsed.lines.is_empty());
        assert!(parsed.markers.is_empty());
    }

    /// why: a fresh, isolated `maps/` scratch dir per test -- same
    /// convention `inventory.rs`'s `find_existing_dump_tests` uses, not a
    /// new `tempfile` dependency for two tests.
    fn scratch_maps_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eqlp-mapsdata-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        dir
    }

    /// A zone covered by both the base game and one community pack shows
    /// up as two versions, base first; a zone only a pack has shows up as
    /// one; a zone neither has shows up as none.
    #[test]
    fn list_zone_versions_reports_every_source_that_covers_a_zone() {
        let base_dir = scratch_maps_dir("versions");
        let maps = base_dir.join("maps");
        std::fs::write(maps.join("befallen.txt"), "").unwrap();
        std::fs::write(maps.join("neighborhood.txt"), "").unwrap();
        let brewall = maps.join("Brewall");
        std::fs::create_dir_all(&brewall).unwrap();
        std::fs::write(brewall.join("befallen.txt"), "").unwrap();
        std::fs::write(brewall.join("cazicthule.txt"), "").unwrap();

        assert_eq!(
            list_zone_versions(&base_dir, "befallen"),
            vec![None, Some("Brewall".to_string())],
            "base game first, then the pack"
        );
        assert_eq!(
            list_zone_versions(&base_dir, "neighborhood"),
            vec![None],
            "base-game-only zone"
        );
        assert_eq!(
            list_zone_versions(&base_dir, "cazicthule"),
            vec![Some("Brewall".to_string())],
            "pack-only zone"
        );
        assert_eq!(
            list_zone_versions(&base_dir, "nonexistent"),
            Vec::<Option<String>>::new()
        );
    }

    #[test]
    fn list_all_zone_names_unions_base_and_every_pack_deduped() {
        let base_dir = scratch_maps_dir("union");
        let maps = base_dir.join("maps");
        std::fs::write(maps.join("befallen.txt"), "").unwrap();
        let brewall = maps.join("Brewall");
        std::fs::create_dir_all(&brewall).unwrap();
        std::fs::write(brewall.join("befallen.txt"), "").unwrap();
        std::fs::write(brewall.join("cazicthule.txt"), "").unwrap();

        assert_eq!(
            list_all_zone_names(&base_dir),
            vec!["befallen".to_string(), "cazicthule".to_string()],
            "befallen from both sources collapses to one entry"
        );
    }
}
