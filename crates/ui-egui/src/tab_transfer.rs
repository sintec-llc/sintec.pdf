//! Sintec.PDF: a tab dragged onto the tab strip of a window in another Sintec.PDF process joins
//! that window.
//!
//! Since a torn-off tab opens in a process of its own ([`crate::tear_off`]), windows of other
//! processes are invisible to [`crate::WindowGeometry`]. So each process publishes where its tab
//! strips are on the screen, and takes documents handed to it:
//!
//! - **Registry.** One small JSON file per process (`<pid>.json`) in a per-user folder beside the
//!   crash-recovery one ([`Registry::default_dir`]): the pid, a loopback port, a random token and,
//!   per window, the strip's and its tabs' rectangles in physical screen pixels. A background
//!   thread rewrites it every [`HEARTBEAT`] and removes it when the process ends, so a file not
//!   rewritten for [`STALE_SECS`] belongs to a process that died and is ignored (and deleted once
//!   it is very old). At most [`MAX_PEERS`] files are read, each at most [`MAX_ENTRY_BYTES`].
//! - **Hand-off.** The window a tab was dragged from connects to `127.0.0.1:<port>` and sends one
//!   JSON line ([`request_line`]): the token, what to open ([`HandOff`]: the file, or the
//!   recovery entry holding unsaved work, as for a torn-off tab), the target window, the gap
//!   (the index of the tab it goes before) and the page. The target checks every field
//!   ([`parse_request`]), opens it on its UI thread at that gap and answers `{"ok":true}`, or
//!   `{"ok":false,"reason":…}`. Only then does the source close its tab; on any failure (a
//!   refusal, a dead peer, a timeout) the tab stays and a notice says why.
//!
//! The folder is the user's own, so only the user's own processes learn the token; the token
//! keeps other local users from handing documents to this window over the loopback port.
//! Every wait is bounded: [`CONNECT_TIMEOUT`] to connect, [`REPLY_TIMEOUT`] for the answer, and
//! the target gives its UI [`TAKE_WAIT`] to take the document before refusing. If the source gives
//! up while the target still opens it, the document is open in both windows: never lost.
//!
//! On the web there are no other processes: the transport is native only.

#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::io::Read;
use std::path::{Path, PathBuf};

use egui::{Pos2, Rect, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::tear_off::{HandOff, valid_recovery_key};

/// A registry file not rewritten for this long (seconds) belongs to a process that has gone.
pub const STALE_SECS: u64 = 10;
/// Registry files this old (seconds) are deleted when the registry is read.
const FORGET_SECS: u64 = 300;
/// How often a process rewrites its registry file, so it stays fresh while the app is idle.
#[cfg(not(target_arch = "wasm32"))]
pub const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(2);
/// Most other processes considered.
pub const MAX_PEERS: usize = 32;
/// Most files looked at in the registry folder.
const MAX_SCAN: usize = 256;
/// Longest registry file read.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024;
/// Most windows one process publishes.
pub const MAX_STRIPS: usize = 16;
/// Most tabs one strip publishes (and the largest gap index accepted).
pub const MAX_TABS: usize = 512;
/// Longest request line, in bytes.
pub const MAX_MESSAGE: usize = 16 * 1024;
/// Longest answer line, in bytes.
const MAX_REPLY: u64 = 1024;
/// Furthest a published coordinate may be from the origin, in pixels.
pub const MAX_COORD: f32 = 1.0e6;
/// Longest file path accepted, in bytes.
const MAX_PATH_LEN: usize = 4096;
/// Highest page index accepted.
const MAX_PAGE: u64 = 10_000_000;
/// The token: 128 bits, hex.
const TOKEN_LEN: usize = 32;
/// Unanswered documents waiting for the UI, at most.
#[cfg(not(target_arch = "wasm32"))]
const MAX_QUEUE: usize = 4;
/// How long the source waits to connect.
#[cfg(not(target_arch = "wasm32"))]
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
/// How long the source waits for the answer: longer than the target waits for its UI.
#[cfg(not(target_arch = "wasm32"))]
pub const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
/// How long the target waits for its UI to take a document before refusing it.
#[cfg(not(target_arch = "wasm32"))]
pub const TAKE_WAIT: std::time::Duration = std::time::Duration::from_secs(4);
/// How long either side waits to read or write the request.
#[cfg(not(target_arch = "wasm32"))]
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long the registry read during a drag is reused.
#[cfg(not(target_arch = "wasm32"))]
const PEERS_REFRESH: std::time::Duration = std::time::Duration::from_millis(250);

/// Why a hand-off failed, as sent between processes (the source says it in its own words,
/// [`reason_text`]).
pub mod reason {
    /// The target is busy (a dialog is open there, or too many hand-offs wait).
    pub const BUSY: &str = "busy";
    /// The target's window didn't take it in time.
    pub const TIMEOUT: &str = "timeout";
    /// The request was malformed or not authorised.
    pub const INVALID: &str = "invalid";
    /// The file to open is no longer there.
    pub const MISSING: &str = "missing";
    /// The target couldn't open it.
    pub const FAILED: &str = "failed";
    /// No connection to the target (its process has gone).
    pub const CLOSED: &str = "closed";
}

/// One window's tab strip on the screen, in physical pixels: `[left, top, right, bottom]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Strip {
    pub window: u64,
    pub strip: [f32; 4],
    pub tabs: Vec<[f32; 4]>,
}

