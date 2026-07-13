import { lazy, memo, Suspense, useState } from "react";
import type { AddonManifest, AuthUser, GameInstance } from "@/types";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Loader2Icon } from "lucide-react";

const Packs = lazy(() => import("./packs").then((m) => ({ default: m.Packs })));
const Profiles = lazy(() => import("./profiles").then((m) => ({ default: m.Profiles })));
const Backups = lazy(() => import("./backups").then((m) => ({ default: m.Backups })));
const ApiCompat = lazy(() => import("./api-compat").then((m) => ({ default: m.ApiCompat })));
const Characters = lazy(() => import("./characters").then((m) => ({ default: m.Characters })));
const Settings = lazy(() => import("./settings").then((m) => ({ default: m.Settings })));
const SavedVariables = lazy(() =>
  import("./saved-variables").then((m) => ({ default: m.SavedVariables }))
);
const MigrationWizard = lazy(() =>
  import("./migration-wizard").then((m) => ({ default: m.MigrationWizard }))
);
const SafetyCenter = lazy(() =>
  import("./safety-center").then((m) => ({ default: m.SafetyCenter }))
);
const UploaderWorkspace = lazy(() =>
  import("./uploader/uploader-workspace").then((m) => ({ default: m.UploaderWorkspace }))
);

type ActiveDialog =
  | "settings"
  | "profiles"
  | "packs"
  | "backups"
  | "api-compat"
  | "characters"
  | "saved-variables"
  | "migration-wizard"
  | "safety-center"
  | "log-upload"
  | null;

const DIALOG_LABELS: Record<Exclude<ActiveDialog, null>, string> = {
  settings: "Settings",
  profiles: "Profiles",
  packs: "Pack Hub",
  backups: "Backups",
  "api-compat": "API Compatibility",
  characters: "Characters",
  "saved-variables": "Saved Variables",
  "migration-wizard": "Migration",
  "safety-center": "Safety Center",
  "log-upload": "Log Uploader",
};

interface AppDialogsProps {
  activeDialog: ActiveDialog;
  addons: AddonManifest[];
  addonsPath: string;
  authUser: AuthUser | null;
  deepLinkPackId: string | null;
  deepLinkShareCode: string | null;
  knownInstances: GameInstance[];
  onAuthChange: (user: AuthUser | null) => void;
  onCheckForAppUpdate: () => void;
  onCloseDialog: () => void;
  onInstancesDetected: (instances: GameInstance[]) => void;
  onPathChange: (path: string) => void;
  onRefresh: () => void;
  onShowDialog: (dialog: Exclude<ActiveDialog, null>) => void;
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
  deepLinkPackId,
  deepLinkShareCode,
  knownInstances,
  onAuthChange,
  onCheckForAppUpdate,
  onCloseDialog,
  onInstancesDetected,
  onPathChange,
  onRefresh,
  onShowDialog,
}: AppDialogsProps) {
  // Shared across the Backups and Characters dialogs so a create/restore/delete
  // (or character backup) started in one surface still gates the destructive
  // buttons in the other if the user switches dialogs mid-operation.
  const [backupSurfaceBusy, setBackupSurfaceBusy] = useState(false);

  if (!activeDialog) return null;
  const fallback = (
    <DialogLoadingFallback title={DIALOG_LABELS[activeDialog]} onClose={onCloseDialog} />
  );

  return (
    <Suspense fallback={fallback}>
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
        <SavedVariables addonsPath={addonsPath} installedAddons={addons} onClose={onCloseDialog} />
      )}

      {activeDialog === "settings" && (
        <Settings
          addonsPath={addonsPath}
          authUser={authUser}
          knownInstances={knownInstances}
          onAuthChange={onAuthChange}
          onInstancesDetected={onInstancesDetected}
          onPathChange={onPathChange}
          onClose={onCloseDialog}
          onRefresh={onRefresh}
          onShowBackups={() => onShowDialog("backups")}
          onShowApiCompat={() => onShowDialog("api-compat")}
          onShowCharacters={() => onShowDialog("characters")}
          onShowMigrationWizard={() => onShowDialog("migration-wizard")}
          onShowSafetyCenter={() => onShowDialog("safety-center")}
          onCheckForAppUpdate={onCheckForAppUpdate}
        />
      )}

      {activeDialog === "migration-wizard" && (
        <MigrationWizard addonsPath={addonsPath} onClose={onCloseDialog} onRefresh={onRefresh} />
      )}

      {activeDialog === "safety-center" && (
        <SafetyCenter addonsPath={addonsPath} onClose={onCloseDialog} onRefresh={onRefresh} />
      )}

      {activeDialog === "log-upload" && (
        <UploaderWorkspace
          authUser={authUser}
          onAuthChange={onAuthChange}
          onClose={onCloseDialog}
        />
      )}
    </Suspense>
  );
}

// Memoized: with activeDialog=null this renders nothing but would otherwise
// still re-render (and re-diff its lazy chunks) on every App state change.
export const AppDialogs = memo(AppDialogsBase);
