import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { CloudUpload, Loader2, LogIn, LogOut, XCircle, Zap } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { getSettingChecked, settingsWritesSettled } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import {
  cancelProfileSignIn,
  setupDirectUploadSession,
  signInWithDirectUploadSetup,
} from "@/lib/account-auth";
import { deriveNativeState } from "@/components/uploader/uploader-shared";
import type { AuthUser } from "@/types";

interface AccountSettingsProps {
  authUser: AuthUser | null;
  authVerifying: boolean;
  onAuthChange: (user: AuthUser | null) => void;
  onOpenLogUpload: () => void;
}

function accountInitial(userName: string): string {
  return Array.from(userName.trim())[0]?.toUpperCase() ?? "?";
}

export function AccountSettings({
  authUser,
  authVerifying,
  onAuthChange,
  onOpenLogUpload,
}: AccountSettingsProps) {
  const [loggingIn, setLoggingIn] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [directChecking, setDirectChecking] = useState(false);
  const [directEnabling, setDirectEnabling] = useState(false);
  const [directOptIn, setDirectOptIn] = useState(false);
  const [directHasSession, setDirectHasSession] = useState(false);
  const [directReadFailed, setDirectReadFailed] = useState(false);

  const directReady = directOptIn && directHasSession && !directReadFailed;

  // `assumeSignedIn` exists because this runs immediately after sign-in, before
  // `authUser` has propagated — the captured value is still null, so the guard
  // would zero the state instead of reading it, and the panel only corrected
  // itself once reopened.
  const refreshDirectUploadState = useCallback(
    async (assumeSignedIn = false) => {
      if (!authUser && !assumeSignedIn) {
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
    [authUser]
  );

  useEffect(() => {
    void (async () => {
      await refreshDirectUploadState();
    })();
  }, [refreshDirectUploadState]);

  const handleLogin = async () => {
    if (loggingIn) return;
    setLoggingIn(true);
    try {
      const result = await signInWithDirectUploadSetup({
        context: "settings",
        onAuthChange,
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
        context: "settings",
        enableWhenSessionExists: true,
      });
      await refreshDirectUploadState();
    } finally {
      setDirectEnabling(false);
    }
  };

  if (!authUser) {
    return (
      <GlassPanel variant="subtle" className="space-y-3 p-3">
        <SectionHeader>ESO Logs Account</SectionHeader>
        <div>
          <p className="text-sm text-foreground">You're not signed in.</p>
          <p className="mt-1 text-xs text-muted-foreground">
            Signing in with ESO Logs lets Kalpa upload your combat logs and publish or vote on Pack
            Hub packs. ESO Logs sign-in is handled through esotk.com. Everything else - installing,
            updating, profiles, backups, SavedVariables - works without an account.
          </p>
        </div>
        {loggingIn ? (
          <div className="flex flex-wrap gap-1.5">
            <Button type="button" size="sm" disabled>
              <Loader2 className="size-3.5 animate-spin" />
              Sign-in in progress...
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => void handleCancelLogin()}
            >
              <XCircle className="size-3.5" />
              Cancel sign-in
            </Button>
          </div>
        ) : (
          <Button type="button" size="sm" onClick={() => void handleLogin()}>
            <LogIn className="size-3.5" />
            Sign in with ESO Logs
          </Button>
        )}
      </GlassPanel>
    );
  }

  const checking = authVerifying;
  const sessionPersisted = authUser.sessionPersisted;

  return (
    <GlassPanel variant="subtle" className="space-y-3 p-3">
      <SectionHeader>ESO Logs Account</SectionHeader>
      <div className="flex items-center gap-3">
        <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-structure-04 text-sm font-semibold text-foreground">
          {accountInitial(authUser.userName)}
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium text-foreground">{authUser.userName}</p>
          {checking ? (
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

      <p className="text-xs text-muted-foreground">
        One ESO Logs account powers Pack Hub and log uploads; sign-in is handled through esotk.com.
      </p>

      {/* Same rule as the account chip: do not offer "enable" until the read has
          actually finished. `directReady` is false while checking, so gating on
          it alone showed an Off panel over a session that was already live. */}
      {!directReady && !directChecking && (
        <div className="rounded-lg border border-structure-06 bg-structure-02 p-2.5">
          <div className="flex items-center justify-between gap-2">
            <div className="min-w-0">
              <p className="text-xs font-medium text-foreground">Direct upload</p>
              <p className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
                {directReadFailed
                  ? "Kalpa could not confirm upload routing. The official uploader handoff still works."
                  : directOptIn
                    ? "Your ESO Logs upload session expired. Reconnect to send logs straight from Kalpa."
                    : "Uploads will use the official uploader until direct upload is restored."}
              </p>
            </div>
            {/* "Off" must mean "you opted out", nothing else — see account-chip.
                An opted-in user whose web session lapsed was told "Off" and went
                hunting for a switch they never touched. */}
            <InfoPill color={directChecking ? "muted" : "amber"} className="shrink-0 text-[10px]">
              {directChecking
                ? "Checking..."
                : directReadFailed
                  ? "Unknown"
                  : directOptIn
                    ? "Session expired"
                    : "Off"}
            </InfoPill>
          </div>
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void handleEnableDirectUpload()}
              disabled={directChecking || directEnabling}
              className="justify-start"
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
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={onOpenLogUpload}
          disabled={checking}
        >
          <CloudUpload className="size-3.5" />
          Upload a log
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => void handleLogout()}
          disabled={checking || loggingOut}
        >
          {loggingOut ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <LogOut className="size-3.5" />
          )}
          Sign out
        </Button>
      </div>

      {sessionPersisted === false && (
        <Alert variant="default">
          <AlertDescription>
            Kalpa couldn't save this sign-in to your system credential store, so you'll be signed
            out when Kalpa closes.
          </AlertDescription>
        </Alert>
      )}

      <p className="text-xs text-muted-foreground">
        To permanently delete your Pack Hub packs, votes and share codes, use the Data tab.
      </p>
    </GlassPanel>
  );
}