fn to_array(r: Rect) -> [f32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y]
}

/// A published rectangle, if it is one: finite, within [`MAX_COORD`], not inverted.
fn to_rect(a: &[f32; 4]) -> Option<Rect> {
    let [x0, y0, x1, y1] = *a;
    let ok = a.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) && x0 <= x1 && y0 <= y1;
    ok.then(|| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)))
}

impl Strip {
    /// Window `window`'s strip and its tabs, left to right (at most [`MAX_TABS`]).
    pub fn new(window: u64, strip: Rect, tabs: impl IntoIterator<Item = Rect>) -> Self {
        Self { window, strip: to_array(strip), tabs: tabs.into_iter().take(MAX_TABS).map(to_array).collect() }
    }

    fn valid(&self) -> bool {
        self.tabs.len() <= MAX_TABS && to_rect(&self.strip).is_some() && self.tabs.iter().all(|t| to_rect(t).is_some())
    }
}

/// A rectangle in points (this window's content) as physical screen pixels.
pub fn to_screen_px(r: Rect, window_origin: egui::Vec2, pixels_per_point: f32) -> Rect {
    let scale = |p: Pos2| ((p + window_origin).to_vec2() * pixels_per_point).to_pos2();
    Rect::from_min_max(scale(r.min), scale(r.max))
}

/// What one process publishes in the registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    /// When it was last written, in seconds since the Unix epoch.
    pub updated: u64,
    pub strips: Vec<Strip>,
}

fn valid_token(t: &str) -> bool {
    t.len() == TOKEN_LEN && t.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether an entry written at `updated` is current at `now`.
fn fresh(updated: u64, now: u64) -> bool {
    now.saturating_sub(updated) <= STALE_SECS && updated <= now.saturating_add(STALE_SECS)
}

impl Entry {
    /// A registry file's entry, if it is a well-formed and current one.
    pub fn parse(bytes: &[u8], now: u64) -> Option<Self> {
        if bytes.len() as u64 > MAX_ENTRY_BYTES {
            return None;
        }
        let e: Self = serde_json::from_slice(bytes).ok()?;
        let ok = e.pid != 0
            && e.port != 0
            && valid_token(&e.token)
            && fresh(e.updated, now)
            && e.strips.len() <= MAX_STRIPS
            && e.strips.iter().all(Strip::valid);
        ok.then_some(e)
    }
}

/// Seconds since the Unix epoch (0 if the clock is before it).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether process `pid` is still running, where that can be told without `unsafe` (Linux); a
/// process that died elsewhere stops refreshing its entry, which then goes stale.
fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        if Path::new("/proc/self").exists() {
            return Path::new(&format!("/proc/{pid}")).exists();
        }
    }
    let _ = pid;
    true
}

/// The per-user folder of registry files.
#[derive(Clone, Debug)]
pub struct Registry {
    dir: PathBuf,
}

impl Registry {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `Windows` beside the crash-recovery folder ([`crate::RecoveryStore::default_dir`]): the
    /// user's own data folder, which other accounts can't read.
    pub fn default_dir() -> Option<PathBuf> {
        crate::RecoveryStore::default_dir().and_then(|d| d.parent().map(|p| p.join("Windows")))
    }

    fn file(&self, pid: u32) -> PathBuf {
        self.dir.join(format!("{pid}.json"))
    }

    /// Write `entry` as its process's file (replacing it whole, so a reader never sees half).
    pub fn publish(&self, entry: &Entry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec(entry).map_err(std::io::Error::other)?;
        let tmp = self.dir.join(format!("{}.tmp", entry.pid));
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, self.file(entry.pid))
    }

    /// Remove process `pid`'s file.
    pub fn withdraw(&self, pid: u32) {
        let _ = std::fs::remove_file(self.file(pid));
        let _ = std::fs::remove_file(self.dir.join(format!("{pid}.tmp")));
    }

    /// The other processes' current entries (not `own_pid`'s), at most [`MAX_PEERS`]. Files of
    /// processes long gone are deleted.
    pub fn peers(&self, own_pid: u32, now: u64) -> Vec<Entry> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out = Vec::new();
        for item in dir.flatten().take(MAX_SCAN) {
            if out.len() >= MAX_PEERS {
                break;
            }
            let path = item.path();
            let pid = path
                .extension()
                .filter(|e| *e == "json")
                .and_then(|_| path.file_stem())
                .and_then(|s| s.to_str())
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u32>().ok());
            let Some(pid) = pid else { continue };
            if pid == own_pid || !item.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            match read_capped(&path).and_then(|b| Entry::parse(&b, now)) {
                Some(e) if e.pid == pid && alive(pid) => out.push(e),
                _ => {
                    let old =
                        item.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > FORGET_SECS);
                    if old {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
        out
    }
}

/// A file's bytes, unless it is longer than [`MAX_ENTRY_BYTES`].
fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_ENTRY_BYTES).then_some(bytes)
}

