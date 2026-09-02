//! why: parses the classic EQ ASCII zone-map format (EQMap/ShowEQ/MQ2 style)
//!
//! ```text
//! L x1, y1, z1, x2, y2, z2, r, g, b     -- a wall/boundary line segment
//! P x, y, z, r, g, b, size, Label_Text  -- a labeled point (NPC/POI)
//! ```
//!
//! Real 3D data (confirmed: Befallen Z ranges -90.6 to +26.1). A zone
//! splits across a base file plus numbered siblings -- confirmed not
//! alternate layers, some siblings carry real geometry too
//! (`neighborhood_1.txt`: 1,634 L lines). `load_zone_map` merges all of them.

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Default)]
pub struct MapPoint3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// why: map file's own coordinate order, NOT `/loc`'s order -- see `Ingest::last_loc`
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

/// why: real files mix bare integers and decimals freely on the same line
fn parse_f32(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// why: always trims -- real files mix single and double spaces after commas
fn fields(rest: &str) -> Vec<&str> {
    rest.split(',').map(str::trim).collect()
}

/// why: unrecognized lines are skipped, not errored -- decades of
/// hand/tool-authored community files, not a format this app controls
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
                a: MapPoint3 {
                    x: x1,
                    y: y1,
                    z: z1,
                },
                b: MapPoint3 {
                    x: x2,
                    y: y2,
                    z: z2,
                },
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
            // why: rejoined, not assumed one field -- a label may contain a comma
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

/// why: `maps/` under `AppConfig::base_dir`, the one stored path every feature reads from
fn maps_root(base_dir: &Path) -> PathBuf {
    base_dir.join("maps")
}

/// why: rejects separators/`..`, same discipline as `inventory::dump_path`
fn safe_component(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\');
    ok.then_some(name)
}

/// why: every community map pack subfolder; empty is valid (base game only)
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

/// why: a map pack's labeled point for a named mob is a real 3D spawn
/// position -- the wiki's own spawn spots are XY only, and in a stacked
/// swim zone a z-less target resolved to the wrong floor (Kedge: Phinigel
/// guessed onto the -153 floor, Brewall labels him at -294; "the
/// navigation just lingers in the entry chamber"). Every installed pack
/// and layer is searched; a label matches when it folds to the NPC's
/// name (underscores, backticks, a trailing "(Hunter,Roam)" note
/// dropped). Nearest match within `max_dist` XY of the wiki spot wins,
/// so a far-off roam marker can't relocate a mob. Map-file coords.
pub fn labeled_point_for(
    base_dir: &Path,
    zone: &str,
    name: &str,
    near: [f32; 2],
    max_dist: f32,
) -> Option<[f32; 3]> {
    let want = fold_label(name);
    if want.is_empty() {
        return None;
    }
    let mut packs: Vec<Option<String>> = vec![None];
    packs.extend(list_map_packs(base_dir).into_iter().map(Some));
    let mut best: Option<(f32, [f32; 3])> = None;
    for pack in packs {
        let Ok(map) = load_zone_map(base_dir, pack.as_deref(), zone) else {
            continue;
        };
        for m in &map.markers {
            if fold_label(&m.label) != want {
                continue;
            }
            let d = ((m.pos.x - near[0]).powi(2) + (m.pos.y - near[1]).powi(2)).sqrt();
            if d <= max_dist && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, [m.pos.x, m.pos.y, m.pos.z]));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// why: names a candidate floor for the ambiguity list -- Brewall marks
/// rooms ("First_Floor", "Temple_of_Prexus"); the nearest label on that
/// floor's own level reads better than a bare z. Display form.
pub fn label_near(
    base_dir: &Path,
    zone: &str,
    at: [f32; 3],
    max_xy: f32,
    max_dz: f32,
) -> Option<String> {
    let mut packs: Vec<Option<String>> = vec![None];
    packs.extend(list_map_packs(base_dir).into_iter().map(Some));
    let mut best: Option<(f32, String)> = None;
    for pack in packs {
        let Ok(map) = load_zone_map(base_dir, pack.as_deref(), zone) else {
            continue;
        };
        for m in &map.markers {
            if (m.pos.z - at[2]).abs() > max_dz {
                continue;
            }
            let d = ((m.pos.x - at[0]).powi(2) + (m.pos.y - at[1]).powi(2)).sqrt();
            if d <= max_xy && best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, m.label.replace('_', " ").replace('`', "'")));
            }
        }
    }
    best.map(|(_, l)| l)
}

