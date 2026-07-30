import { toast } from "sonner";
import { setSettings } from "@/lib/store";
import { getTauriErrorMessage, invokeOrThrow, warnIfSessionNotPersisted } from "@/lib/tauri";
import type { AuthUser } from "@/types";

type SignInContext = "account" | "settings" | "uploader" | "packHub";

export type DirectUploadSetupStatus = "already-ready" | "ready" | "fallback";

interface SignInOptions {
  context: SignInContext;
  onAuthChange: (user: AuthUser | null) => void;
  onDirectUploadSetupComplete?: (status: DirectUploadSetupStatus) => void | Promise<void>;
}

interface DirectUploadSetupOptions {
  context: SignInContext;
  userName?: string;
  profileJustSignedIn?: boolean;
  enableWhenSessionExists?: boolean;
}

export interface SignInWithDirectUploadResult {
  user: AuthUser | null;
  directUploadStatus: DirectUploadSetupStatus | null;
}

const DIRECT_UPLOAD_SETTING = {
  manualUseOfficialUploader: false,
  liveUseOfficialUploader: false,
} as const;

const OFFICIAL_UPLOADER_SETTING = {
  manualUseOfficialUploader: true,
  liveUseOfficialUploader: true,
} as const;

let activeProfileSignIn: Promise<AuthUser> | null = null;

function openingDirectUploadMessage(options: DirectUploadSetupOptions): string {
  return options.profileJustSignedIn
    ? "You're signed in. Finishing ESO Logs sign-in for direct upload."
    : "Finishing ESO Logs sign-in for direct upload.";
}

function directUploadReadyMessage(_options: DirectUploadSetupOptions): string {
  return "Direct upload is on. Logs can go straight from Kalpa.";
}

function directUploadFallbackMessage(context: SignInContext, profileJustSignedIn: boolean): string {
  if (context === "packHub") {
    return profileJustSignedIn
      ? "You're signed in. Kalpa couldn't finish direct upload setup, so uploads will use the official uploader for now."
      : "Kalpa couldn't finish direct upload setup, so uploads will use the official uploader for now.";
  }

  return profileJustSignedIn
    ? "You're signed in. Kalpa couldn't finish direct upload setup, so uploads will use the official uploader for now."
    : "Kalpa couldn't finish direct upload setup, so uploads will use the official uploader for now.";
}

function withStaleTabHint(message: string): string {
  if (/older ESO Logs tab|newest ESO Logs tab|older tab may be stale/i.test(message)) {
    return message;
  }
  return `${message} Use the newest ESO Logs tab; an older tab may be stale.`;
}
function isCancelMessage(message: string): boolean {
  return /cancelled|canceled/i.test(message);
}

function isTimeoutMessage(message: string): boolean {
  return /timed out|timeout/i.test(message);
}

async function preferOfficialUploader(): Promise<void> {
  await setSettings(OFFICIAL_UPLOADER_SETTING);
}

async function enableDirectUploadOrFallback(
  options: DirectUploadSetupOptions,
  toastId?: string | number
): Promise<DirectUploadSetupStatus> {
  const hasSession = await invokeOrThrow<boolean>("uploader_has_session").catch(() => false);
  if (!hasSession) {
    await preferOfficialUploader();
    toast.info(directUploadFallbackMessage(options.context, options.profileJustSignedIn ?? false), {
      id: toastId,
      duration: 9000,
    });
    return "fallback";
  }

  const enabled = await setSettings(DIRECT_UPLOAD_SETTING);
  if (!enabled) {
    toast.info(
      "Direct upload sign-in finished, but Kalpa couldn't save it as your upload route. Uploads will use the official uploader; you can turn direct upload on any time from the account menu.",
      { id: toastId, duration: 9000 }
    );
    return "fallback";
  }

  toast.success(directUploadReadyMessage(options), toastId ? { id: toastId } : undefined);
  return "ready";
}

