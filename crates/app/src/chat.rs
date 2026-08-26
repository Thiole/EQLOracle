//! why: Social tab queries -- Guild/Party/Raid channel history and PM
//! threads. Thin DTO layer over `Ingest::chat` (`ChatLog`), which does
//! the real work of grouping by channel/partner as lines are parsed.

use crate::ingest::{ChatMessage, Ingest};
use eqlp_source::Millis;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessageDto {
    pub ts_ms: Millis,
    /// why: the real sender -- "You" for the player's own outgoing line
    pub who: String,
    pub text: String,
}

impl From<&ChatMessage> for ChatMessageDto {
    fn from(m: &ChatMessage) -> Self {
        ChatMessageDto {
            ts_ms: m.ts,
            who: m.who.clone(),
            text: m.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PmThreadDto {
    /// why: the other side of the conversation, regardless of who sent
    /// the most recent line -- see ChatChannel::Pm's own doc
    pub player: String,
    pub last_ts_ms: Millis,
    pub last_text: String,
}

pub fn guild_chat(ing: &Ingest) -> Vec<ChatMessageDto> {
    ing.chat.guild().iter().map(ChatMessageDto::from).collect()
}

pub fn party_chat(ing: &Ingest) -> Vec<ChatMessageDto> {
    ing.chat.party().iter().map(ChatMessageDto::from).collect()
}

pub fn raid_chat(ing: &Ingest) -> Vec<ChatMessageDto> {
    ing.chat.raid().iter().map(ChatMessageDto::from).collect()
}

/// why: most-recent-message-first -- the PM player list's own order
pub fn pm_threads(ing: &Ingest) -> Vec<PmThreadDto> {
    let mut out: Vec<PmThreadDto> = ing
        .chat
        .pm_threads()
        .map(|(name, last)| PmThreadDto {
            player: name.to_string(),
            last_ts_ms: last.ts,
            last_text: last.text.clone(),
        })
        .collect();
    out.sort_by_key(|t| std::cmp::Reverse(t.last_ts_ms));
    out
}

/// why: whole thread, oldest first -- empty for a never-messaged name, not an error
pub fn pm_history(ing: &Ingest, player: &str) -> Vec<ChatMessageDto> {
    ing.chat
        .pm_history(player)
        .iter()
        .map(ChatMessageDto::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::backfill_lines;
    use crate::parser::build_engine;

    fn run(lines: &[&[u8]]) -> Ingest {
        let engine = build_engine().expect("pack builds");
        let mut ing = Ingest::default();
        backfill_lines(&mut ing, &engine, lines, 1);
        ing
    }

    #[test]
    fn pm_threads_sorts_most_recent_first() {
        let ing = run(&[
            b"[Thu Jul 30 18:04:38 2026] Kaeus tells you, 'first'",
            b"[Thu Jul 30 18:05:00 2026] Opticon tells you, 'second'",
        ]);
        let threads = pm_threads(&ing);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].player, "Opticon", "most recent first");
        assert_eq!(threads[1].player, "Kaeus");
    }

    #[test]
    fn pm_history_is_oldest_first_across_both_directions() {
        let ing = run(&[
            b"[Thu Jul 30 18:04:38 2026] Kaeus tells you, 'busy right now'",
            b"[Thu Jul 30 22:47:34 2026] You told Kaeus, 'no worries'",
        ]);
        let history = pm_history(&ing, "Kaeus");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].text, "busy right now");
        assert_eq!(history[1].text, "no worries");
    }

    #[test]
    fn pm_history_for_an_unknown_player_is_empty_not_an_error() {
        let ing = run(&[]);
        assert!(pm_history(&ing, "Nobody").is_empty());
    }
}