/// Where a tab let go over another process's strip goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pid: u32,
    pub port: u16,
    pub token: String,
    /// The window there (its id in that process).
    pub window: u64,
    /// The gap: before that window's tab with this index (its tab count: after the last).
    pub before: usize,
}

/// The strip among `peers` under `at` (physical screen pixels), counting `slack` pixels above
/// and below it, and the gap there.
pub fn find_target(peers: &[Entry], at: Pos2, slack: f32) -> Option<Target> {
    if !(at.x.is_finite() && at.y.is_finite()) {
        return None;
    }
    let slack = if slack.is_finite() { slack.clamp(0.0, 1000.0) } else { 0.0 };
    for e in peers {
        for s in &e.strips {
            let Some(strip) = to_rect(&s.strip) else { continue };
            if !strip.expand2(vec2(0.0, slack)).contains(at) {
                continue;
            }
            let tabs: Vec<Rect> = s.tabs.iter().filter_map(to_rect).collect();
            let before = tabs.iter().position(|r| at.x < r.center().x).unwrap_or(tabs.len());
            return Some(Target { pid: e.pid, port: e.port, token: e.token.clone(), window: s.window, before });
        }
    }
    None
}

/// What letting go of a dragged tab does.
#[derive(Clone, Debug, PartialEq)]
pub enum TabDrop<W> {
    /// Moves along its own strip.
    Reorder,
    /// Joins another window of this process.
    ToWindow(W),
    /// Goes to a window of another process.
    ToProcess(Target),
    /// Opens in a new window.
    NewWindow,
    /// Nothing happens.
    Stay,
}

/// The drop: its own strip first, then another window of this process, then one of another
/// process, then (pulled well out, where it may) a new window.
pub fn decide<W>(in_strip: bool, this_process: Option<W>, other_process: Option<Target>, pulled_out: bool, can_tear: bool) -> TabDrop<W> {
    if in_strip {
        TabDrop::Reorder
    } else if let Some(w) = this_process {
        TabDrop::ToWindow(w)
    } else if let Some(t) = other_process {
        TabDrop::ToProcess(t)
    } else if pulled_out && can_tear {
        TabDrop::NewWindow
    } else {
        TabDrop::Stay
    }
}

/// A document handed to another process: what to open, in which window, at which gap and page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    pub what: HandOff,
    pub window: u64,
    pub before: usize,
    pub page: usize,
}

/// The request line for `take`, authorised by `token`.
pub fn request_line(token: &str, take: &Take) -> String {
    let mut m = Map::new();
    m.insert("v".into(), Value::from(1));
    m.insert("token".into(), Value::from(token));
    match &take.what {
        HandOff::File(path) => m.insert("file".into(), Value::from(path.as_str())),
        HandOff::Recovery(key) => m.insert("recovery".into(), Value::from(key.as_str())),
    };
    m.insert("window".into(), Value::from(take.window));
    m.insert("before".into(), Value::from(take.before));
    m.insert("page".into(), Value::from(take.page));
    let mut line = Value::Object(m).to_string();
    line.push('\n');
    line
}

/// A file path handed over: absolute, not empty, not too long, no control characters.
pub fn valid_hand_off_path(path: &str) -> bool {
    !path.is_empty() && path.len() <= MAX_PATH_LEN && !path.chars().any(char::is_control) && Path::new(path).is_absolute()
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The document a request line hands over, if it is a well-formed request carrying `token`.
pub fn parse_request(line: &str, token: &str) -> Result<Take, &'static str> {
    if line.len() > MAX_MESSAGE {
        return Err("the request is too long");
    }
    let value: Value = serde_json::from_str(line.trim_end()).map_err(|_| "not JSON")?;
    let o = value.as_object().ok_or("not an object")?;
    if o.get("v").and_then(Value::as_u64) != Some(1) {
        return Err("unknown version");
    }
    let given = o.get("token").and_then(Value::as_str).ok_or("no token")?;
    if !valid_token(token) || !constant_time_eq(given, token) {
        return Err("wrong token");
    }
    let what = match (o.get("file"), o.get("recovery")) {
        (Some(f), None) => {
            let path = f.as_str().filter(|p| valid_hand_off_path(p)).ok_or("bad file path")?;
            HandOff::File(path.to_owned())
        }
        (None, Some(k)) => {
            let key = k.as_str().filter(|k| valid_recovery_key(k)).ok_or("bad recovery key")?;
            HandOff::Recovery(key.to_owned())
        }
        _ => return Err("needs a file or a recovery key"),
    };
    let window = o.get("window").and_then(Value::as_u64).ok_or("bad window")?;
    let before = o.get("before").and_then(Value::as_u64).filter(|&b| b <= MAX_TABS as u64).ok_or("bad gap")?;
    let page = o.get("page").and_then(Value::as_u64).filter(|&p| p <= MAX_PAGE).ok_or("bad page")?;
    let before = usize::try_from(before).map_err(|_| "bad gap")?;
    let page = usize::try_from(page).map_err(|_| "bad page")?;
    Ok(Take { what, window, before, page })
}

/// The answer line: taken, or the reason ([`reason`]) it wasn't.
pub fn reply_line(result: &Result<(), String>) -> String {
    let mut line = match result {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(reason) => serde_json::json!({ "ok": false, "reason": reason }),
    }
    .to_string();
    line.push('\n');
    line
}

