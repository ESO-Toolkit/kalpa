//! Debug-only timing helper shared by the desktop client and native Slint sidecar.
//!
//! Keeping this small utility outside `commands` is important because the
//! sidecar includes the install transaction modules directly, without the
//! desktop command module.

/// Debug-only stopwatch that prints the elapsed time of each install phase to
/// the `tauri dev` console. Release builds get a zero-sized no-op: the fields
/// and the printing are both behind `debug_assertions`, so nothing is measured
/// or logged in a shipped binary.
pub(crate) struct PhaseTimer {
    #[cfg(debug_assertions)]
    label: &'static str,
    #[cfg(debug_assertions)]
    started: std::time::Instant,
    #[cfg(debug_assertions)]
    last: std::cell::Cell<std::time::Instant>,
}

impl PhaseTimer {
    #[cfg(debug_assertions)]
    pub(crate) fn start(label: &'static str) -> Self {
        let now = std::time::Instant::now();
        Self {
            label,
            started: now,
            last: std::cell::Cell::new(now),
        }
    }

    #[cfg(not(debug_assertions))]
    pub(crate) fn start(_label: &'static str) -> Self {
        Self {}
    }

    pub(crate) fn mark(&self, _phase: &str) {
        #[cfg(debug_assertions)]
        {
            let now = std::time::Instant::now();
            let step = now.duration_since(self.last.get());
            self.last.set(now);
            eprintln!(
                "[{}] {_phase}: {:.2}s (total {:.2}s)",
                self.label,
                step.as_secs_f64(),
                now.duration_since(self.started).as_secs_f64()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhaseTimer;

    #[test]
    fn timer_can_start_and_mark_in_all_build_profiles() {
        let timer = PhaseTimer::start("test");
        timer.mark("phase");
    }
}
