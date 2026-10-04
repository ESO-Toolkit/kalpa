import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { CloudUpload, ExternalLink, Loader2, LogIn, LogOut, XCircle, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { InfoPill } from "@/components/ui/info-pill";
import {
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverTitle,
  PopoverTrigger,
} from "@/components/ui/popover";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { getSettingChecked, settingsWritesSettled } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import {
  cancelProfileSignIn,
  setupDirectUploadSession,
  signInWithDirectUploadSetup,
} from "@/lib/account-auth";
import { deriveNativeState } from "@/components/uploader/uploader-shared";
import { cn } from "@/lib/utils";
import type { AuthUser } from "@/types";

interface AccountChipProps {
  authUser: AuthUser | null;
  authVerifying: boolean;
  onAuthChange: (user: AuthUser | null) => void;
  onOpenLogUpload: () => void;
}

function accountInitial(userName: string): string {
  return Array.from(userName.trim())[0]?.toUpperCase() ?? "?";
}

function AccountAvatar({ userName, verifying }: { userName: string; verifying: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex size-3.5 shrink-0 items-center justify-center rounded-full bg-structure-10 text-[9px] font-semibold text-foreground",
        verifying && "motion-safe:animate-pulse"
      )}
    >
      {accountInitial(userName)}
    </span>
  );
}

