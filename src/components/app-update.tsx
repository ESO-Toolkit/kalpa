import { useState, useEffect, useCallback, useRef } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";
import { DownloadIcon, RefreshCwIcon } from "lucide-react";
import { Slide } from "@/components/animate-ui/primitives/effects/slide";
import { CountingNumber } from "@/components/animate-ui/primitives/texts/counting-number";

type AppUpdateState =
  | { status: "idle" }
  | { status: "available"; update: Update }
  | { status: "downloading"; progress: number }
  | { status: "ready" };

const RELEASES_URL = "https://github.com/ESO-Toolkit/kalpa/releases/latest";

export function useAppUpdate() {
  const [state, setState] = useState<AppUpdateState>({ status: "idle" });
  // deb/rpm installs can't self-update; they get pointed at the release page.
  const [selfUpdatable, setSelfUpdatable] = useState(true);
  // Synchronous mirror of the current status. `checkForAppUpdate` must keep a
  // stable identity (App holds it in a ref for the deep-link handler), so it
  // cannot read `state` — and a render-lagging mirror would leave the guard
  // below open for the first moments of a download.
  const statusRef = useRef<AppUpdateState["status"]>("idle");
  const applyState = useCallback((next: AppUpdateState) => {
    statusRef.current = next.status;
    setState(next);
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        setSelfUpdatable(await invoke<boolean>("is_portable_update_supported"));
      } catch {
        // keep the self-update default if the probe fails
      }
    })();
  }, []);

  // Guards against overlapping `check()` calls. Now that a check can be
  // triggered from three places (mount, interval, focus) rather than just
  // one, a slow network response to one must not let a second fire on top of
  // it — e.g. a focus event landing mid-request from the interval.
  const isCheckingRef = useRef(false);

  const checkForAppUpdate = useCallback(
    async (silent = true) => {
      // A download in flight, or an installer already staged, owns the state
      // machine. Overwriting it with a fresh "available" would offer a second
      // concurrent downloadAndInstall on a different Update object, or drop the
      // Restart affordance for an update that is already installed.
      if (statusRef.current === "downloading" || statusRef.current === "ready") {
        if (!silent) {
          toast.info(
            statusRef.current === "downloading"
              ? "An update is already downloading."
              : "An update is ready — restart to apply it."
          );
        }
        return;
      }

      // Only background checks are suppressed. A user-initiated check
      // (silent === false) must always run: App.tsx calls it that way from the
      // deep link and from the Check-for-updates action, and swallowing one
      // would leave the button the user just pressed with no toast and no
      // visible effect at all.
      if (silent && isCheckingRef.current) return;
      isCheckingRef.current = true;

      try {
        const update = await check();
        if (update) {
          applyState({ status: "available", update });
        } else if (!silent) {
          toast.info("You're on the latest version.");
        }
      } catch (e) {
        if (!silent) {
          toast.error(`Update check failed: ${e}`);
        }
      } finally {
        isCheckingRef.current = false;
      }
    },
    [applyState]
  );

  const downloadAndInstall = useCallback(async () => {
    if (state.status !== "available") return;
    const { update } = state;

    if (!selfUpdatable) {
      // Package-manager install (deb/rpm): open the release page instead of
      // attempting an in-place update the updater can't perform.
      try {
        const { openUrl } = await import("@tauri-apps/plugin-opener");
        await openUrl(RELEASES_URL);
      } catch (e) {
        toast.error(`Could not open the releases page: ${e}`);
      }
      return;
    }

    applyState({ status: "downloading", progress: 0 });

    try {
      let downloaded = 0;
      let contentLength = 0;

      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case "Started":
            contentLength = event.data.contentLength ?? 0;
            break;
          case "Progress":
            downloaded += event.data.chunkLength;
            if (contentLength > 0) {
              applyState({
                status: "downloading",
                progress: Math.round((downloaded / contentLength) * 100),
              });
            }
            break;
          case "Finished":
            break;
        }
      });

      applyState({ status: "ready" });
      toast.success("Update installed. Restart to apply.", {
        action: {
          label: "Restart Now",
          onClick: () => relaunch(),
        },
        duration: Infinity,
      });
    } catch (e) {
      applyState({ status: "available", update });
      toast.error(`Update failed: ${e}`);
    }
  }, [state, selfUpdatable, applyState]);

  const restartApp = useCallback(async () => {
    await relaunch();
  }, []);

  // Timestamp of the last check (of any origin), used to throttle the focus
  // trigger below. A ref, not state — it must not cause a render.
  const lastCheckedAtRef = useRef(0);

  // Check on mount (silent) — scheduled to avoid synchronous setState in effect
  useEffect(() => {
    const id = setTimeout(() => {
      lastCheckedAtRef.current = Date.now();
      void checkForAppUpdate(true);
    }, 0);

    // Kalpa is a desktop app people leave open for days at a stretch, and the
    // mount check above only ever fires once per session — a long-lived
    // window would otherwise never learn a new version shipped. Re-check
    // periodically as a floor under that. Every 8 hours lands comfortably
    // inside the 6-12h window CLAUDE.md's no-background-spam rule implies:
    // frequent enough that a multi-day session still notices a release within
    // the same day, infrequent enough that it reads as "occasional", not
    // polling.
    const EIGHT_HOURS_MS = 8 * 60 * 60 * 1000;
    const intervalId = setInterval(() => {
      lastCheckedAtRef.current = Date.now();
      void checkForAppUpdate(true);
    }, EIGHT_HOURS_MS);

    // The moment a user tabs/alt-tabs back into a long-running window is the
    // moment a fresh check is most useful — it's exactly when they'd notice
    // (and act on) an update banner. But window focus fires on every
    // alt-tab, so without a floor this would turn into exactly the
    // background-spam the interval above tries to stay clear of: rapidly
    // refocusing must not fire a check per focus. Skip if the last check
    // (mount, interval, or a prior focus) was within the last 30 minutes —
    // long enough that normal window-switching never re-triggers it, short
    // enough that coming back after a lunch break does.
    const FOCUS_THROTTLE_MS = 30 * 60 * 1000;
    const onFocus = () => {
      if (Date.now() - lastCheckedAtRef.current < FOCUS_THROTTLE_MS) return;
      lastCheckedAtRef.current = Date.now();
      void checkForAppUpdate(true);
    };
    window.addEventListener("focus", onFocus);

    return () => {
      clearTimeout(id);
      clearInterval(intervalId);
      window.removeEventListener("focus", onFocus);
    };
  }, [checkForAppUpdate]);

  return { state, checkForAppUpdate, downloadAndInstall, restartApp };
}