/// What an answer line says: taken, or why not (a [`reason`]; anything unreadable is
/// [`reason::INVALID`]).
pub fn parse_reply(line: &str) -> Result<(), String> {
    let value: Value = serde_json::from_str(line.trim_end()).map_err(|_| reason::INVALID.to_owned())?;
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(()),
        Some(false) => {
            let why = value.get("reason").and_then(Value::as_str).unwrap_or(reason::INVALID);
            Err(why.chars().filter(|c| !c.is_control()).take(64).collect())
        }
        None => Err(reason::INVALID.to_owned()),
    }
}

/// A failure's reason in the user's words (on the UI thread, which knows the language).
pub fn reason_text(code: &str) -> String {
    match code {
        reason::CLOSED => tl!("that window has closed"),
        reason::TIMEOUT => tl!("that window didn't answer"),
        reason::BUSY => tl!("that window is busy"),
        reason::MISSING => tl!("the file is no longer there"),
        reason::FAILED => tl!("it couldn't be opened there"),
        _ => tl!("that window refused it"),
    }
    .to_string()
}

#[cfg(not(target_arch = "wasm32"))]
pub use transport::{Inbox, Peer, Pending, send};

#[cfg(not(target_arch = "wasm32"))]
mod transport {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
    use std::time::Instant;

    use super::*;

    enum State {
        Waiting,
        Done(Result<(), String>),
        Cancelled,
    }

    type Shared = Arc<(Mutex<State>, Condvar)>;

    /// A document handed to this process, waiting for its UI.
    pub struct Pending {
        take: Take,
        state: Shared,
    }

    impl Pending {
        pub fn take(&self) -> &Take {
            &self.take
        }

        /// Take it with `open` (on the UI thread), unless the sender has given up on it. The
        /// sender learns the result once `open` returns. Returns whether `open` ran.
        pub fn answer(self, open: impl FnOnce(&Take) -> Result<(), String>) -> bool {
            let (lock, cvar) = &*self.state;
            let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if !matches!(*state, State::Waiting) {
                return false;
            }
            *state = State::Done(open(&self.take));
            cvar.notify_all();
            true
        }

        fn waiting(&self) -> bool {
            matches!(*self.state.0.lock().unwrap_or_else(PoisonError::into_inner), State::Waiting)
        }
    }

    /// Documents handed over, for the UI to take every frame.
    #[derive(Clone, Default)]
    pub struct Inbox(Arc<Mutex<Vec<Pending>>>);

    impl Inbox {
        pub fn drain(&self) -> Vec<Pending> {
            std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
        }

        /// Queue `take`, wake the UI and wait (at most [`TAKE_WAIT`]) for its answer.
        pub(super) fn deliver(&self, take: Take, wake: &(dyn Fn() + Send + Sync)) -> Result<(), String> {
            let state: Shared = Arc::new((Mutex::new(State::Waiting), Condvar::new()));
            {
                let mut queue = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                queue.retain(Pending::waiting);
                if queue.len() >= MAX_QUEUE {
                    return Err(reason::BUSY.to_owned());
                }
                queue.push(Pending { take, state: Arc::clone(&state) });
            }
            wake();
            let (lock, cvar) = &*state;
            let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
            let (mut guard, _) = cvar.wait_timeout_while(guard, TAKE_WAIT, |s| matches!(s, State::Waiting)).unwrap_or_else(PoisonError::into_inner);
            // Not taken in time: the UI must not take it later, after the sender gave up.
            match std::mem::replace(&mut *guard, State::Cancelled) {
                State::Done(result) => result,
                State::Waiting | State::Cancelled => Err(reason::TIMEOUT.to_owned()),
            }
        }
    }

    /// The strips this process shows, for the heartbeat to write.
    #[derive(Default)]
    struct Published {
        strips: Vec<Strip>,
        dirty: bool,
    }

    /// This process's end: its registry file, its listener and the documents handed to it.
    pub struct Peer {
        pid: u32,
        port: u16,
        token: String,
        registry: Registry,
        inbox: Inbox,
        published: Arc<Mutex<Published>>,
        stopped: Arc<AtomicBool>,
        /// Dropped to stop the heartbeat at once.
        _stop: mpsc::Sender<()>,
        peers: Vec<Entry>,
        peers_read: Option<Instant>,
    }

    impl Peer {
        /// Listen on loopback, publish in `registry`, and call `wake` (from another thread)
        /// when a document arrives.
        pub fn start(registry: Registry, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
            Self::start_as(registry, std::process::id(), wake)
        }

