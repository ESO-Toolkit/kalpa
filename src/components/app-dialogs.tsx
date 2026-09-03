import { lazy, memo, Suspense, useState } from "react";
import type { AddonManifest, AuthUser, GameInstance, UpdateCheckResult } from "@/types";
import { DIALOG_LABELS } from "@/lib/features";
import type { ActiveDialog, DialogId, FeatureId } from "@/lib/features";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Loader2Icon } from "lucide-react";
import { sameAddonsFolder } from "@/lib/removal-queue";

const Packs = lazy(() => import("./packs").then((m) => ({ default: m.Packs })));
const Profiles = lazy(() => import("./profiles").then((m) => ({ default: m.Profiles })));
const Backups = lazy(() => import("./backups").then((m) => ({ default: m.Backups })));
const ApiCompat = lazy(() => import("./api-compat").then((m) => ({ default: m.ApiCompat })));
const Characters = lazy(() => import("./characters").then((m) => ({ default: m.Characters })));
const Settings = lazy(() => import("./settings").then((m) => ({ default: m.Settings })));
const KeyboardShortcuts = lazy(() =>
  import("./keyboard-shortcuts").then((m) => ({ default: m.KeyboardShortcuts }))
);
const SavedVariables = lazy(() =>
  import("./saved-variables").then((m) => ({ default: m.SavedVariables }))
);
const MigrationWizard = lazy(() =>
  import("./migration-wizard").then((m) => ({ default: m.MigrationWizard }))
);
const SafetyCenter = lazy(() =>
  import("./safety-center").then((m) => ({ default: m.SafetyCenter }))
);
const ClientHealthPanel = lazy(() => import("./client-health"));
const SupportDialog = lazy(() =>
  import("./support-dialog").then((m) => ({ default: m.SupportDialog }))
);
const UploaderWorkspace = lazy(() =>
  import("./uploader/uploader-workspace").then((m) => ({ default: m.UploaderWorkspace }))
);

interface AppDialogsProps {
  activeDialog: ActiveDialog;
  addons: AddonManifest[];
  addonsPath: string;
  authUser: AuthUser | null;
  authVerifying: boolean;
  deepLinkPackId: string | null;
  deepLinkShareCode: string | null;
  knownInstances: GameInstance[];
  checkingUpdates: boolean;
  isOffline: boolean;
  lastError: string | null;
  logUploaderMounted: boolean;
  minionDetected: boolean;
  toolbarHidden: FeatureId[];
  /** Takes an updater, not a value — see `handleToolbarHiddenChange` in App.tsx. */
  onToolbarHiddenChange: (update: (prev: FeatureId[]) => FeatureId[]) => void;
  onAuthChange: (user: AuthUser | null) => void;
  onCheckForAppUpdate: () => void;
  onCloseDialog: () => void;
  onInstancesDetected: (instances: GameInstance[]) => void;
  onPathChange: (path: string) => void;
  onRefresh: () => void;
  onShowDialog: (dialog: DialogId) => void;
  updateResults: UpdateCheckResult[];
}

