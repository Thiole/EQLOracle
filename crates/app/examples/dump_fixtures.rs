//! why: dumps real command responses for the UI's mock IPC harness
//! input: fixtures/reference-slice.log
//! output: ui/tests/fixtures/reference-slice.json
//! run: cargo run -p eqlp-app --example dump_fixtures

use eqlp_app::ingest::{backfill_lines, framed_lines, Ingest};
use eqlp_app::parser::build_engine;
use eqlp_app::{
    aadata, character, combat, debugview, gearplanner, history, inventory, monsters, npcdata,
    progression, spelldata, spelleffect, zonedata,
};
use serde_json::{json, Map, Value};
use std::path::Path;

fn main() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let log_path = repo_root.join("fixtures/reference-slice.log");
    let raw = std::fs::read(&log_path)
        .unwrap_or_else(|e| panic!("couldn't read {}: {e}", log_path.display()));

    let engine = build_engine().expect("pack builds");
    let mut ing = Ingest::default();
    let lines = framed_lines(&raw);
    // All history, `Instant` speed -- this is a startup backfill, not a
    // live tail, so nothing here should ever end up `live` (see `route`'s
    // own doc on why that gate matters: it's what keeps `recent`/
    // `pending_notifications` from firing for a decade of replayed
    // history).
    backfill_lines(&mut ing, &engine, &lines, 8);

    let mut out: Map<String, Value> = Map::new();

    // ---- get_status: hand-built, no AppHandle needed for a static status ----
    out.insert(
        "get_status".to_string(),
        json!({ "": {
            "configured": true,
            "status": {
                "log_dir": "/fixtures",
                "file": "eqlog_Manipulator_rivervale.txt",
                "character": "Manipulator",
                "server": "rivervale",
                "watching": true,
                "tail_status": "watching",
                "backfilling": false,
                "pets_attributed": 0,
            },
            "counts": ing.counts,
        }}),
    );

    // ---- list_zone_visits ----
    let visits = combat::list_zone_visits(&ing);
    out.insert("list_zone_visits".to_string(), json!({ "": visits }));

    // ---- list_encounters: aggregate plus every real zone visit ----
    // why: keyed offset=null&limit=null -- stores/combat.ts always fetches
    // the whole list now (rendering, not fetching, is what actually chokes
    // on a huge list; see Combat.svelte's own row virtualization). Also two
    // small-limit slices of the same real 340-encounter aggregate (offset=
    // 0/limit=50, offset=50/limit=50), proving the backend's own optional
    // windowing genuinely slices -- not just that the plumbing carries *a*
    // value -- the second page must be 50 different, real, older fights,
    // not an empty/repeated one. Nothing calls those two today; kept as
    // fixture coverage for combat::list_encounters' own offset/limit path.
    let mut encounters_by_visit: Map<String, Value> = Map::new();
    encounters_by_visit.insert(
        "zoneVisit=null&offset=null&limit=null".to_string(),
        json!(combat::list_encounters(&ing, None, 0, usize::MAX)),
    );
    encounters_by_visit.insert(
        "zoneVisit=null&offset=0&limit=50".to_string(),
        json!(combat::list_encounters(&ing, None, 0, 50)),
    );
    encounters_by_visit.insert(
        "zoneVisit=null&offset=50&limit=50".to_string(),
        json!(combat::list_encounters(&ing, None, 50, 50)),
    );
    for v in &visits {
        if let Some(idx) = v.index {
            let key = format!("zoneVisit={idx}&offset=null&limit=null");
            encounters_by_visit.insert(
                key,
                json!(combat::list_encounters(
                    &ing,
                    Some(idx as i64),
                    0,
                    usize::MAX
                )),
            );
        }
    }
    out.insert(
        "list_encounters".to_string(),
        Value::Object(encounters_by_visit),
    );

    // why: richest real fight, best exercise for allies/timeline rendering
    // -- the whole real list, not a page of it (a windowed slice could
    // miss the actual richest fight if it sits past the first page).
    let all_encounters = combat::list_encounters(&ing, None, 0, usize::MAX);
    let richest = all_encounters
        .iter()
        .max_by_key(|e| e.total_damage)
        .expect("reference log has at least one real encounter");
    let richest_id = richest.id;

    let mut summary_by_selection: Map<String, Value> = Map::new();
    summary_by_selection.insert(
        "zoneVisit=null&encounterId=null".to_string(),
        json!(combat::summarize(&ing, None, None, None)),
    );
    summary_by_selection.insert(
        format!("zoneVisit=null&encounterId={richest_id}"),
        json!(combat::summarize(&ing, None, Some(richest_id), None)),
    );
    out.insert(
        "get_combat_summary".to_string(),
        Value::Object(summary_by_selection),
    );

    let mut allies_by_selection: Map<String, Value> = Map::new();
    allies_by_selection.insert(
        "zoneVisit=null&encounterId=null".to_string(),
        json!(combat::list_allies(&ing, None, None)),
    );
    allies_by_selection.insert(
        format!("zoneVisit=null&encounterId={richest_id}"),
        json!(combat::list_allies(&ing, None, Some(richest_id))),
    );
    out.insert(
        "list_allies".to_string(),
        Value::Object(allies_by_selection),
    );

    let timeline = combat::fight_timeline(&ing, richest_id);
    out.insert(
        "get_fight_timeline".to_string(),
        json!({ format!("encounterId={richest_id}"): timeline }),
    );

    // why: real bucket timestamps for the click-to-scrub interaction
    if let Some(t) = &timeline {
        let mut state_by_ts: Map<String, Value> = Map::new();
        let sample_count = 5.min(t.buckets.len());
        let step = (t.buckets.len() as f64 / sample_count as f64).max(1.0);
        for i in 0..sample_count {
            let idx = ((i as f64 * step) as usize).min(t.buckets.len() - 1);
            let ts = t.buckets[idx];
            let key = format!("encounterId={richest_id}&tsMs={ts}");
            state_by_ts.insert(key, json!(combat::fight_state_at(&ing, richest_id, ts)));
        }
        out.insert("get_fight_state_at".to_string(), Value::Object(state_by_ts));
    }

    // ---- Character module: pure Ingest functions, no AppHandle needed ----
    // "You" is the log's own first-person name for the player, not a placeholder.
    let cfgs = combat::class_configurations(&ing, "You");
    // why: one fixture entry per real configuration, for the Debug > Character drill-down
    let mut visits_by_cfg = serde_json::Map::new();
    for cfg in &cfgs.configurations {
        let key = format!("name=You&classes={}", cfg.classes.join(","));
        let visits = combat::zone_visits_for_configuration(&ing, "You", &cfg.classes);
        visits_by_cfg.insert(key, json!(visits));
    }
    out.insert(
        "get_configuration_zone_visits".to_string(),
        Value::Object(visits_by_cfg),
    );
    out.insert(
        "get_class_configurations".to_string(),
        json!({ "name=You": cfgs }),
    );
    out.insert(
        "get_current_level".to_string(),
        json!({ "": ing.levels.latest() }),
    );
    // why: filesystem-dependent, not log-derivable -- fixture the "none found" case
    out.insert(
        "find_existing_inventory_dump".to_string(),
        json!({ "": Value::Null }),
    );
    let default_classes = gearplanner::default_classes(&ing, "You");
    out.insert(
        "get_default_gear_classes".to_string(),
        json!({ "name=You": default_classes }),
    );
    out.insert(
        "get_aa_log".to_string(),
        json!({ "": progression::aa_log(&ing) }),
    );
    out.insert(
        "get_spellbook".to_string(),
        json!({ "": progression::spellbook(&ing) }),
    );
    out.insert("list_aa".to_string(), json!({ "": aadata::aas() }));

    // why: one real representative race/classes/levels combo, not exhaustive
    let (classes, levels): (Vec<String>, Vec<u8>) = match cfgs.configurations.first() {
        Some(top) => {
            let hi = top.level_range.map(|(_, hi)| hi).unwrap_or(10);
            (
                top.classes.clone(),
                top.classes.iter().map(|_| hi).collect(),
            )
        }
        None => (
            vec![
                "Warrior".to_string(),
                "Cleric".to_string(),
                "Wizard".to_string(),
            ],
            vec![20, 20, 20],
        ),
    };
    let race = "Human".to_string();
    let level = ing.levels.latest();

    // why: real dump file, parse/resolve_inventory are pure
    let inv_file = "Manipulator_rivervale-Inventory.txt";
    let inv_path = repo_root.join(inv_file);
    let parsed_inv = inv_path
        .exists()
        .then(|| inventory::parse(&inv_path).expect("real inventory dump parses"));
    let equipped_names: Option<std::collections::HashMap<String, String>> =
        parsed_inv.as_ref().map(|p| {
            p.equipped
                .iter()
                .map(|(k, v)| (k.clone(), v.name.clone()))
                .collect()
        });

    // why: the same "sum whatever's equipped" gearStatTotals does in
    // stores/character.ts -- reusing resolve_inventory's own ItemDtos
    // (already tier-scaled) rather than re-deriving scale_stat here,
    // same reason that function reads ScoredItemDto.stats instead of the
    // raw catalog. Every slot here really is equipped (a real dump), so
    // this only exercises that function's equipped-item half.
    let gear: std::collections::HashMap<String, f64> = parsed_inv
        .as_ref()
        .map(|p| {
            let dump = gearplanner::resolve_inventory(p, Some(&ing.exaltation_procs));
            let primary_is_2h = dump
                .resolved
                .get("PRIMARY")
                .and_then(|it| it.skill.as_deref())
                .is_some_and(|s| s.starts_with("2H"));
            let mut totals = std::collections::HashMap::new();
            for (slot, it) in &dump.resolved {
                if slot == "SECONDARY" && primary_is_2h {
                    continue;
                }
                for (stat, val) in &it.stats {
                    *totals.entry(stat.clone()).or_insert(0.0) += val;
                }
            }
            totals
        })
        .unwrap_or_default();

    let est = character::estimate(&race, &classes, &levels, &gear);
    let est_key = format!(
        "race=Human&classes={}&classLevels={}",
        classes.join(","),
        levels
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    out.insert(
        "get_character_estimate".to_string(),
        json!({ est_key: est }),
    );

    let recs = gearplanner::recommend(
        &classes,
        Some(&race),
        None,
        50,
        None,
        level,
        equipped_names.as_ref(),
        parsed_inv.as_ref().map(|p| &p.owned),
        parsed_inv.as_ref().map(|p| &p.owned_tier),
    );
    let recs_key = format!(
        "classes={}&race=Human&level={}",
        classes.join(","),
        level
            .map(|l| l.to_string())
            .unwrap_or_else(|| "null".to_string())
    );
    out.insert(
        "get_gear_recommendations".to_string(),
        json!({ recs_key: recs }),
    );

    let weights = gearplanner::weights_for(&classes, level);
    let weights_key = format!(
        "classes={}&level={}",
        classes.join(","),
        level
            .map(|l| l.to_string())
            .unwrap_or_else(|| "null".to_string())
    );
    out.insert(
        "get_gear_weights".to_string(),
        json!({ weights_key: weights }),
    );

    // why: pending_history already has real records, no AppHandle needed
    let history_target = &richest.target;
    for confirmed_only in [false, true] {
        let records =
            history::mob_history_view(ing.pending_history.clone(), history_target, confirmed_only);
        let loadouts = history::by_loadout(&history::filter_for_target(
            if confirmed_only {
                history::only_confirmed_kills(ing.pending_history.clone())
            } else {
                ing.pending_history.clone()
            },
            history_target,
        ));
        let key = format!("target={history_target}&confirmedOnly={confirmed_only}");
        out.entry("get_mob_history".to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(key.clone(), json!(records));
        out.entry("get_loadout_summary".to_string())
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap()
            .insert(key, json!(loadouts));
    }

    if let Some(parsed) = &parsed_inv {
        let dump = gearplanner::resolve_inventory(parsed, Some(&ing.exaltation_procs));
        // why: doll tier picker's "what if I upgrade this" preview -- real
        // equipped items, re-derived at a tier above what's owned. FACE is
        // the one with a locked-at-its-own-tier exaltation socket (a Worn
        // effect gated at +3, owned only at +1) -- the case that actually
        // exercises exalt_slots' unlock behavior end to end.
        let mut item_at_tier_table = serde_json::Map::new();
        if let Some(neck) = dump.resolved.get("NECK") {
            item_at_tier_table.insert(
                format!("id={}&tier=9", neck.id),
                json!(gearplanner::item_at_tier(&neck.id, 9)),
            );
        }
        if let Some(face) = dump.resolved.get("FACE") {
            item_at_tier_table.insert(
                format!("id={}&tier=3", face.id),
                json!(gearplanner::item_at_tier(&face.id, 3)),
            );
        }
        // why: the exaltation-fill picker's own tier bump -- Brass Ring
        // starts at tier 0 in the catalog, every exalt socket but
        // Ornamentation needs at least +1 to unlock.
        item_at_tier_table.insert(
            "id=Brass_Ring&tier=5".to_string(),
            json!(gearplanner::item_at_tier("Brass_Ring", 5)),
        );
        out.insert(
            "get_item_at_tier".to_string(),
            Value::Object(item_at_tier_table),
        );
        out.insert(
            "get_inventory_dump".to_string(),
            json!({ format!("file={inv_file}"): dump }),
        );
    }

    // why: real Ingest state, no AppHandle needed for either debug view
    out.insert(
        "list_debug_encounters".to_string(),
        json!({ "limit=null": debugview::list_debug_encounters(&ing, 100) }),
    );
    out.insert(
        "get_unmatched_coverage".to_string(),
        json!({ "top=null": debugview::unmatched_coverage(&ing, 100) }),
    );

    // ---- Game Data module: the wiki catalogs are static, one call each ----
    out.insert("list_zones".to_string(), json!({ "": zonedata::zones() }));
    out.insert("list_npcs".to_string(), json!({ "": npcdata::npcs() }));
    out.insert(
        "get_mob_aliases".to_string(),
        json!({ "": eqlp_app::mobalias::all() }),
    );
    out.insert(
        "list_spells".to_string(),
        json!({ "": spelldata::spells() }),
    );
    out.insert(
        "list_spell_effects".to_string(),
        json!({ "": spelleffect::all_effects() }),
    );
    // why: GameData's own refreshItems always resolves a concrete era
    // (never sends a literal null -- see stores/settings.ts's
    // effectiveEra), so the fixture key matches what it actually sends:
    // the real CURRENT_ERA default, not the bare-None browsing shape
    // other callers (list_items' own unit tests) still exercise directly.
    // why: two era ceilings, not just the default -- proves the Settings
    // module's era picker actually narrows the Items tab (a real, lower
    // item count under "Classic Era" than under the default "Sky Era"),
    // not just that the plumbing sends *a* value.
    let mut items_table = serde_json::Map::new();
    for era in [gearplanner::CURRENT_ERA, "Classic Era", "All"] {
        items_table.insert(
            format!("classes=&slot=null&maxEra={era}"),
            json!(gearplanner::list_items(&[], None, Some(era), None, None)),
        );
    }
    out.insert("list_gear_items".to_string(), Value::Object(items_table));
    out.insert(
        "get_era_options".to_string(),
        json!({ "": {
            "eras": gearplanner::ERA_ORDER,
            "current": gearplanner::CURRENT_ERA,
        } }),
    );
    out.insert(
        "get_preferences".to_string(),
        json!({ "": { "volume": 100, "era": Value::Null } }),
    );
    // why: real catalog items ("Brass Ring", "Adamantite Band" -- both
    // confirmed present, and exercised the same way in gearplanner.rs's
    // own exalt_candidate_tests) -- the exaltation picker's own round
    // trip, not the equipped dump's items, so this doesn't depend on
    // whichever slot the reference dump happens to carry. `classes`
    // matches the same Warrior/Cleric/Wizard trio every other fixture in
    // this file's own fallback uses -- GearPanel always sends the real
    // active trio, never an empty filter, so that's the realistic key.
    let exalt_classes = classes.clone();
    out.insert(
        "get_exalt_candidates".to_string(),
        json!({
            format!(
                "id=Brass_Ring&socketKey=focus&other=&classes={}&maxEra={}",
                exalt_classes.join(","),
                gearplanner::CURRENT_ERA,
            ):
                gearplanner::exalt_candidates("Brass_Ring", "focus", &std::collections::HashMap::new(), &exalt_classes, Some(gearplanner::CURRENT_ERA))
        }),
    );
    {
        let mut assignments = std::collections::HashMap::new();
        assignments.insert("focus".to_string(), "Adamantite_Band".to_string());
        out.insert(
            "get_item_with_exalts".to_string(),
            json!({
                "id=Brass_Ring&tier=5&exalts=focus:Adamantite_Band":
                    gearplanner::item_with_exalts("Brass_Ring", 5, &assignments)
            }),
        );
    }
    // why: real session evidence for a page's own "your history" section --
    // a real looted item and a real fought mob, both confirmed present in
    // fixtures/reference-slice.log, not fixtures for a name that happens
    // to look plausible.
    out.insert(
        "get_item_loot_history".to_string(),
        json!({ "item=Fragile Pet's Skull": monsters::item_loot_history(&ing, "Fragile Pet's Skull") }),
    );
    out.insert(
        "get_mob_stats".to_string(),
        json!({ format!("mobName={history_target}"): monsters::mob_stats(&ing, history_target) }),
    );
    // why: a zone/NPC page's own "your parsed encounters" section --
    // "Blackburrow" is a real zone id (packs/zones.json) real fights in
    // this log actually resolve to; history_target/richest_id are the
    // same real richest-encounter facts every other fixture above uses.
    out.insert(
        "list_zone_encounters".to_string(),
        json!({ "zoneId=Blackburrow&limit=null": combat::list_zone_encounters(&ing, "Blackburrow", 30) }),
    );
    out.insert(
        "list_mob_encounters".to_string(),
        json!({ format!("mobName={history_target}&limit=null"): combat::list_mob_encounters(&ing, history_target, 30) }),
    );
    out.insert(
        "get_encounter_detail".to_string(),
        json!({ format!("encounterId={richest_id}"): combat::encounter_detail(&ing, richest_id) }),
    );

    let dest = repo_root.join("ui/tests/fixtures/reference-slice.json");
    std::fs::create_dir_all(dest.parent().unwrap()).expect("create fixtures dir");
    let json_text = serde_json::to_string_pretty(&Value::Object(out)).expect("serialize fixtures");
    std::fs::write(&dest, &json_text)
        .unwrap_or_else(|e| panic!("couldn't write {}: {e}", dest.display()));

    println!("wrote {} ({} bytes)", dest.display(), json_text.len());
    println!("zone visits: {}, encounters (aggregate): {}, richest encounter id: {richest_id} ({} damage)", visits.len(), all_encounters.len(), richest.total_damage);
}
