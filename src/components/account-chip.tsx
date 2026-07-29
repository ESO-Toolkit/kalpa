import { useCallback, useEffect, useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { CloudUpload, ExternalLink, Loader2, LogIn, LogOut, Zap } from "lucide-react";
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
import { getSettingChecked, setSettings, settingsWritesSettled } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
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
  const [directNextStep, setDirectNextStep] = useState(false);

  const signedIn = authUser !== null;
  const verifyingSignedIn = signedIn && authVerifying;
  const tooltip = signedIn
    ? "ESO Logs profile and upload routing"
    : "Sign in to ESO Logs for Pack Hub and log uploads";
  const sessionPersisted = authUser?.sessionPersisted;
  const directReady = directOptIn && directHasSession && !directReadFailed;

  const refreshDirectUploadState = useCallback(async () => {
    if (!signedIn) {
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
      const readFailed = !manual.ok || !live.ok || tainted;
      setDirectOptIn(!readFailed && !manual.value && !live.value);
      setDirectHasSession(hasSession);
      setDirectReadFailed(readFailed);
    } catch {
      setDirectOptIn(false);
      setDirectHasSession(false);
      setDirectReadFailed(true);
    } finally {
      setDirectChecking(false);
    }
  }, [signedIn]);

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
      const user = await invokeOrThrow<AuthUser>("auth_login");
      onAuthChange(user);
      setDirectNextStep(true);
      setOpen(true);
      toast.success(`Signed in as ${user.userName}`);
      warnIfSessionNotPersisted(user);
    } catch (e) {
      toast.error(`Sign in failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoggingIn(false);
    }
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
      const ok = await setSettings({
        manualUseOfficialUploader: false,
        liveUseOfficialUploader: false,
      });
      if (!ok) {
        toast.error("Couldn't enable direct upload - Kalpa will keep using the official uploader.");
        return;
      }

      const result = await invokeOrThrow<{ sessionPersisted?: boolean }>("uploader_login_esologs");
      warnIfSessionNotPersisted(result);
      const hasSession = await invokeOrThrow<boolean>("uploader_has_session").catch(() => false);
      await refreshDirectUploadState();
      if (hasSession) {
        setDirectNextStep(false);
        toast.success("Direct upload ready - logs can go straight from Kalpa.");
      } else {
        toast.info("Direct upload is still off - Kalpa will use the official uploader.");
      }
    } catch (e) {
      toast.error(`Couldn't enable direct upload: ${getTauriErrorMessage(e)}`);
      await refreshDirectUploadState();
    } finally {
      setDirectEnabling(false);
    }
  };

  const handleOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (!nextOpen) setDirectNextStep(false);
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
          <Button
            type="button"
            size="sm"
            onClick={() => void handleLogin()}
            disabled={loggingIn}
            className="w-full justify-start"
          >
            {loggingIn ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <LogIn className="size-3.5" />
            )}
            {loggingIn ? "Opening ESO Logs..." : "Sign in with ESO Logs"}
          </Button>
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
              <p className="text-xs font-medium text-foreground">
                {directNextStep && !directReady ? "Optional next step" : "Direct upload"}
              </p>
              <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
                {directReady
                  ? "Logs can go straight from Kalpa."
                  : directReadFailed
                    ? "Kalpa could not confirm upload routing."
                    : directOptIn
                      ? "Open ESO Logs once to capture the upload session. Skipping is fine - the official uploader still works."
                      : "Set it up for supported logs, or skip it and keep using the official uploader handoff."}
              </p>
            </div>
            <InfoPill
              color={directChecking ? "muted" : directReady ? "emerald" : "amber"}
              className="shrink-0 text-[10px]"
            >
              {directChecking ? "Checking..." : directReady ? "On" : "Off"}
            </InfoPill>
          </div>
          {!directReady && (
            <div className="mt-2 flex gap-1.5">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void handleEnableDirectUpload()}
                disabled={directChecking || directEnabling}
                className="min-w-0 flex-1 justify-start"
              >
                {directEnabling ? (
                  <Loader2 className="size-3.5 animate-spin" />
                ) : (
                  <Zap className="size-3.5" />
                )}
                {directEnabling ? "Opening ESO Logs..." : "Enable direct upload"}
              </Button>
              {directNextStep && (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setDirectNextStep(false)}
                  className="shrink-0"
                >
                  Skip
                </Button>
              )}
            </div>
          )}
        </div>

        <div className="border-t border-structure-06" />

        <div className="grid gap-1.5">
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
