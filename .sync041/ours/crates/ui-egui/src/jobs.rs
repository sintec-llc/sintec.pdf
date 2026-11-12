//! Background jobs that always report back.
//!
//! A job hands its result to the UI through a slot the UI polls each frame. A worker that
//! panicked, or a thread the system refused to start, used to leave that slot empty for ever:
//! the job showed "running" until the app was closed and could not be started again. Here
//! either case reports a message instead (AGENTS.md §4).

/// Run `work` on a background thread named `name`, or right here when `inline` (tests and
/// automation). A panic in `work`, or a thread that can't be started, is handed to `failed` as
/// a message in the interface's language, so the job's slot is always filled.
pub(crate) fn spawn<W, F>(name: &str, inline: bool, work: W, failed: F)
where
    W: FnOnce() + Send + 'static,
    F: FnOnce(String) + Clone + Send + 'static,
{
    let lang = crate::i18n::current();
    let message = move |m: &str| {
        crate::i18n::set_current(lang);
        crate::i18n::fmt(tl!("That didn't work: an internal error stopped it ({m}). Your documents are unchanged."), &[("m", m)])
    };
    let on_panic = failed.clone();
    let guarded = move || {
        if let Err(m) = pdfcraft_engine::guard(work) {
            on_panic(message(&m));
        }
    };
    if inline {
        guarded();
    } else if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(guarded) {
        log::error!("could not start the {name} thread: {e}");
        failed(message(&e.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    /// A worker that panics still fills its slot, on a thread and inline.
    #[test]
    fn a_panicking_worker_reports_instead_of_running_for_ever() {
        let (tx, rx) = mpsc::channel();
        super::spawn("test-job", false, || panic!("boom"), move |m| tx.send(m).unwrap_or(()));
        let m = rx.recv_timeout(Duration::from_secs(20)).expect("the failure is reported");
        assert!(m.contains("boom"), "{m}");

        let slot = Arc::new(Mutex::new(None));
        let s = slot.clone();
        super::spawn("test-job", true, || panic!("inline boom"), move |m| *s.lock().unwrap() = Some(m));
        assert!(slot.lock().unwrap().as_deref().is_some_and(|m| m.contains("inline boom")));
    }

    /// A worker that finishes normally is not reported as failed.
    #[test]
    fn a_finished_worker_is_not_reported() {
        let (tx, rx) = mpsc::channel();
        let done = tx.clone();
        super::spawn("test-job", false, move || done.send("done".to_owned()).unwrap_or(()), move |m| tx.send(m).unwrap_or(()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).as_deref(), Ok("done"));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