function DialogLoadingFallback({ title, onClose }: { title: string; onClose: () => void }) {
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Loader2Icon className="size-4 animate-spin text-primary" />
            {title}
          </DialogTitle>
        </DialogHeader>
        <div className="flex items-center gap-3 py-6 text-sm text-muted-foreground">
          <Loader2Icon className="size-4 animate-spin text-primary" />
          <span>Loading...</span>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function AppDialogsBase({
  activeDialog,
  addons,
  addonsPath,
  authUser,
  authVerifying,
  deepLinkPackId,
  deepLinkShareCode,
  knownInstances,
  checkingUpdates,
  isOffline,
  lastError,
  logUploaderMounted,
  minionDetected,
  toolbarHidden,
  onToolbarHiddenChange,
  onAuthChange,
  onCheckForAppUpdate,
  onCloseDialog,
  onInstancesDetected,
  onPathChange,
  onRefresh,
  onShowDialog,
  updateResults,
}: AppDialogsProps) {
  // Shared across the Backups and Characters dialogs so a create/restore/delete
  // (or character backup) started in one surface still gates the destructive
  // buttons in the other if the user switches dialogs mid-operation.
  const [backupSurfaceBusy, setBackupSurfaceBusy] = useState(false);

  const visibleDialog = activeDialog && activeDialog !== "log-upload" ? activeDialog : null;
  const shouldRenderUploader = logUploaderMounted || activeDialog === "log-upload";
  if (!visibleDialog && !shouldRenderUploader) return null;

  return (
    <>
      {visibleDialog && (
        <Suspense
          fallback={
            <DialogLoadingFallback title={DIALOG_LABELS[visibleDialog]} onClose={onCloseDialog} />
          }
        >
          {activeDialog === "packs" && (
            <Packs
              addonsPath={addonsPath}
              installedAddons={addons}
              authUser={authUser}
              onAuthChange={onAuthChange}
              onClose={onCloseDialog}
              onRefresh={onRefresh}
              initialPackId={deepLinkPackId}
              initialShareCode={deepLinkShareCode}
            />
          )}

          {activeDialog === "profiles" && (
            <Profiles
              addonsPath={addonsPath}
              instanceLabel={
                knownInstances.find((inst) => inst.addonsPath === addonsPath)?.displayLabel ?? null
              }
              enabledFolders={addons.filter((a) => !a.disabled).map((a) => a.folderName)}
              onClose={onCloseDialog}
              onRefresh={onRefresh}
            />
          )}

          {activeDialog === "backups" && (
            <Backups
              addonsPath={addonsPath}
              onClose={onCloseDialog}
              sharedOpInFlight={backupSurfaceBusy}
              onSharedOpInFlightChange={setBackupSurfaceBusy}
            />
          )}

          {activeDialog === "api-compat" && (
            <ApiCompat addonsPath={addonsPath} onClose={onCloseDialog} />
          )}

          {activeDialog === "characters" && (
            <Characters
              addonsPath={addonsPath}
              onClose={onCloseDialog}
              sharedOpInFlight={backupSurfaceBusy}
              onSharedOpInFlightChange={setBackupSurfaceBusy}
            />
          )}

          {activeDialog === "saved-variables" && (
            <SavedVariables
              addonsPath={addonsPath}
              installedAddons={addons}
              onClose={onCloseDialog}
            />
          )}

          {activeDialog === "settings" && (
            <Settings
              addonsPath={addonsPath}
              authUser={authUser}
              authVerifying={authVerifying}
              knownInstances={knownInstances}
              onAuthChange={onAuthChange}
              onInstancesDetected={onInstancesDetected}
              onPathChange={onPathChange}
              onClose={onCloseDialog}
              onRefresh={onRefresh}
              minionDetected={minionDetected}
              toolbarHidden={toolbarHidden}
              onToolbarHiddenChange={onToolbarHiddenChange}
              onOpenFeature={onShowDialog}
              onShowShortcuts={() => onShowDialog("shortcuts")}
              onCheckForAppUpdate={onCheckForAppUpdate}
              onOpenLogUpload={() => {
                onCloseDialog();
                onShowDialog("log-upload");
              }}
            />
          )}

          {activeDialog === "migration-wizard" && (
            <MigrationWizard
              addonsPath={addonsPath}
              onClose={onCloseDialog}
              onRefresh={onRefresh}
            />
          )}

          {activeDialog === "safety-center" && (
            <SafetyCenter addonsPath={addonsPath} onClose={onCloseDialog} onRefresh={onRefresh} />
          )}

          {activeDialog === "support" && (
            <SupportDialog
              addons={addons}
              addonsPath={addonsPath}
              checkingUpdates={checkingUpdates}
              instanceLabel={
                knownInstances.find((inst) => sameAddonsFolder(inst.addonsPath, addonsPath))
                  ?.displayLabel ?? null
              }
              isOffline={isOffline}
              lastError={lastError}
              onClose={onCloseDialog}
              updateResults={updateResults}
            />
          )}

          {activeDialog === "client-health" && <ClientHealthPanel open onClose={onCloseDialog} />}

          {activeDialog === "shortcuts" && <KeyboardShortcuts onClose={onCloseDialog} />}
        </Suspense>
      )}

      {shouldRenderUploader && (
        <Suspense
          fallback={
            activeDialog === "log-upload" ? (
              <DialogLoadingFallback title={DIALOG_LABELS["log-upload"]} onClose={onCloseDialog} />
            ) : null
          }
        >
          <UploaderWorkspace
            open={activeDialog === "log-upload"}
            authUser={authUser}
            onAuthChange={onAuthChange}
            onClose={onCloseDialog}
            onOpen={() => onShowDialog("log-upload")}
          />
        </Suspense>
      )}
    </>
  );
}

// Memoized: with activeDialog=null this renders nothing but would otherwise
// still re-render (and re-diff its lazy chunks) on every App state change.
export const AppDialogs = memo(AppDialogsBase);
