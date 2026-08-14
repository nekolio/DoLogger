//! Fanout sink — broadcasts every write to all child sinks.
//!
//! A [`FanoutSink`] implements the [`Sink`] trait by delegating each operation
//! to a list of child [`SinkRef`]s. The pipeline keeps its single-sink dispatch
//! contract; multi-destination output is expressed by wrapping the configured
//! sinks in one fanout.

use crate::sink::{Sink, SinkRef, SinkResult};

/// A sink that broadcasts every record to all configured child sinks.
///
/// # Failure semantics
///
/// Writes are best-effort: every child receives the record even if an earlier
/// child fails, and the first error (if any) is reported to the caller. This
/// keeps one failing destination from starving the others.
pub struct FanoutSink {
    sinks: Vec<SinkRef>,
}

impl FanoutSink {
    /// Create a fanout over the given sinks.
    ///
    /// An empty list is allowed; every write is then a successful no-op. The
    /// config layer guarantees at least the console default, so callers only
    /// see an empty fanout when explicitly constructed that way.
    pub fn new(sinks: Vec<SinkRef>) -> Self {
        Self { sinks }
    }

    /// The number of child sinks.
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// True when there are no child sinks.
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

impl Sink for FanoutSink {
    fn open(&mut self) -> SinkResult {
        for sink in &mut self.sinks {
            sink.open()?;
        }
        Ok(())
    }

    fn write(&mut self, formatted: &str) -> SinkResult {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.write(formatted) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn write_batch(&mut self, formatted: &[String]) -> SinkResult {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.write_batch(formatted) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn flush(&mut self) -> SinkResult {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.flush() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn close(&mut self) -> SinkResult {
        let mut first_err = None;
        for sink in &mut self.sinks {
            if let Err(e) = sink.close() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn is_healthy(&self) -> bool {
        self.sinks.iter().all(|s| s.is_healthy())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sink::SinkError;
    use std::sync::{Arc, Mutex};

    /// An in-memory sink that records every write into a shared list.
    #[derive(Clone)]
    struct RecordingSink {
        writes: Arc<Mutex<Vec<String>>>,
        fail: bool,
        is_open: bool,
    }

    impl RecordingSink {
        fn new(writes: Arc<Mutex<Vec<String>>>) -> Self {
            Self {
                writes,
                fail: false,
                is_open: false,
            }
        }
    }

    impl Sink for RecordingSink {
        fn open(&mut self) -> SinkResult {
            self.is_open = true;
            Ok(())
        }

        fn write(&mut self, formatted: &str) -> SinkResult {
            if self.fail {
                return Err(SinkError::WriteFailed("simulated failure".into()));
            }
            self.writes.lock().unwrap().push(formatted.to_string());
            Ok(())
        }

        fn flush(&mut self) -> SinkResult {
            Ok(())
        }

        fn close(&mut self) -> SinkResult {
            self.is_open = false;
            Ok(())
        }
    }

    #[test]
    fn fanout_broadcasts_to_all_sinks() {
        let a = Arc::new(Mutex::new(Vec::new()));
        let b = Arc::new(Mutex::new(Vec::new()));
        let mut fanout = FanoutSink::new(vec![
            SinkRef::new(RecordingSink::new(Arc::clone(&a))),
            SinkRef::new(RecordingSink::new(Arc::clone(&b))),
        ]);
        fanout.open().unwrap();
        fanout.write("hello").unwrap();
        fanout.write("world").unwrap();
        assert_eq!(
            *a.lock().unwrap(),
            vec!["hello".to_string(), "world".to_string()]
        );
        assert_eq!(
            *b.lock().unwrap(),
            vec!["hello".to_string(), "world".to_string()]
        );
    }

    #[test]
    fn fanout_best_effort_keeps_writing_on_child_failure() {
        let good = Arc::new(Mutex::new(Vec::new()));
        let mut bad = RecordingSink::new(Arc::new(Mutex::new(Vec::new())));
        bad.fail = true;

        let mut fanout = FanoutSink::new(vec![
            SinkRef::new(RecordingSink::new(Arc::clone(&good))),
            SinkRef::new(bad),
        ]);
        fanout.open().unwrap();
        // The failing child must not prevent the healthy one from writing.
        let result = fanout.write("still delivered");
        assert!(result.is_err(), "first child error must be surfaced");
        assert_eq!(*good.lock().unwrap(), vec!["still delivered".to_string()]);
    }

    #[test]
    fn fanout_empty_is_noop() {
        let mut fanout = FanoutSink::new(vec![]);
        assert!(fanout.is_empty());
        assert_eq!(fanout.len(), 0);
        fanout.open().unwrap();
        assert!(fanout.write("anything").is_ok());
        assert!(fanout.flush().is_ok());
        assert!(fanout.close().is_ok());
        assert!(fanout.is_healthy());
    }

    #[test]
    fn fanout_health_requires_all_children() {
        let fanout = FanoutSink::new(vec![SinkRef::new(RecordingSink::new(Arc::new(
            Mutex::new(Vec::new()),
        )))]);
        // Default Sink::is_healthy returns true; the fanout aggregates.
        assert!(fanout.is_healthy());
    }
}
