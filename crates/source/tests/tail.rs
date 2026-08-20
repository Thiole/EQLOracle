//! Tests for `tail`. Kept out of the production module by
//! convention: src/ contains shipping code only.

use eqlp_source::tail::{identity_from_filename, newest_log_in, Tail, TailEvent};
use std::path::{Path, PathBuf};

use std::io::Write;

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("eqlp-tail-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn collect(t: &mut Tail) -> (TailEvent, Vec<u8>) {
    let mut got = Vec::new();
    let ev = t.poll(|b| got.extend_from_slice(b));
    (ev, got)
}

#[test]
fn missing_file_is_not_an_error_and_recovers() {
    let d = tmpdir("missing");
    let p = d.join("eqlog_A_b.txt");
    let mut t = Tail::at_end(&p);
    assert_eq!(collect(&mut t).0, TailEvent::Missing);

    std::fs::write(&p, b"hello\n").unwrap();
    let (ev, got) = collect(&mut t);
    assert!(matches!(ev, TailEvent::Grew(6)), "{ev:?}");
    assert_eq!(got, b"hello\n");
}

#[test]
fn appends_are_picked_up_incrementally() {
    let d = tmpdir("append");
    let p = d.join("eqlog_A_b.txt");
    std::fs::write(&p, b"old content\n").unwrap();

    // at_end must not replay history
    let mut t = Tail::at_end(&p);
    assert_eq!(collect(&mut t).0, TailEvent::Idle);

    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(b"line one\n").unwrap();
    f.flush().unwrap();
    assert_eq!(collect(&mut t).1, b"line one\n");

    f.write_all(b"line two\n").unwrap();
    f.flush().unwrap();
    assert_eq!(collect(&mut t).1, b"line two\n");
    assert_eq!(collect(&mut t).0, TailEvent::Idle);
}

#[test]
fn from_start_replays_history_then_follows() {
    let d = tmpdir("fromstart");
    let p = d.join("eqlog_A_b.txt");
    std::fs::write(&p, b"one\ntwo\n").unwrap();
    let mut t = Tail::from_start(&p);
    assert_eq!(collect(&mut t).1, b"one\ntwo\n");

    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    f.write_all(b"three\n").unwrap();
    f.flush().unwrap();
    assert_eq!(collect(&mut t).1, b"three\n");
}

#[test]
fn truncation_rewinds_instead_of_seeking_past_the_end() {
    let d = tmpdir("trunc");
    let p = d.join("eqlog_A_b.txt");
    std::fs::write(&p, b"aaaaaaaaaa\n").unwrap();
    let mut t = Tail::from_start(&p);
    assert_eq!(collect(&mut t).1.len(), 11);

    std::fs::write(&p, b"b\n").unwrap(); // shorter
    let (ev, got) = collect(&mut t);
    assert!(
        matches!(ev, TailEvent::Truncated | TailEvent::Replaced),
        "{ev:?}"
    );
    assert_eq!(
        got, b"b\n",
        "must re-read from the start, not from a stale offset"
    );
}

/// A partial write must never surface as a line. This is the property that
/// keeps a live tail from emitting half a damage number.
#[test]
fn byte_at_a_time_writes_never_yield_a_partial_line() {
    use eqlp_core::frame::Framer;
    let d = tmpdir("partial");
    let p = d.join("eqlog_A_b.txt");
    std::fs::write(&p, b"").unwrap();
    let mut t = Tail::from_start(&p);
    let _ = collect(&mut t);

    let payload = b"[Wed Aug 06 21:14:33 2025] You slash a rat for 12 points of damage.\n";
    let mut fr = Framer::default();
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();

    for b in payload.iter() {
        f.write_all(&[*b]).unwrap();
        f.flush().unwrap();
        t.poll(|chunk| fr.push(chunk, |l| lines.push(l.to_vec())));
        // Until the final newline arrives, nothing may be emitted.
        if *b != b'\n' {
            assert!(lines.is_empty(), "emitted a partial line");
        }
    }
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], &payload[..payload.len() - 1]);
}

#[test]
fn newest_log_wins_and_identity_parses() {
    let d = tmpdir("newest");
    std::fs::write(d.join("eqlog_Older_rivervale.txt"), b"x").unwrap();
    std::fs::write(d.join("notalog.txt"), b"x").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(d.join("eqlog_Manipulator_rivervale.txt"), b"x").unwrap();

    let got = newest_log_in(&d).unwrap();
    assert_eq!(got.file_name().unwrap(), "eqlog_Manipulator_rivervale.txt");
    assert_eq!(
        identity_from_filename(&got),
        Some(("Manipulator".into(), "rivervale".into()))
    );
    // Server names containing an underscore must survive.
    assert_eq!(
        identity_from_filename(Path::new("eqlog_Bob_test_server.txt")),
        Some(("Bob".into(), "test_server".into()))
    );
    assert_eq!(identity_from_filename(Path::new("random.txt")), None);
}
