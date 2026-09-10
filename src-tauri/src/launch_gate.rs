//! Serialises process startup so two launches cannot race each other.
//!
//! `tauri-plugin-single-instance` decides "am I a duplicate?" in two steps that
//! are not atomic: it creates a named mutex, and then, on `ERROR_ALREADY_EXISTS`,
//! looks for the first instance's hidden `-siw` window. The first instance
//! creates that mutex measurably before it creates that window, because the
//! first `CreateWindowExW` on a thread also initialises IME. A second launch
//! landing in the gap finds no window, falls through with **no retry**, and runs
//! on as a full second instance.
//!
//! Two independent Kalpa processes then contend for one WebView2 user-data
//! folder and one cross-process authority lock. The loser burns the authority
//! timeout and exits; the winner can have its WebView2 creation fail, which
//! `tauri-runtime-wry` swallows, leaving a live process with a tray icon and no
//! window that answers every future launch by doing nothing at all.
//!
//! This gate closes that window. It takes its own mutex before the plugin runs
//! and, when another launch already holds it, waits for that launch to publish
//! its single-instance window before continuing.
//!
//! It deliberately **fails open**: a launch that cannot prove another instance
//! is alive proceeds normally. "Occasionally two instances" is a far better
//! failure than "the app will not start", and a genuine second instance is
//! already terminated within the authority timeout by `native_boot`.

/// What this launch is, once the gate has had its say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This launch owns the app: nobody else was starting or running.
    First,
    /// Another instance is live and has published its single-instance window,
    /// so `tauri-plugin-single-instance` will now reliably forward and exit.
    ///
    /// Only ever constructed on Windows — the macOS and Linux single-instance
    /// backends have no mutex-then-window handshake to wait for.
    #[cfg_attr(not(windows), allow(dead_code))]
    Secondary,
}

#[cfg(windows)]
mod imp {
    use super::Outcome;
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, WAIT_TIMEOUT,
    };
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

    /// How long to wait for an in-flight first instance to publish its window.
    const OWNER_PUBLISH_TIMEOUT: Duration = Duration::from_millis(2000);

    /// Distinct from the plugin's own `{identifier}-sim`. Reusing that name
    /// would make the plugin's own `CreateMutexW` see this process's handle,
    /// report `ERROR_ALREADY_EXISTS`, find no window, and fall through — which
    /// would disable single-instance detection permanently.
    const GATE_MUTEX: &str = "com.kalpa.desktop-launch-gate";
    const SINGLE_INSTANCE_CLASS: &str = "com.kalpa.desktop-sic";
    const SINGLE_INSTANCE_WINDOW: &str = "com.kalpa.desktop-siw";

    /// Held for the life of the process. Never closed and never released: the
    /// gate is only meaningful while this instance is running, and dropping it
    /// would silently reopen the race for every later launch.
    static GATE: OnceLock<usize> = OnceLock::new();

    /// Wait for the running instance to publish its single-instance window.
    ///
    /// Split out from the Win32 calls so the three exits are unit-testable: the
    /// window appears, the owner dies first, or the deadline elapses. The real
    /// `owner_still_running` blocks for 25ms per call, which is what paces this
    /// loop in production.
    fn wait_for_owner_window(
        mut window_published: impl FnMut() -> bool,
        mut owner_still_running: impl FnMut() -> bool,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if window_published() {
                return true;
            }
            if !owner_still_running() {
                // The other launch died before publishing. Its mutex is ours
                // now, and this launch is the real first instance.
                return false;
            }
            if Instant::now() >= deadline {
                return false;
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `CreateMutexW` only *sets* `ERROR_ALREADY_EXISTS`; it never clears a
    /// stale one. The plugin reads `GetLastError()` immediately after its own
    /// call on this same thread, so leaving 183 behind would make a legitimate
    /// first instance mistake itself for a duplicate.
    fn clear_last_error() {
        unsafe { SetLastError(ERROR_SUCCESS) };
    }

    fn single_instance_window_exists() -> bool {
        let class = wide(SINGLE_INSTANCE_CLASS);
        let window = wide(SINGLE_INSTANCE_WINDOW);
        unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(window.as_ptr())) }.is_ok()
    }

    /// The gate mutex is created with `bInitialOwner`, so the owning instance
    /// holds it until it exits. A short wait that times out proves the owner is
    /// still alive; anything else means ownership just passed to us.
    fn owner_still_running(gate: HANDLE) -> bool {
        unsafe { WaitForSingleObject(gate, 25) == WAIT_TIMEOUT }
    }

    pub fn acquire() -> Outcome {
        let name = wide(GATE_MUTEX);
        let Ok(gate) = (unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }) else {
            // Without the gate we are exactly where we were before it existed.
            clear_last_error();
            return Outcome::First;
        };
        let contended = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let _ = GATE.set(gate.0 as usize);

        if !contended {
            clear_last_error();
            return Outcome::First;
        }

        let published = wait_for_owner_window(
            single_instance_window_exists,
            || owner_still_running(gate),
            OWNER_PUBLISH_TIMEOUT,
        );
        clear_last_error();
        if published {
            Outcome::Secondary
        } else {
            log::warn!(
                "another launch holds the gate but never published its window in {OWNER_PUBLISH_TIMEOUT:?}; \
                 continuing as a full instance"
            );
            Outcome::First
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_published_window_marks_this_launch_secondary() {
            let mut polls = 0;
            let published = wait_for_owner_window(
                || {
                    polls += 1;
                    polls >= 3
                },
                || true,
                Duration::from_secs(5),
            );
            assert!(published);
        }

        #[test]
        fn an_owner_that_dies_before_publishing_hands_over_the_launch() {
            // The owner crashed mid-startup. Waiting out the full timeout would
            // add two seconds to a launch that is already the first instance.
            let published = wait_for_owner_window(|| false, || false, Duration::from_secs(5));
            assert!(!published);
        }

        #[test]
        fn a_silent_owner_fails_open_at_the_deadline() {
            // Never publishes, never dies: the gate must give up and let this
            // launch proceed rather than leaving the user with no app at all.
            let published = wait_for_owner_window(|| false, || true, Duration::from_millis(50));
            assert!(!published);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Outcome;

    /// Only Windows has the mutex-then-window gap: the macOS and Linux
    /// single-instance backends do not use that handshake.
    pub fn acquire() -> Outcome {
        Outcome::First
    }
}

/// Decide whether this launch owns the app.
///
/// Call this early in `run()`. Anywhere after `Builder::build()` is useless: the
/// single-instance plugin does its work during that call, long before the app's
/// own `setup` closure runs.
pub fn acquire() -> Outcome {
    imp::acquire()
}
