// why: the scrape keeps a handful of fields as raw MediaWiki wikitext
// (`[[Giant Rats | Giant Rat]]`, `[[Skill Conjuration | Conjuration]]`,
// `[[Talk:A Rabid Grizzly|See Talk for Qeynos Hills locs]]`) rather than
// resolving the link at scrape time -- the display text after the `|` is
// what a reader wants, not the raw page-name half duplicated alongside
// it. A plain `[[Name]]` (no pipe) just loses its brackets.

/** `"[[Skill Conjuration | Conjuration]]"` -> `"Conjuration"`.
 * `"[[Human]]"` -> `"Human"`. Plain text with no wiki markup passes
 * through unchanged. */
export function wikiLinkText(raw: string): string {
  const stripped = raw.replace(/\[\[|\]\]/g, '');
  const display = stripped.includes('|') ? (stripped.split('|').pop() ?? stripped) : stripped;
  return display.trim();
}