/// why: "Phinigel_Autropos_(Raid)" and "Phinigel Autropos" are one name
fn fold_label(label: &str) -> String {
    let base = label.split("_(").next().unwrap_or(label);
    let base = base.split(" (").next().unwrap_or(base);
    base.chars()
        .map(|c| match c {
            '_' => ' ',
            '`' => '\'',
            c => c.to_ascii_lowercase(),
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// why: strips a numbered sibling suffix, e.g. "befallen_1" -> "befallen"
fn zone_stem(file_stem: &str) -> &str {
    match file_stem.rsplit_once('_') {
        Some((base, suffix))
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => file_stem,
    }
}

/// why: distinct zones with a map file; numbered siblings collapse to one entry
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

/// why: every zone across base game + every pack, deduped, for the
/// zone-first picker -- `list_zone_versions` then tells which source(s) cover it
pub fn list_all_zone_names(base_dir: &Path) -> Vec<String> {
    let mut zones = list_zone_names(base_dir, None);
    for pack in list_map_packs(base_dir) {
        zones.extend(list_zone_names(base_dir, Some(&pack)));
    }
    zones.sort();
    zones.dedup();
    zones
}

/// why: every source covering `zone`; None=base game, Some(pack)=community
/// pack, base sorts first. Drives the "available versions" picker.
pub fn list_zone_versions(base_dir: &Path, zone: &str) -> Vec<Option<String>> {
    let mut out = Vec::new();
    if list_zone_names(base_dir, None).iter().any(|z| z == zone) {
        out.push(None);
    }
    for pack in list_map_packs(base_dir) {
        if list_zone_names(base_dir, Some(&pack))
            .iter()
            .any(|z| z == zone)
        {
            out.push(Some(pack));
        }
    }
    out
}

/// why: merges base + every numbered sibling; Ok(empty) if all unparseable,
/// Err only if the directory itself can't be listed
pub fn load_zone_map(
    base_dir: &Path,
    pack: Option<&str>,
    zone: &str,
) -> std::io::Result<ParsedZoneMap> {
    let zone = safe_component(zone).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a plain zone name: {zone}"),
        )
    })?;
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
        // why: lossy, not strict-UTF8 -- a community map file with a
        // Latin-1 label (or an editor's stray bytes) used to be
        // SILENTLY skipped here, dropping the whole layer: "maps not
        // displaying" with no error anywhere. A replacement char in one
        // label beats losing the geometry.
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
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

    /// why: real lines, mixed decimal/bare-integer case a naive parser could drop
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

    /// why: real line, labeled marker with double-spaced fields
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
        assert_eq!(fold_label("Phinigel_Autropos_(Raid)"), "phinigel autropos");
        assert_eq!(
            fold_label("Estrella_of_Gloomwater"),
            "estrella of gloomwater"
        );
        assert_eq!(fold_label("a_fierce_impaler_(Hunter)"), "a fierce impaler");
        assert_eq!(fold_label("to_Dagnor`s_Cauldron"), "to dagnor's cauldron");
        assert_eq!(zone_stem("befallen"), "befallen");
        // why: a zone name ending in digits must not be mistaken for a numbered sibling
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
        let dir =
            std::env::temp_dir().join(format!("eqlp-mapsdata-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("maps")).unwrap();
        dir
    }

    /// why: base+pack -> two versions, pack-only -> one, neither -> none
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
