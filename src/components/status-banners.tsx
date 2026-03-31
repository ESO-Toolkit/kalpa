import { Alert, AlertAction } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { AppUpdateBanner, useAppUpdate } from "./app-update";

interface StatusBannersProps {
  error: string | null;
  isOffline: boolean;
  appUpdateState: ReturnType<typeof useAppUpdate>["state"];
  onDownload: () => void;
  onRestart: () => void;
  onOpenSettings?: () => void;
}

export function StatusBanners({
  error,
  isOffline,
  appUpdateState,
  onDownload,
  onRestart,
  onOpenSettings,
}: StatusBannersProps) {
  return (
    <>
      {error && (
        <Alert variant="destructive" className="rounded-none border-x-0 border-t-0">
          {error}
          {onOpenSettings && (
            <AlertAction>
              <Button variant="outline" size="sm" onClick={onOpenSettings}>
                Open Settings
              </Button>
            </AlertAction>
          )}
        </Alert>
      )}

      {isOffline && (
        <Alert className="rounded-none border-x-0 border-t-0 bg-muted/50 text-muted-foreground">
          You&apos;re offline — updates, installs, and discovery are unavailable until you
          reconnect.
        </Alert>
      )}

      <AppUpdateBanner state={appUpdateState} onDownload={onDownload} onRestart={onRestart} />
    </>
  );
}
