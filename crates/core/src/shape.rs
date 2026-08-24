//! why: templates a line's variable parts, clusters into a rule backlog
//!
//! Design notes: `docs/design/parsing.md`

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShapeMode {
    /// why: digit runs only, keeps every proper noun
    Digits,
    /// why: also collapses names, but keeps the sentence-initial actor
    DigitsAndNames,
    /// why: collapses every capitalised word, actor-agnostic template
    #[default]
    Aggressive,
}

/// why: without this, "You"/"Your" collapse and self-vs-other is lost
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

/// why: bridges "Footman of V`Zher" into one name, not three tokens
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

/// why: reused token buffer keeps per-line shaping allocation-free
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
                    // why: absorbs a same-class run bridged by connectives
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
                        // why: a leading bracket/quote is a boundary too
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
