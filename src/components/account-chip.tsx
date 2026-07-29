import { useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import { CloudUpload, ExternalLink, Loader2, LogIn, LogOut } from "lucide-react";
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

  const signedIn = authUser !== null;
  const verifyingSignedIn = signedIn && authVerifying;
  const tooltip = signedIn
    ? "ESO Logs account"
    : "Sign in to ESO Logs - upload combat logs and publish packs";
  const sessionPersisted = authUser?.sessionPersisted;

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

  const handleLogin = async () => {
    if (loggingIn) return;
    setLoggingIn(true);
    try {
      const user = await invokeOrThrow<AuthUser>("auth_login");
      onAuthChange(user);
      toast.success(`Signed in as ${user.userName}`, {
        action: {
          label: "Upload a log",
          onClick: onOpenLogUpload,
        },
      });
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

  if (!signedIn) {
    return (
      <SimpleTooltip content={tooltip} side="bottom">
        <button
          type="button"
          onClick={() => void handleLogin()}
          disabled={loggingIn}
          aria-label="Sign in to ESO Logs"
          className={chipClassName}
        >
          {loggingIn ? <Loader2 className="size-3 animate-spin" /> : <LogIn className="size-3" />}
          <span className="hidden min-[860px]:inline">Sign in</span>
        </button>
      </SimpleTooltip>
    );
  }

  return (
    <Popover open={open} onOpenChange={setOpen}>
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
      <PopoverContent side="bottom" align="end" className="w-64 space-y-3">
        <div>
          <PopoverTitle>ESO Logs account</PopoverTitle>
          <PopoverDescription>
            {sessionPersisted === false
              ? "Kalpa couldn't save this sign-in securely - you'll need to sign in again next time you open Kalpa."
              : "Powers log uploads and Pack Hub packs."}
          </PopoverDescription>
        </div>

        <div className="flex items-center gap-2">
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
