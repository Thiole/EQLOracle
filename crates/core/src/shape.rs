//! Collapses the variable parts of a line into a template, for clustering
//! unmatched lines into a ranked rule backlog.
//!
//! Design notes: `docs/design/parsing.md`

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeMode {
    /// Collapse digit runs only. Conservative; keeps every proper noun.
    Digits,
    /// Also collapse capitalised words that are not sentence-initial.
    ///
    /// The position exception keeps the actor concrete, so `Kaeus slashes ...`
    /// and `Bouncer Krik slashes ...` stay separate. Useful when you care who
    /// is doing things.
    DigitsAndNames,
    /// Collapse every capitalised word including the first. This is the mode
    /// for discovery — it yields the line template regardless of actor.
    #[default]
    Aggressive,
}

/// Capitalised words that are grammar, not names.
///
/// Pure English function words; no game knowledge encoded here. Without this,
/// `You` and `Your` collapse to `@` and self-versus-other is destroyed.
#[inline]
fn is_function_word(w: &[u8]) -> bool {
    matches!(
        w,
        b"You"
            | b"YOU"
            | b"Your"
            | b"YOUR"
            | b"Yours"
            | b"I"
            | b"A"
            | b"An"
            | b"The"
            | b"It"
            | b"Its"
            | b"This"
            | b"That"
            | b"He"
            | b"She"
            | b"His"
            | b"Her"
            | b"Hers"
            | b"Him"
            | b"They"
            | b"Them"
            | b"Their"
            | b"We"
            | b"Our"
            | b"Us"
    )
}

/// Lowercase words that routinely sit *inside* a proper noun. Bridging these
/// merges ``Footman of V`Zher`` and `Blessing of the Squire` into one `@`.
#[inline]
fn is_connective(w: &[u8]) -> bool {
    matches!(
        w,
        b"of" | b"the" | b"de" | b"del" | b"da" | b"van" | b"von" | b"du" | b"la" | b"le"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Lit,
    Num,
    Name,
    Connective,
}

#[derive(Clone, Copy)]
struct Tok {
    lead: (u32, u32),
    core: (u32, u32),
    trail: (u32, u32),
    class: Class,
}

/// Reusable shaping state. Holding the token buffer here keeps the per-line
/// cost allocation-free when shaping a whole log.
#[derive(Default)]
pub struct Shaper {
    toks: Vec<Tok>,
}

impl Shaper {
    pub fn new() -> Self {
        Shaper {
            toks: Vec::with_capacity(48),
        }
    }

    pub fn shape_into(&mut self, body: &[u8], mode: ShapeMode, out: &mut Vec<u8>) {
        out.clear();
        self.tokenize(body, mode);
        self.emit(body, out);
    }

    fn tokenize(&mut self, body: &[u8], mode: ShapeMode) {
        self.toks.clear();
        let mut i = 0usize;
        let mut first = true;

        while i < body.len() {
            while i < body.len() && body[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= body.len() {
                break;
            }
            let start = i;
            while i < body.len() && !body[i].is_ascii_whitespace() {
                i += 1;
            }
            let tok = &body[start..i];

            // Peel surrounding punctuation so "damage." and "damage" agree.
            let cs = tok.iter().position(|c| c.is_ascii_alphanumeric());
            let ce = tok
                .iter()
                .rposition(|c| c.is_ascii_alphanumeric())
                .map(|p| p + 1);
            let (cs, ce) = match (cs, ce) {
                (Some(a), Some(b)) => (a, b),
                _ => (tok.len(), tok.len()),
            };
            let core = &tok[cs..ce];

            let class = if core.is_empty() {
                Class::Lit
            } else if core.iter().all(|c| c.is_ascii_digit()) {
                Class::Num
            } else if is_connective(core) {
                Class::Connective
            } else if matches!(mode, ShapeMode::DigitsAndNames | ShapeMode::Aggressive)
                && (!first || mode == ShapeMode::Aggressive)
                && core[0].is_ascii_uppercase()
                && core.len() > 1
                && !is_function_word(core)
            {
                Class::Name
            } else {
                Class::Lit
            };

            self.toks.push(Tok {
                lead: (start as u32, (start + cs) as u32),
                core: ((start + cs) as u32, (start + ce) as u32),
                trail: ((start + ce) as u32, i as u32),
                class,
            });
            first = false;
        }
    }

    fn emit(&self, body: &[u8], out: &mut Vec<u8>) {
        let t = &self.toks;
        let n = t.len();
        let mut i = 0usize;

        while i < n {
            if i > 0 {
                out.push(b' ');
            }
            let cur = t[i];
            match cur.class {
                Class::Name | Class::Num => {
                    let ph = if cur.class == Class::Name { b'@' } else { b'#' };
                    // Absorb a run of the same class, optionally bridged by one
                    // or more connectives. Trailing punctuation ends the run —
                    // a comma is a real clause boundary, not part of the name.
                    let mut end = i;
                    loop {
                        if !span_empty(t[end].trail) {
                            break;
                        }
                        let mut k = end + 1;
                        while k < n
                            && t[k].class == Class::Connective
                            && span_empty(t[k].trail)
                            && span_empty(t[k].lead)
                        {
                            k += 1;
                        }
                        // A leading bracket or quote is a boundary too: the
                        // "(Exaltation)" in an item proc is not part of the
                        // item name, and swallowing its "(" mangles the shape.
                        if k < n && t[k].class == cur.class && span_empty(t[k].lead) {
                            end = k;
                        } else {
                            break;
                        }
                    }
                    push(out, body, cur.lead);
                    out.push(ph);
                    push(out, body, t[end].trail);
                    i = end + 1;
                }
                _ => {
                    push(out, body, cur.lead);
                    push(out, body, cur.core);
                    push(out, body, cur.trail);
                    i += 1;
                }
            }
        }
    }
}

#[inline]
fn span_empty(s: (u32, u32)) -> bool {
    s.0 == s.1
}

#[inline]
fn push(out: &mut Vec<u8>, body: &[u8], s: (u32, u32)) {
    out.extend_from_slice(&body[s.0 as usize..s.1 as usize]);
}

/// Convenience wrapper; allocates. Prefer `Shaper` in a loop.
pub fn shape(body: &[u8], mode: ShapeMode) -> Vec<u8> {
    let mut sh = Shaper::new();
    let mut v = Vec::with_capacity(body.len());
    sh.shape_into(body, mode, &mut v);
    v
}

pub fn shape_into(body: &[u8], mode: ShapeMode, out: &mut Vec<u8>) {
    Shaper::new().shape_into(body, mode, out)
}
