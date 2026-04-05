import { lazy, Suspense } from "react";
import type { AddonManifest, AuthUser, GameInstance } from "@/types";

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
  | null;

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
  onPathChange: (path: string) => void;
  onRefresh: () => void;
  onShowDialog: (dialog: Exclude<ActiveDialog, null>) => void;
}

export function AppDialogs({
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
  onPathChange,
  onRefresh,
  onShowDialog,
}: AppDialogsProps) {
  if (!activeDialog) return null;

  return (
    <Suspense fallback={null}>
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
        <Profiles addonsPath={addonsPath} onClose={onCloseDialog} onRefresh={onRefresh} />
      )}

      {activeDialog === "backups" && <Backups addonsPath={addonsPath} onClose={onCloseDialog} />}

      {activeDialog === "api-compat" && (
        <ApiCompat addonsPath={addonsPath} onClose={onCloseDialog} />
      )}

      {activeDialog === "characters" && (
        <Characters addonsPath={addonsPath} onClose={onCloseDialog} />
      )}

      {activeDialog === "saved-variables" && (
        <SavedVariables addonsPath={addonsPath} installedAddons={addons} onClose={onCloseDialog} />
      )}

      {activeDialog === "settings" && (
        <Settings
          addonsPath={addonsPath}
          knownInstances={knownInstances}
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
    </Suspense>
  );
}