export function AccountChip({
  authUser,
  authVerifying,
  onAuthChange,
  onOpenLogUpload,
}: AccountChipProps) {
  const [open, setOpen] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [directChecking, setDirectChecking] = useState(false);
  const [directEnabling, setDirectEnabling] = useState(false);
  const [directOptIn, setDirectOptIn] = useState(false);
  const [directHasSession, setDirectHasSession] = useState(false);
  const [directReadFailed, setDirectReadFailed] = useState(false);

  const signedIn = authUser !== null;
  const verifyingSignedIn = signedIn && authVerifying;
  const tooltip = signedIn
    ? "ESO Logs profile and upload routing"
    : "Sign in to ESO Logs for Pack Hub and log uploads";
  const sessionPersisted = authUser?.sessionPersisted;
  const directReady = directOptIn && directHasSession && !directReadFailed;

  // `assumeSignedIn` exists because this is called immediately after sign-in,
  // before `authUser` has propagated — so the captured `signedIn` is still
  // false and the guard below would zero the state instead of reading it. That
  // is why the popover only showed "On" after being closed and reopened.
  const refreshDirectUploadState = useCallback(
    async (assumeSignedIn = false) => {
      if (!signedIn && !assumeSignedIn) {
        setDirectOptIn(false);
        setDirectHasSession(false);
        setDirectReadFailed(false);
        setDirectChecking(false);
        return;
      }

      setDirectChecking(true);
      try {
        await settingsWritesSettled();
        const [manual, live, hasSession] = await Promise.all([
          getSettingChecked<boolean>("manualUseOfficialUploader", false),
          getSettingChecked<boolean>("liveUseOfficialUploader", false),
          invokeOrThrow<boolean>("uploader_has_session").catch(() => false),
        ]);
        const tainted = await invokeOrThrow<boolean>("settings_tainted").catch(() => true);
        const next = deriveNativeState({ manual, live, session: hasSession, tainted });
        // "Direct" here means BOTH paths route natively, so the live opt-out
        // counts too — the readout must not promise direct uploads for a manual
        // path the live key has already opted out of.
        setDirectOptIn(next.nativeOptIn && !next.liveUseOfficial);
        setDirectHasSession(next.hasNativeSession);
        setDirectReadFailed(next.readFailed);
      } catch {
        setDirectOptIn(false);
        setDirectHasSession(false);
        setDirectReadFailed(true);
      } finally {
        setDirectChecking(false);
      }
    },
    [signedIn]
  );

  const chipClassName = useMemo(
    () =>
      cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium backdrop-blur-sm transition-colors duration-300 outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        signedIn
          ? "border-structure-08 bg-structure-04 text-foreground hover:border-accent-sky/20"
          : "border-primary/40 bg-primary/[0.08] text-primary hover:border-primary/60 hover:bg-primary/[0.12]"
      ),
    [signedIn]
  );

  useEffect(() => {
    void (async () => {
      await refreshDirectUploadState();
    })();
  }, [refreshDirectUploadState]);

  useEffect(() => {
    if (!open) return;
    void (async () => {
      await refreshDirectUploadState();
    })();
  }, [open, refreshDirectUploadState]);

  const handleLogin = async () => {
    if (loggingIn) return;
    setLoggingIn(true);
    try {
      const result = await signInWithDirectUploadSetup({
        context: "account",
        onAuthChange,
        // Pass assumeSignedIn: these fire before `authUser` has propagated, so
        // the closure's `signedIn` is still false and would zero the state.
        onDirectUploadSetupComplete: () => refreshDirectUploadState(true),
      });
      if (result.user) {
        await refreshDirectUploadState(true);
      }
    } finally {
      setLoggingIn(false);
    }
  };

  const handleCancelLogin = async () => {
    await cancelProfileSignIn();
    setLoggingIn(false);
  };
  const handleLogout = async () => {
    if (loggingOut) return;
    setLoggingOut(true);
    try {
      await invokeOrThrow("auth_logout");
      onAuthChange(null);
      setOpen(false);
      toast.success("Signed out of ESO Logs");
    } catch (e) {
      toast.error(`Sign out failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoggingOut(false);
    }
  };

  const handleEnableDirectUpload = async () => {
    if (directEnabling) return;
    setDirectEnabling(true);
    try {
      await setupDirectUploadSession({
        context: "account",
        enableWhenSessionExists: true,
      });
      await refreshDirectUploadState();
    } finally {
      setDirectEnabling(false);
    }
  };

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
  };

  if (!signedIn) {
    return (
      <Popover open={open} onOpenChange={handleOpenChange}>
        <SimpleTooltip content={tooltip} side="bottom">
          <PopoverTrigger
            className={chipClassName}
            aria-label="ESO Logs account sign-in"
            aria-haspopup="dialog"
            aria-expanded={open}
            aria-busy={loggingIn}
          >
            {loggingIn ? <Loader2 className="size-3 animate-spin" /> : <LogIn className="size-3" />}
            <span className="hidden min-[860px]:inline">ESO Logs</span>
          </PopoverTrigger>
        </SimpleTooltip>
        <PopoverContent side="bottom" align="end" className="w-72 space-y-3">
          <div>
            <PopoverTitle>ESO Logs account</PopoverTitle>
            <PopoverDescription>
              One account for Pack Hub and log uploads; sign-in is handled through esotk.com.
            </PopoverDescription>
          </div>
          <p className="text-xs text-muted-foreground">
            Installing, updating, profiles, backups and SavedVariables work without signing in.
          </p>
          {loggingIn ? (
            <div className="grid gap-1.5">
              <Button type="button" size="sm" disabled className="w-full justify-start">
                <Loader2 className="size-3.5 animate-spin" />
                Sign-in in progress...
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => void handleCancelLogin()}
                className="w-full justify-start"
              >
                <XCircle className="size-3.5" />
                Cancel sign-in
              </Button>
            </div>
          ) : (
            <Button
              type="button"
              size="sm"
              onClick={() => void handleLogin()}
              className="w-full justify-start"
            >
              <LogIn className="size-3.5" />
              Sign in with ESO Logs
            </Button>
          )}
        </PopoverContent>
      </Popover>
    );
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <SimpleTooltip content={tooltip} side="bottom">
        <PopoverTrigger
          className={chipClassName}
          aria-label={`ESO Logs account: ${authUser.userName}`}
          aria-busy={verifyingSignedIn}
          aria-haspopup="dialog"
          aria-expanded={open}
        >
          <AccountAvatar userName={authUser.userName} verifying={verifyingSignedIn} />
          <span className="hidden max-w-[10ch] truncate min-[860px]:inline">
            {authUser.userName}
          </span>
        </PopoverTrigger>
      </SimpleTooltip>
      <PopoverContent side="bottom" align="end" className="w-72 space-y-3">
        <div>
          <PopoverTitle>ESO Logs account</PopoverTitle>
          <PopoverDescription>
            {sessionPersisted === false
              ? "Kalpa couldn't save this sign-in securely - you'll need to sign in again next time you open Kalpa."
              : "One ESO Logs account for Pack Hub and log uploads; sign-in is handled through esotk.com."}
          </PopoverDescription>
        </div>

        <div className="flex items-start gap-2">
          <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-structure-04 text-sm font-semibold text-foreground">
            {accountInitial(authUser.userName)}
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate text-sm font-medium text-foreground">{authUser.userName}</p>
            {verifyingSignedIn ? (
              <InfoPill color="muted" className="mt-1 w-fit text-[10px]">
                Checking...
              </InfoPill>
            ) : sessionPersisted === false ? (
              <InfoPill color="amber" className="mt-1 w-fit text-[10px]">
                This session only
              </InfoPill>
            ) : (
              <InfoPill color="emerald" className="mt-1 w-fit text-[10px]">
                Session saved
              </InfoPill>
            )}
          </div>
        </div>

        <div className="rounded-lg border border-structure-06 bg-structure-02 p-2.5">
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0">
              <p className="text-xs font-medium text-foreground">Direct upload</p>
              <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
                {directChecking
                  ? "Checking upload routing..."
                  : directReady
                    ? "Logs can go straight from Kalpa."
                    : directReadFailed
                      ? "Kalpa could not confirm upload routing."
                      : directOptIn
                        ? "Your ESO Logs upload session expired. Reconnect to send logs straight from Kalpa."
                        : "Uploads will use the official uploader until direct upload is restored."}
              </p>
            </div>
            {/* "Off" must mean "you opted out", nothing else. An opted-in user
                whose ESO Logs web session simply lapsed was being told "Off",
                which reads as a setting they changed — so they go looking for
                the switch they never touched. The session is a separate
                credential from the sign-in token and expires on its own. */}
            <InfoPill
              color={directChecking ? "muted" : directReady ? "emerald" : "amber"}
              className="shrink-0 text-[10px]"
            >
              {directChecking
                ? "Checking..."
                : directReady
                  ? "On"
                  : directReadFailed
                    ? "Unknown"
                    : directOptIn
                      ? "Session expired"
                      : "Off"}
            </InfoPill>
          </div>
          {/* Only offer "enable" once we KNOW it is off. While the read is in
              flight `directReady` is false, so gating on that alone rendered
              "Checking..." next to an Enable button and told users to switch on
              something that was already on. */}
          {!directReady && !directChecking && (
            <div className="mt-2 flex gap-1.5">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void handleEnableDirectUpload()}
                disabled={directEnabling}
                className="min-w-0 flex-1 justify-start"
              >
                {directEnabling ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Zap className="size-3.5" />
                )}
                {directEnabling
                  ? "Opening ESO Logs..."
                  : directOptIn
                    ? "Reconnect ESO Logs"
                    : "Enable direct upload"}
              </Button>
            </div>
          )}
        </div>

        <div className="border-t border-structure-06" />

        <div className="grid gap-1.5">
          {/* Always offered, pinned or not: this popover is already an
              explicit ESO Logs surface the user opened, so it is the obvious
              second door to the uploader rather than header clutter. */}
          <Button type="button" variant="outline" size="sm" onClick={onOpenLogUpload}>
            <CloudUpload className="size-3.5" />
            Upload a log
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void openUrl("https://esotk.com")}
            className="justify-start"
          >
            <ExternalLink className="size-3.5" />
            Open esotk.com
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => void handleLogout()}
            disabled={loggingOut}
            className="justify-start"
          >
            {loggingOut ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <LogOut className="size-3.5" />
            )}
            Sign out
          </Button>
        </div>
      </PopoverContent>
    </Popover>
  );
}
