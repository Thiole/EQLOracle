//! Following a log file the game is actively writing. Polls rather than
//! watching, and handles growth, truncation, replacement and absence.
//!
//! Design notes: `docs/design/sources.md`

use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Identity used to detect file replacement. Falls back through inode,
/// creation time, then "assume same". See `docs/design/sources.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileId {
    Index(u64),
    Created(u64),
    Unknown,
}

fn file_id(m: &Metadata) -> FileId {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ino = m.ino();
        if ino != 0 {
            return FileId::Index(ino);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Not exposed as file_index on stable for all versions; use the
        // creation-time path, which Wine and NTFS both populate.
        let _ = m;
    }
    m.created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| FileId::Created(d.as_nanos() as u64))
        .unwrap_or(FileId::Unknown)
}

impl FileId {
    /// Conservative: only report a change when both sides are known and differ.
    fn definitely_differs(self, other: FileId) -> bool {
        match (self, other) {
            (FileId::Unknown, _) | (_, FileId::Unknown) => false,
            (a, b) => a != b,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailEvent {
    /// Bytes were appended and handed to the sink.
    Grew(u64),
    /// File shrank; we rewound to the start.
    Truncated,
    /// Path now refers to a different file; we rewound to the start.
    Replaced,
    /// Path does not currently exist. Not an error — we keep watching.
    Missing,
    /// Nothing changed this poll.
    Idle,
}

pub struct Tail {
    path: PathBuf,
    file: Option<File>,
    offset: u64,
    id: FileId,
    buf: Vec<u8>,
    pub bytes_read: u64,
    pub resets: u64,
}

impl Tail {
    /// Start at EOF: live monitoring, no history replay.
    pub fn at_end(path: impl Into<PathBuf>) -> Tail {
        Tail::new(path, true)
    }

    /// Open at the beginning: replay history, then continue live.
    pub fn from_start(path: impl Into<PathBuf>) -> Tail {
        Tail::new(path, false)
    }

    fn new(path: impl Into<PathBuf>, skip_existing: bool) -> Tail {
        let path = path.into();
        let mut t = Tail {
            path,
            file: None,
            offset: 0,
            id: FileId::Unknown,
            buf: vec![0u8; 256 * 1024],
            bytes_read: 0,
            resets: 0,
        };
        if skip_existing {
            if let Ok(m) = std::fs::metadata(&t.path) {
                t.offset = m.len();
                t.id = file_id(&m);
            }
        }
        t
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// One poll. Feeds newly appended bytes to `sink`. Non-blocking.
    pub fn poll(&mut self, mut sink: impl FnMut(&[u8])) -> TailEvent {
        let meta = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => {
                // Drop the handle so a replacement is picked up cleanly.
                self.file = None;
                return TailEvent::Missing;
            }
        };

        let id = file_id(&meta);
        let len = meta.len();

        let mut event = TailEvent::Idle;
        if self.id.definitely_differs(id) {
            self.rewind(id);
            event = TailEvent::Replaced;
        } else if len < self.offset {
            self.rewind(id);
            event = TailEvent::Truncated;
        } else if self.id == FileId::Unknown {
            self.id = id;
        }

        if len == self.offset && matches!(event, TailEvent::Idle) {
            return TailEvent::Idle;
        }

        if self.file.is_none() {
            match File::open(&self.path) {
                Ok(f) => self.file = Some(f),
                Err(_) => return TailEvent::Missing,
            }
        }

        let mut grew = 0u64;
        {
            let f = self.file.as_mut().expect("opened above");
            if f.seek(SeekFrom::Start(self.offset)).is_err() {
                self.file = None;
                return event;
            }
            loop {
                match f.read(&mut self.buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        sink(&self.buf[..n]);
                        self.offset += n as u64;
                        grew += n as u64;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        self.file = None;
                        break;
                    }
                }
            }
        }
        self.bytes_read += grew;

        match event {
            TailEvent::Idle if grew > 0 => TailEvent::Grew(grew),
            TailEvent::Idle => TailEvent::Idle,
            other => other,
        }
    }

    fn rewind(&mut self, id: FileId) {
        self.offset = 0;
        self.id = id;
        self.file = None;
        self.resets += 1;
    }
}

/// Newest `eqlog_*.txt` in `dir`. Call periodically: characters switch.
pub fn newest_log_in(dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in rd.flatten() {
        let p = e.path();
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if !name.starts_with("eqlog_") || !name.ends_with(".txt") {
            continue;
        }
        let mt = match e.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if best.as_ref().map_or(true, |(bt, _)| mt > *bt) {
            best = Some((mt, p));
        }
    }
    best.map(|(_, p)| p)
}

/// Character and server from an `eqlog_<char>_<server>.txt` filename.
pub fn identity_from_filename(path: &Path) -> Option<(String, String)> {
    let stem = path.file_stem()?.to_str()?;
    let rest = stem.strip_prefix("eqlog_")?;
    // Character names contain no underscore; server is whatever follows the
    // first one, so a server like "test_server" survives.
    let (chr, srv) = rest.split_once('_')?;
    if chr.is_empty() || srv.is_empty() {
        return None;
    }
    Some((chr.to_string(), srv.to_string()))
}
