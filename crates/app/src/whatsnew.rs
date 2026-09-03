//! why: "a what's new page when a user updates, so they can quickly read
//! what's new since they last updated." CHANGELOG.md is embedded at build
//! time; the last version the user acknowledged lives in preferences;
//! the sections between that and the running version are the page.

use serde::Serialize;

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

#[derive(Debug, Clone, Serialize)]
pub struct ChangelogSection {
    pub version: String,
    pub date: String,
    /// why: the section's markdown as written -- headings and bullets;
    /// the page renders that small subset itself
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhatsNewDto {
    pub current: String,
    /// why: None on a fresh install -- nothing to catch up on
    pub last_seen: Option<String>,
    /// why: newest first, only the versions newer than last_seen and no
    /// newer than current; empty when there's nothing to show
    pub sections: Vec<ChangelogSection>,
}

/// why: "## 2026-09-02 (0.15.0)" headers split the file
pub fn sections() -> Vec<ChangelogSection> {
    let mut out: Vec<ChangelogSection> = Vec::new();
    let mut cur: Option<ChangelogSection> = None;
    for line in CHANGELOG.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            if let Some(s) = cur.take() {
                out.push(s);
            }
            let (date, ver) = match rest.split_once(" (") {
                Some((d, v)) => (
                    d.trim().to_string(),
                    v.trim_end_matches(')').trim().to_string(),
                ),
                None => (String::new(), rest.trim().to_string()),
            };
            cur = Some(ChangelogSection {
                version: ver,
                date,
                body: String::new(),
            });
        } else if let Some(s) = cur.as_mut() {
            s.body.push_str(line);
            s.body.push('\n');
        }
    }
    if let Some(s) = cur.take() {
        out.push(s);
    }
    for s in &mut out {
        s.body = s.body.trim().to_string();
    }
    out
}

fn parse_version(v: &str) -> (u64, u64, u64) {
    let mut it = v.trim().split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

/// why: the sections a user on `last_seen` has not read, up to `current`
pub fn since(current: &str, last_seen: Option<&str>) -> Vec<ChangelogSection> {
    let cur = parse_version(current);
    let seen = last_seen.map(parse_version);
    sections()
        .into_iter()
        .filter(|s| {
            let v = parse_version(&s.version);
            v <= cur && seen.is_some_and(|l| v > l)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_changelog_splits_into_versioned_sections() {
        let all = sections();
        assert!(all.len() > 3);
        assert!(all
            .iter()
            .any(|s| s.version == "0.15.0" && s.date == "2026-09-02"));
        let s = all.iter().find(|s| s.version == "0.15.0").unwrap();
        assert!(s.body.contains("### "));
    }

    #[test]
    fn since_gives_only_the_unread_versions() {
        let v = since("0.15.0", Some("0.13.0"));
        let vers: Vec<&str> = v.iter().map(|s| s.version.as_str()).collect();
        assert_eq!(vers, vec!["0.15.0", "0.14.0"]);
        assert!(since("0.15.0", Some("0.15.0")).is_empty());
        assert!(
            since("0.15.0", None).is_empty(),
            "a fresh install has nothing to catch up on"
        );
    }
}