export async function cancelProfileSignIn(): Promise<boolean> {
  return invokeOrThrow<boolean>("auth_cancel_login").catch(() => false);
}

async function loginProfile(): Promise<AuthUser> {
  if (activeProfileSignIn) {
    toast.info("Starting a new sign-in. Any older ESO Logs tab is stale; use the newest one.", {
      duration: 7000,
    });
    await cancelProfileSignIn();
  }

  const login = invokeOrThrow<AuthUser>("auth_login");
  activeProfileSignIn = login;
  try {
    return await login;
  } finally {
    if (activeProfileSignIn === login) activeProfileSignIn = null;
  }
}

export async function setupDirectUploadSession(
  options: DirectUploadSetupOptions
): Promise<DirectUploadSetupStatus> {
  const hasExistingSession = await invokeOrThrow<boolean>("uploader_has_session").catch(
    () => false
  );
  if (hasExistingSession) {
    if (!options.enableWhenSessionExists) return "already-ready";

    const enabled = await setSettings(DIRECT_UPLOAD_SETTING);
    if (enabled) {
      toast.success(directUploadReadyMessage(options));
      return "ready";
    }

    toast.info(
      "Direct upload session is already signed in, but Kalpa couldn't save it as your upload route. Uploads will use the official uploader; you can turn direct upload on any time from the account menu.",
      { duration: 9000 }
    );
    return "fallback";
  }

  try {
    const silentResult = await invokeOrThrow<{ sessionPersisted?: boolean } | null>(
      "uploader_try_login_esologs_silent"
    );
    if (silentResult) {
      warnIfSessionNotPersisted(silentResult);
      return enableDirectUploadOrFallback(options);
    }
  } catch (error) {
    console.info("[auth] Silent direct upload setup unavailable:", getTauriErrorMessage(error));
  }

  const toastId = toast.loading(openingDirectUploadMessage(options));

  try {
    const result = await invokeOrThrow<{ sessionPersisted?: boolean }>("uploader_login_esologs");
    warnIfSessionNotPersisted(result);
    return enableDirectUploadOrFallback(options, toastId);
  } catch (error) {
    await preferOfficialUploader();
    console.info("[auth] Direct upload setup fell back:", getTauriErrorMessage(error));
    toast.info(directUploadFallbackMessage(options.context, options.profileJustSignedIn ?? false), {
      id: toastId,
      duration: 9000,
    });
    return "fallback";
  }
}

export async function signInWithDirectUploadSetup(
  options: SignInOptions
): Promise<SignInWithDirectUploadResult> {
  let user: AuthUser;
  try {
    user = await loginProfile();
  } catch (error) {
    const message = getTauriErrorMessage(error);
    if (isCancelMessage(message)) {
      toast.info("Sign-in cancelled.");
    } else if (isTimeoutMessage(message)) {
      toast.info(withStaleTabHint(message), { duration: 9000 });
    } else {
      toast.error(`Sign in failed: ${withStaleTabHint(message)}`);
    }
    return { user: null, directUploadStatus: null };
  }

  options.onAuthChange(user);
  warnIfSessionNotPersisted(user);
  toast.success(`Signed in as ${user.userName}`);

  let directUploadStatus: DirectUploadSetupStatus = "fallback";
  try {
    directUploadStatus = await setupDirectUploadSession({
      context: options.context,
      userName: user.userName,
      profileJustSignedIn: true,
      enableWhenSessionExists: true,
    });
  } catch (error) {
    await preferOfficialUploader().catch(() => undefined);
    console.info("[auth] Direct upload setup failed:", getTauriErrorMessage(error));
    toast.info(directUploadFallbackMessage(options.context, true), { duration: 9000 });
  }

  try {
    await options.onDirectUploadSetupComplete?.(directUploadStatus);
  } catch (error) {
    console.info("[auth] Direct upload completion callback failed:", getTauriErrorMessage(error));
  }

  return { user, directUploadStatus };
}
