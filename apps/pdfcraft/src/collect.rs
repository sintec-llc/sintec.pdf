//! Explorer ▸ «Преобразовать в PDF» on several files: Windows starts one process per selected
//! file, each with one path. They pool their paths in a spool folder; the first to take the lock
//! waits until no more arrive and converts them all at once, and the rest exit without a window.
//! Plain files and an exclusive-create lock: no IPC, no unsafe code.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How long the collector waits after the last path arrived before converting.
const QUIET: Duration = Duration::from_millis(800);
/// Never wait longer than this in total (a huge selection keeps adding paths).
const MAX_WAIT: Duration = Duration::from_secs(15);
/// A lock older than this belongs to a collector that died; take it over.
const STALE_LOCK: Duration = Duration::from_secs(60);

/// What this process should do with the paths it was given.
pub enum Role {
    /// Convert these paths (this process collected them).
    Convert(Vec<PathBuf>),
    /// Another process is collecting; exit quietly.
    Handled,
}

/// Pool `paths` with any other launches and decide this process's role.
pub fn collect(paths: Vec<PathBuf>, spool: &Path) -> Role {
    if std::fs::create_dir_all(spool).is_err() {
        return Role::Convert(paths);
    }
    let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let list = spool.join(format!("{stamp}-{}.list", std::process::id()));
    let text: String = paths.iter().map(|p| format!("{}\n", p.display())).collect();
    if std::fs::write(&list, text).is_err() {
        return Role::Convert(paths);
    }
    let lock = spool.join("collector.lock");
    if !take_lock(&lock) {
        return Role::Handled;
    }
    // This process collects: wait for the others to write their lists, then take them all.
    let start = Instant::now();
    let mut seen = count_lists(spool);
    let mut last_change = Instant::now();
    while last_change.elapsed() < QUIET && start.elapsed() < MAX_WAIT {
        std::thread::sleep(Duration::from_millis(100));
        let now = count_lists(spool);
        if now != seen {
            seen = now;
            last_change = Instant::now();
        }
    }
    let mut all = Vec::new();
    if let Ok(entries) = std::fs::read_dir(spool) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "list") {
                if let Ok(t) = std::fs::read_to_string(&p) {
                    all.extend(t.lines().filter(|l| !l.trim().is_empty()).map(PathBuf::from));
                }
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    let _ = std::fs::remove_file(&lock);
    if all.is_empty() {
        all = paths;
    }
    all.sort();
    all.dedup();
    Role::Convert(all)
}

fn count_lists(spool: &Path) -> usize {
    std::fs::read_dir(spool).map_or(0, |d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "list")).count())
}

/// Create the lock file exclusively; take over a stale one.
fn take_lock(lock: &Path) -> bool {
    for _ in 0..2 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(lock) {
            Ok(_) => return true,
            Err(_) => {
                let stale =
                    std::fs::metadata(lock).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > STALE_LOCK);
                if !stale {
                    return false;
                }
                let _ = std::fs::remove_file(lock);
            }
        }
    }
    false
}

/// The spool folder for this user.
pub fn spool_dir() -> PathBuf {
    std::env::temp_dir().join("sintec-pdf-convert")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simultaneous_launches_pool_into_one_conversion() {
        let spool = std::env::temp_dir().join(format!("sintec-collect-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&spool);
        // Five "processes" (threads here) start at once, one file each, like Explorer does.
        let handles: Vec<_> = (0..5)
            .map(|i| {
                let spool = spool.clone();
                std::thread::spawn(move || collect(vec![PathBuf::from(format!("C:\\docs\\file{i}.jpg"))], &spool))
            })
            .collect();
        let roles: Vec<Role> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let converting: Vec<&Vec<PathBuf>> = roles.iter().filter_map(|r| if let Role::Convert(p) = r { Some(p) } else { None }).collect();
        assert_eq!(converting.len(), 1, "exactly one process converts");
        assert_eq!(converting[0].len(), 5, "and it has every file: {:?}", converting[0]);
        assert!(!spool.join("collector.lock").exists(), "the lock is released");
        let _ = std::fs::remove_dir_all(spool);
    }

    #[test]
    fn a_fresh_lock_belongs_to_another_collector() {
        let spool = std::env::temp_dir().join(format!("sintec-collect-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&spool);
        std::fs::create_dir_all(&spool).unwrap();
        let lock = spool.join("collector.lock");
        std::fs::write(&lock, b"").unwrap();
        // Fresh: someone else is collecting.
        assert!(!take_lock(&lock));
        let _ = std::fs::remove_dir_all(spool);
    }
}
