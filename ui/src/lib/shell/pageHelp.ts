// why: one sentence per module, so every page carries a "?" without ten
// separate edits -- App.svelte renders one HelpTip over <main> and looks
// the text up by the active module. A page whose own sections already
// explain themselves still gets the top-level "what am I looking at".
export const PAGE_HELP: Record<string, string> = {
  overview:
    "What is going on right now: your character, this session's rates, what you have been killing, and which buffs you are missing. Each card links out to the module that actually owns the answer.",
  combat:
    'Every fight your log has parsed, by zone and by mob. Pick a fight to see the ally breakdown, the abilities behind it, and past parses against the same target for comparison.',
  social:
    'Who you have talked to and grouped with, read straight from the log -- tells, group chat, and the guild and channel traffic around you.',
  character:
    'Your character as the log proves it, plus the parts it never states. Race and hand-set levels are yours to enter; classes, levels and gear are inferred from what you actually did.',
  endgame:
    'Raiding, Sky, and Epic progress. Materials are tracked against what you have looted, and anything out of era for the current server is left out rather than shown as farmable.',
  tradeskill:
    'Every combine your log recorded, by skill and by recipe -- successes, failures, and what the attempts cost you.',
  gamedata:
    'The whole scraped catalog: zones, items, NPCs, spells and AAs. Capped to the era set in Settings, so what you see is what exists on the server right now.',
  maps: 'Zone maps with a live "you are here", plus GPS routing between zones that accounts for your own ports and their levels. Destinations are limited to zones the server actually has.',
  overlay:
    'The in-game overlay: separate always-on-top windows for DPS, cooldowns, crowd control, drops, session rates and group buffs. Each has its own on/off, layout, transparency and position, and all of it is remembered between launches.',
  settings: 'App-wide preferences -- era ceiling, theme, update channel, and how much of your character is remembered between launches.',
  info: 'Version, changelog, and where this app keeps its files.',
};