        /// [`Self::start`] under pid `pid` (tests run several peers in one process).
        #[doc(hidden)]
        pub fn start_as(registry: Registry, pid: u32, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Self> {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            let port = listener.local_addr()?.port();
            let token = crate::control::random_token()?;
            let inbox = Inbox::default();
            let stopped = Arc::new(AtomicBool::new(false));
            {
                let (token, inbox, stopped) = (token.clone(), inbox.clone(), Arc::clone(&stopped));
                std::thread::Builder::new().name("tab-transfer".into()).spawn(move || {
                    for stream in listener.incoming() {
                        if stopped.load(Ordering::Relaxed) {
                            break;
                        }
                        match stream {
                            Ok(s) => serve_one(&s, &token, &inbox, &wake),
                            // Don't spin if accepting keeps failing.
                            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
                        }
                    }
                })?;
            }
            let published = Arc::new(Mutex::new(Published { strips: Vec::new(), dirty: true }));
            let (stop, stop_rx) = mpsc::channel::<()>();
            {
                let (registry, token, published) = (registry.clone(), token.clone(), Arc::clone(&published));
                let beat = move || {
                    let mut last = None::<Instant>;
                    loop {
                        let strips = {
                            let mut p = published.lock().unwrap_or_else(PoisonError::into_inner);
                            let due = p.dirty || last.is_none_or(|t: Instant| t.elapsed() >= HEARTBEAT);
                            p.dirty = false;
                            due.then(|| p.strips.clone())
                        };
                        if let Some(strips) = strips {
                            let entry = Entry { pid, port, token: token.clone(), updated: now_secs(), strips };
                            if let Err(e) = registry.publish(&entry) {
                                log::debug!("tab strips not published: {e}");
                            }
                            last = Some(Instant::now());
                        }
                        match stop_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            _ => break,
                        }
                    }
                    registry.withdraw(pid);
                };
                if let Err(e) = std::thread::Builder::new().name("tab-strips".into()).spawn(beat) {
                    stopped.store(true, Ordering::Relaxed);
                    return Err(e);
                }
            }
            Ok(Self { pid, port, token, registry, inbox, published, stopped, _stop: stop, peers: Vec::new(), peers_read: None })
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn port(&self) -> u16 {
            self.port
        }

        pub fn token(&self) -> &str {
            &self.token
        }

        /// The documents handed over, for the UI to take.
        pub fn inbox(&self) -> Inbox {
            self.inbox.clone()
        }

        /// This frame's strips (written by the heartbeat thread when they change).
        pub fn publish(&self, strips: Vec<Strip>) {
            let mut p = self.published.lock().unwrap_or_else(PoisonError::into_inner);
            if p.strips != strips {
                p.strips = strips;
                p.dirty = true;
            }
        }

        /// The other processes' strips, read at most every [`PEERS_REFRESH`].
        pub fn peers(&mut self) -> &[Entry] {
            if self.peers_read.is_none_or(|t| t.elapsed() >= PEERS_REFRESH) {
                self.peers = self.registry.peers(self.pid, now_secs());
                self.peers_read = Some(Instant::now());
            }
            &self.peers
        }
    }

