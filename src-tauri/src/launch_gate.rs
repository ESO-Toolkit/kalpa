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
        CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE,
        WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
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

    /// Where `release` can find the gate handle.
    ///
    /// Note this is not what keeps the handle alive: `HANDLE` is `Copy` with no
    /// `Drop` impl (closing is opt-in through `windows_core::Free`, which only
    /// `Owned<T>` invokes), so the local in `acquire` going out of scope neither
    /// closes it nor releases the mutex. Ownership simply lasts until the
    /// process does — which is the intent everywhere except `release`.
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

    /// Belt-and-braces, not the mechanism: what actually keeps the plugin's
    /// detection working is that `GATE_MUTEX` is a different name from its own.
    /// This only avoids leaving a stale `ERROR_ALREADY_EXISTS` in the thread's
    /// last-error slot, since `CreateMutexW` is documented to *set* 183 but not
    /// to clear it, and the plugin reads `GetLastError()` right after its own
    /// call on this same thread.
    fn clear_last_error() {
        unsafe { SetLastError(ERROR_SUCCESS) };
    }

    fn single_instance_window_exists() -> bool {
        let class = wide(SINGLE_INSTANCE_CLASS);
        let window = wide(SINGLE_INSTANCE_WINDOW);
        unsafe { FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(window.as_ptr())) }.is_ok()
    }

    /// What a short wait on the gate says about the launch that owns it.
    enum OwnerState {
        /// Still holding the mutex: that launch is alive and starting.
        Running,
        /// Ownership just passed to us, so this launch is the real first one.
        /// `abandoned` means the owner died without releasing — the crash this
        /// module exists to notice, and worth a line in the log.
        TookOver { abandoned: bool },
        /// The wait itself failed. We cannot tell, so fail open.
        Unknown,
    }

    /// The gate mutex is created with `bInitialOwner`, so the owning instance
    /// holds it until it exits. A short wait that times out proves the owner is
    /// still alive; every other outcome hands us the mutex.
    fn gate_owner_state(gate: HANDLE) -> OwnerState {
        // `WAIT_EVENT` is a newtype, so these cannot be `match` patterns.
        let wait = unsafe { WaitForSingleObject(gate, 25) };
        if wait == WAIT_TIMEOUT {
            OwnerState::Running
        } else if wait == WAIT_OBJECT_0 {
            OwnerState::TookOver { abandoned: false }
        } else if wait == WAIT_ABANDONED {
            OwnerState::TookOver { abandoned: true }
        } else {
            OwnerState::Unknown
        }
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
            || match gate_owner_state(gate) {
                OwnerState::Running => true,
                OwnerState::TookOver { abandoned } => {
                    if abandoned {
                        log::warn!("the previous launch died holding the launch gate; taking over");
                    }
                    false
                }
                OwnerState::Unknown => {
                    log::warn!("could not read the launch gate; continuing as a full instance");
                    false
                }
            },
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

    /// Give up the gate early, so a launch arriving while this process winds
    /// down starts cleanly instead of being told to wait for a corpse.
    ///
    /// Only for the unrecoverable-window path, which raises a modal and then
    /// exits: for as long as that modal is up this process would otherwise
    /// still look like a live owner. Normal shutdown must NOT call this — the
    /// gate is meant to outlive everything except the process itself.
    pub fn release() {
        if let Some(handle) = GATE.get() {
            // SAFETY: the handle came from `CreateMutexW` in `acquire` and is
            // closed exactly once, on a path that exits immediately after.
            unsafe {
                let _ = CloseHandle(HANDLE(*handle as *mut core::ffi::c_void));
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The plugin derives its class and window names from the bundle
        /// identifier, so a rename in `tauri.conf.json` would leave these
        /// constants naming a window that never appears — silently reverting the
        /// gate to the race it exists to close, with no other symptom.
        ///
        /// Keep `tauri-plugin-single-instance`'s `semver` feature OFF: it
        /// appends a version to those names and would break this pairing too.
        #[test]
        fn the_gate_targets_this_bundle_identifier() {
            let config = include_str!("../tauri.conf.json");
            let identifier = config
                .lines()
                .find_map(|line| line.trim().strip_prefix("\"identifier\": \""))
                .and_then(|rest| rest.split('"').next())
                .expect("tauri.conf.json declares an identifier");
            assert_eq!(SINGLE_INSTANCE_CLASS, format!("{identifier}-sic"));
            assert_eq!(SINGLE_INSTANCE_WINDOW, format!("{identifier}-siw"));
            assert_eq!(GATE_MUTEX, format!("{identifier}-launch-gate"));
        }

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

    pub fn release() {}
}

/// Decide whether this launch owns the app.
///
/// Call this early in `run()`. Anywhere after `Builder::build()` is useless: the
/// single-instance plugin does its work during that call, long before the app's
/// own `setup` closure runs.
pub fn acquire() -> Outcome {
    imp::acquire()
}

/// Release the gate ahead of process exit. See `imp::release` — this is for the
/// unrecoverable-window path only, never for a normal shutdown.
pub fn release() {
    imp::release()
}
