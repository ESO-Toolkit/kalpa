import { useState } from "react";
import { toast } from "sonner";
import { CloudUpload, Loader2, LogIn, LogOut } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
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

  const handleLogin = async () => {
    if (loggingIn) return;
    setLoggingIn(true);
    try {
      const user = await invokeOrThrow<AuthUser>("auth_login");
      onAuthChange(user);
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
      toast.success("Signed out of ESO Logs");
    } catch (e) {
      toast.error(`Sign out failed: ${getTauriErrorMessage(e)}`);
    } finally {
      setLoggingOut(false);
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
            Hub packs. Everything else - installing, updating, profiles, backups, SavedVariables -
            works without an account.
          </p>
        </div>
        <Button type="button" size="sm" onClick={() => void handleLogin()} disabled={loggingIn}>
          {loggingIn ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <LogIn className="size-3.5" />
          )}
          Sign in with ESO Logs
        </Button>
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
        One account for two things: uploading combat logs to ESO Logs, and publishing or voting on
        Pack Hub packs.
      </p>

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