interface AppUpdateBannerProps {
  state: AppUpdateState;
  onDownload: () => void;
  onRestart: () => void;
}

export function AppUpdateBanner({ state, onDownload, onRestart }: AppUpdateBannerProps) {
  if (state.status === "idle") return null;

  return (
    <Slide
      direction="down"
      offset={12}
      transition={{ type: "spring", stiffness: 300, damping: 25 }}
    >
      <div className="flex items-center gap-2 border-b border-structure-06 bg-primary/[0.08] px-3 py-1.5 text-xs">
        {state.status === "available" && (
          <>
            <DownloadIcon className="h-3.5 w-3.5 text-primary" />
            <span className="text-primary">Version {state.update.version} available</span>
            <button
              onClick={onDownload}
              className="ml-auto rounded-md bg-primary/20 px-2 py-0.5 text-primary transition-colors hover:bg-primary/30"
            >
              Update Now
            </button>
          </>
        )}
        {state.status === "downloading" && (
          <>
            <RefreshCwIcon className="h-3.5 w-3.5 animate-spin text-primary" />
            <span className="text-primary">Downloading update...</span>
            <div className="ml-auto h-1.5 w-24 overflow-hidden rounded-full bg-structure-10">
              <div
                className="h-full rounded-full bg-primary transition-all"
                style={{ width: `${state.progress}%` }}
              />
            </div>
            <span className="text-primary">
              <CountingNumber
                number={state.progress}
                transition={{ stiffness: 200, damping: 25 }}
              />
              %
            </span>
          </>
        )}
        {state.status === "ready" && (
          <>
            <DownloadIcon className="h-3.5 w-3.5 text-status-success" />
            <span className="text-status-success">Update ready</span>
            <button
              onClick={onRestart}
              className="ml-auto rounded-md bg-status-success/20 px-2 py-0.5 text-status-success transition-colors hover:bg-status-success/30"
            >
              Restart Now
            </button>
          </>
        )}
      </div>
    </Slide>
  );
}