    impl Drop for Peer {
        fn drop(&mut self) {
            self.stopped.store(true, Ordering::Relaxed);
            self.registry.withdraw(self.pid);
            // Wake the listener so it sees the flag and ends.
            let _ = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], self.port)), std::time::Duration::from_millis(200));
        }
    }

    /// One connection: read the request, hand it to the UI, answer.
    fn serve_one(stream: &TcpStream, token: &str, inbox: &Inbox, wake: &(dyn Fn() + Send + Sync)) {
        if stream.set_read_timeout(Some(IO_TIMEOUT)).is_err() || stream.set_write_timeout(Some(IO_TIMEOUT)).is_err() {
            return;
        }
        let mut line = String::new();
        let limit = u64::try_from(MAX_MESSAGE).unwrap_or(u64::MAX).saturating_add(1);
        if BufReader::new(stream.take(limit)).read_line(&mut line).is_err() || line.is_empty() {
            return;
        }
        let result = match parse_request(&line, token) {
            Ok(take) => inbox.deliver(take, wake),
            Err(why) => {
                log::warn!("a tab handed over was refused: {why}");
                Err(reason::INVALID.to_owned())
            }
        };
        let mut writer = stream;
        let _ = writer.write_all(reply_line(&result).as_bytes()).and_then(|()| writer.flush());
    }

    /// Hand `take` to `target`'s process and wait for its answer. The error is a [`reason`].
    pub fn send(target: &Target, take: &Take) -> Result<(), String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], target.port));
        let closed = |_| reason::CLOSED.to_owned();
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).map_err(closed)?;
        stream.set_write_timeout(Some(IO_TIMEOUT)).map_err(closed)?;
        stream.set_read_timeout(Some(REPLY_TIMEOUT)).map_err(closed)?;
        let mut writer = &stream;
        writer.write_all(request_line(&target.token, take).as_bytes()).and_then(|()| writer.flush()).map_err(closed)?;
        let mut line = String::new();
        match BufReader::new((&stream).take(MAX_REPLY)).read_line(&mut line) {
            Ok(0) => Err(reason::CLOSED.to_owned()),
            Ok(_) => parse_reply(&line),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(reason::TIMEOUT.to_owned()),
            Err(_) => Err(reason::CLOSED.to_owned()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl crate::PdfCraftApp {
    /// Take the documents other processes handed over (each frame).
    pub(crate) fn take_handed_tabs(&mut self) {
        let Some(inbox) = self.tab_peer.as_ref().map(Peer::inbox) else { return };
        for pending in inbox.drain() {
            pending.answer(|take| self.take_tab(take));
        }
    }

    /// Open a document another process handed over, in window `take.window` at gap
    /// `take.before`. The error is a [`reason`].
    pub fn take_tab(&mut self, take: &Take) -> Result<(), String> {
        // A dialog or prompt is open: opening now could replace a password prompt.
        if self.modal_open() {
            return Err(reason::BUSY.to_owned());
        }
        let had: std::collections::HashSet<pdfcraft_engine::DocId> = self.views.iter().map(|v| v.id).collect();
        match &take.what {
            HandOff::File(path) => {
                if !valid_hand_off_path(path) {
                    return Err(reason::INVALID.to_owned());
                }
                if !Path::new(path).is_file() {
                    return Err(reason::MISSING.to_owned());
                }
                self.open_path(path);
            }
            HandOff::Recovery(key) => {
                if !valid_recovery_key(key) {
                    return Err(reason::INVALID.to_owned());
                }
                self.open_handed_off(key);
            }
        }
        let Some(doc) = self.views.iter().find(|v| !had.contains(&v.id)).map(|v| v.id) else {
            // Encrypted: the password prompt holds it now and opens it.
            return if self.password_prompt.is_some() { Ok(()) } else { Err(reason::FAILED.to_owned()) };
        };
        let window = if take.window == 0 || self.doc_windows.iter().any(|w| w.id == take.window) { take.window } else { 0 };
        let before = self.views.iter().filter(|v| v.window == window && v.id != doc).nth(take.before).map(|v| v.id);
        self.move_tab(doc, window, before);
        if take.page > 0
            && let Some(view) = self.views.iter_mut().find(|v| v.id == doc)
        {
            view.go_to_page(take.page);
        }
        if let Some(ctx) = &self.ctx {
            let viewport = if window == 0 { egui::ViewportId::ROOT } else { egui::ViewportId::from_hash_of(("sintec-doc-window", window)) };
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
        log::info!("tab taken from another window: {:?}", take.what);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
    }

    fn entry(pid: u32, updated: u64) -> Entry {
        Entry {
            pid,
            port: 4000,
            token: TOKEN.into(),
            updated,
            strips: vec![Strip::new(0, rect(100.0, 50.0, 900.0, 90.0), [rect(110.0, 55.0, 300.0, 85.0), rect(310.0, 55.0, 500.0, 85.0)])],
        }
    }

    fn take(what: HandOff) -> Take {
        Take { what, window: 3, before: 1, page: 4 }
    }

    fn abs_path() -> String {
        std::env::temp_dir().join("Report.pdf").to_string_lossy().into_owned()
    }

    #[test]
    fn a_request_round_trips_with_its_token() {
        for what in [HandOff::File(abs_path()), HandOff::Recovery("1760000000-42-tear-3".into())] {
            let t = take(what);
            let line = request_line(TOKEN, &t);
            assert!(line.ends_with('\n') && !line.trim_end().contains('\n'), "one line");
            assert_eq!(parse_request(&line, TOKEN), Ok(t));
        }
    }

    #[test]
    fn requests_without_the_token_or_with_bad_fields_are_refused() {
        let good = request_line(TOKEN, &take(HandOff::File(abs_path())));
        assert!(parse_request(&good, "fedcba9876543210fedcba9876543210").is_err(), "another token");
        assert!(parse_request(&good, "").is_err(), "no token expected: nothing accepted");
        let path = serde_json::to_string(&abs_path()).unwrap();
        let bad = [
            String::new(),
            "not json".into(),
            "[]".into(),
            format!(r#"{{"v":2,"token":"{TOKEN}","file":{path},"window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"file":{path},"window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"recovery":"k","window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":"relative.pdf","window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":"","window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":3,"window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","recovery":"../x","window":0,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":-1,"before":0,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":0,"before":1e9,"page":0}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":0,"before":{},"page":0}}"#, MAX_TABS + 1),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":0,"before":0,"page":"1"}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":0,"before":0,"page":99999999999}}"#),
            format!(r#"{{"v":1,"token":"{TOKEN}","file":{path},"window":0,"before":0}}"#),
        ];
        for line in &bad {
            assert!(parse_request(line, TOKEN).is_err(), "{line}");
        }
        let long = format!(r#"{{"v":1,"token":"{TOKEN}","file":"/{}","window":0,"before":0,"page":0}}"#, "a".repeat(MAX_MESSAGE));
        assert_eq!(parse_request(&long, TOKEN), Err("the request is too long"));
        let control = format!(r#"{{"v":1,"token":"{TOKEN}","file":"/a\u0000b.pdf","window":0,"before":0,"page":0}}"#);
        assert!(parse_request(&control, TOKEN).is_err(), "control characters in a path");
    }

    #[test]
    fn replies_say_taken_or_why_not() {
        assert_eq!(parse_reply(&reply_line(&Ok(()))), Ok(()));
        assert_eq!(parse_reply(&reply_line(&Err(reason::BUSY.into()))), Err(reason::BUSY.into()));
        for junk in ["", "ok", "{}", r#"{"ok":"yes"}"#] {
            assert_eq!(parse_reply(junk), Err(reason::INVALID.into()), "{junk:?}");
        }
        let long = parse_reply(&format!(r#"{{"ok":false,"reason":"{}\u0007"}}"#, "x".repeat(500))).unwrap_err();
        assert_eq!(long.len(), 64, "bounded, no control characters");
        assert_eq!(reason_text("closed"), "that window has closed");
        assert_eq!(reason_text("anything else"), "that window refused it");
    }

    #[test]
    fn registry_entries_are_validated() {
        let now = 1_760_000_000;
        let good = serde_json::to_vec(&entry(42, now)).unwrap();
        assert_eq!(Entry::parse(&good, now), Some(entry(42, now)));
        assert_eq!(Entry::parse(&good, now + STALE_SECS), Some(entry(42, now)), "still fresh");
        assert_eq!(Entry::parse(&good, now + STALE_SECS + 1), None, "stale");
        assert_eq!(Entry::parse(&good, now - STALE_SECS - 1), None, "from the future");
        let mut bad = Vec::new();
        let mut e = entry(0, now);
        bad.push(e.clone());
        e = entry(42, now);
        e.port = 0;
        bad.push(e);
        e = entry(42, now);
        e.token = "short".into();
        bad.push(e);
        e = entry(42, now);
        e.token = "z".repeat(TOKEN_LEN);
        bad.push(e);
        e = entry(42, now);
        e.strips[0].strip = [f32::NAN, 0.0, 1.0, 1.0];
        bad.push(e);
        e = entry(42, now);
        e.strips[0].tabs.push([10.0, 10.0, 5.0, 5.0]);
        bad.push(e);
        e = entry(42, now);
        e.strips[0].strip = [0.0, 0.0, 2.0e6, 1.0];
        bad.push(e);
        e = entry(42, now);
        e.strips = vec![e.strips[0].clone(); MAX_STRIPS + 1];
        bad.push(e);
        for e in bad {
            assert_eq!(Entry::parse(&serde_json::to_vec(&e).unwrap(), now), None, "{e:?}");
        }
        assert_eq!(Entry::parse(b"{\"pid\":1", now), None);
        assert_eq!(Entry::parse(&vec![b' '; MAX_ENTRY_BYTES as usize + 1], now), None, "too long");
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdfcraft-tab-registry-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_registry_lists_other_current_processes_only() {
        let dir = scratch("list");
        let reg = Registry::new(&dir);
        let now = now_secs();
        assert!(reg.peers(1, now).is_empty(), "no folder yet");
        reg.publish(&entry(std::process::id(), now)).unwrap();
        reg.publish(&entry(u32::MAX - 1, now)).unwrap();
        reg.publish(&entry(u32::MAX - 2, now.saturating_sub(STALE_SECS + 5))).unwrap();
        // A file whose content claims another pid, junk, an oversized file, a foreign name.
        std::fs::write(dir.join("7.json"), serde_json::to_vec(&entry(8, now)).unwrap()).unwrap();
        std::fs::write(dir.join("9.json"), b"garbage").unwrap();
        std::fs::write(dir.join("10.json"), vec![b' '; MAX_ENTRY_BYTES as usize + 10]).unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("x1.json"), serde_json::to_vec(&entry(1, now)).unwrap()).unwrap();
        let peers = reg.peers(1, now);
        let mut pids: Vec<u32> = peers.iter().map(|e| e.pid).collect();
        pids.sort_unstable();
        // This process's own pid is listed only for a different "own" pid; the stale one never.
        #[cfg(not(target_os = "linux"))]
        assert_eq!(pids, [std::process::id(), u32::MAX - 1]);
        #[cfg(target_os = "linux")]
        assert_eq!(pids, [std::process::id()], "Linux also drops pids that aren't running");
        assert!(reg.peers(std::process::id(), now).iter().all(|e| e.pid != std::process::id()), "never itself");
        reg.withdraw(u32::MAX - 1);
        assert!(reg.peers(1, now).iter().all(|e| e.pid != u32::MAX - 1), "withdrawn");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_registry_reads_a_bounded_number_of_peers() {
        let dir = scratch("cap");
        let reg = Registry::new(&dir);
        let now = now_secs();
        for i in 0..(MAX_PEERS as u32 + 8) {
            reg.publish(&entry(std::process::id().wrapping_add(i), now)).unwrap();
        }
        assert!(reg.peers(0, now).len() <= MAX_PEERS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_target_is_the_strip_under_the_pointer_and_the_gap_there() {
        let peers = [entry(42, 0)];
        let t = find_target(&peers, pos2(120.0, 70.0), 24.0).unwrap();
        assert_eq!((t.pid, t.port, t.window, t.before), (42, 4000, 0, 0), "before the first tab");
        assert_eq!(find_target(&peers, pos2(400.0, 70.0), 24.0).map(|t| t.before), Some(1), "on the right half of the first");
        assert_eq!(find_target(&peers, pos2(800.0, 70.0), 24.0).map(|t| t.before), Some(2), "after the last");
        assert_eq!(find_target(&peers, pos2(800.0, 110.0), 24.0).map(|t| t.before), Some(2), "just below the strip");
        assert_eq!(find_target(&peers, pos2(800.0, 200.0), 24.0), None, "over the page");
        assert_eq!(find_target(&peers, pos2(f32::NAN, 70.0), 24.0), None);
        assert_eq!(find_target(&[], pos2(120.0, 70.0), 24.0), None);
    }

    #[test]
    fn rectangles_go_to_physical_screen_pixels() {
        let r = to_screen_px(rect(10.0, 20.0, 30.0, 40.0), vec2(100.0, 200.0), 1.5);
        assert_eq!(r, rect(165.0, 330.0, 195.0, 360.0));
    }

    #[test]
    fn the_drop_prefers_its_own_strip_then_this_process_then_another() {
        let t = Target { pid: 1, port: 2, token: TOKEN.into(), window: 0, before: 0 };
        assert_eq!(decide(true, Some(5), Some(t.clone()), true, true), TabDrop::Reorder);
        assert_eq!(decide(false, Some(5), Some(t.clone()), true, true), TabDrop::ToWindow(5));
        assert_eq!(decide::<u8>(false, None, Some(t.clone()), true, false), TabDrop::ToProcess(t.clone()), "even a window's only tab");
        assert_eq!(decide::<u8>(false, None, None, true, true), TabDrop::NewWindow);
        assert_eq!(decide::<u8>(false, None, None, true, false), TabDrop::Stay);
        assert_eq!(decide::<u8>(false, None, None, false, true), TabDrop::Stay, "not pulled far enough");
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod transport {
        use super::*;

        fn target_of(peer: &Peer, before: usize) -> Target {
            Target { pid: peer.pid(), port: peer.port(), token: peer.token().to_owned(), window: 0, before }
        }

        #[test]
        fn a_peer_publishes_its_strips_and_withdraws_them_when_dropped() {
            let dir = scratch("peer");
            let reg = Registry::new(&dir);
            let peer = Peer::start_as(reg.clone(), u32::MAX - 7, || {}).unwrap();
            let strip = Strip::new(0, rect(0.0, 0.0, 500.0, 40.0), [rect(5.0, 5.0, 100.0, 35.0)]);
            peer.publish(vec![strip.clone()]);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let found = loop {
                let e = reg.peers(1, now_secs()).into_iter().find(|e| e.pid == u32::MAX - 7 && e.strips == [strip.clone()]);
                if e.is_some() || std::time::Instant::now() > deadline {
                    break e;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            let found = found.expect("published");
            assert_eq!((found.port, found.token.as_str()), (peer.port(), peer.token()));
            drop(peer);
            std::thread::sleep(std::time::Duration::from_millis(300));
            assert!(reg.peers(1, now_secs()).iter().all(|e| e.pid != u32::MAX - 7), "withdrawn");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_document_is_handed_over_and_confirmed() {
            let dir = scratch("send");
            let peer = Peer::start_as(Registry::new(&dir), u32::MAX - 8, || {}).unwrap();
            let inbox = peer.inbox();
            let ui = std::thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    if let Some(p) = inbox.drain().into_iter().next() {
                        let got = p.take().clone();
                        p.answer(|_| Ok(()));
                        return Some(got);
                    }
                    if std::time::Instant::now() > deadline {
                        return None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
            let t = take(HandOff::Recovery("1760000000-42-tear-3".into()));
            assert_eq!(send(&target_of(&peer, 1), &t), Ok(()));
            assert_eq!(ui.join().unwrap(), Some(t));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_refusal_a_wrong_token_and_a_dead_peer_fail_quickly() {
            let dir = scratch("refuse");
            let peer = Peer::start_as(Registry::new(&dir), u32::MAX - 9, || {}).unwrap();
            let inbox = peer.inbox();
            let ui = std::thread::spawn(move || {
                for _ in 0..500 {
                    if let Some(p) = inbox.drain().into_iter().next() {
                        p.answer(|_| Err(reason::MISSING.to_owned()));
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            });
            let t = take(HandOff::File(abs_path()));
            assert_eq!(send(&target_of(&peer, 0), &t), Err(reason::MISSING.into()));
            ui.join().unwrap();
            let mut wrong = target_of(&peer, 0);
            wrong.token = "fedcba9876543210fedcba9876543210".into();
            assert_eq!(send(&wrong, &t), Err(reason::INVALID.into()));
            assert!(peer.inbox().drain().is_empty(), "a wrong token reaches nothing");
            // Its process has gone: nothing listens on the port any more.
            let dead = target_of(&peer, 0);
            drop(peer);
            std::thread::sleep(std::time::Duration::from_millis(100));
            let started = std::time::Instant::now();
            assert_eq!(send(&dead, &t), Err(reason::CLOSED.into()));
            assert!(started.elapsed() < std::time::Duration::from_secs(3), "no long wait for a dead peer");
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn a_document_not_taken_in_time_is_refused_and_never_taken_later() {
            let inbox = Inbox::default();
            let deliverer = inbox.clone();
            let started = std::time::Instant::now();
            let result = std::thread::spawn(move || deliverer.deliver(take(HandOff::File(abs_path())), &|| {})).join().unwrap();
            assert_eq!(result, Err(reason::TIMEOUT.into()));
            assert!(started.elapsed() >= TAKE_WAIT);
            for p in inbox.drain() {
                assert!(!p.answer(|_| panic!("the sender gave up on it")), "cancelled");
            }
        }
    }
}
