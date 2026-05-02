import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { Button } from "@/components/ui/button";
import { Logo } from "@/components/ui/logo";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { Blur } from "@/components/animate-ui/primitives/effects/blur";
import type { GameInstance } from "@/types";

interface SetupWizardProps {
  instances: GameInstance[];
  onSelect: (path: string) => void;
}

export function SetupWizard({ instances, onSelect }: SetupWizardProps) {
  const hasOneDriveWarning = instances.some((inst) => inst.isOnedrive);

  return (
    <div className="flex h-screen items-center justify-center p-8">
      <Blur
        initialBlur={8}
        transition={{ type: "spring", stiffness: 100, damping: 20 }}
        className="w-full max-w-lg"
      >
        <Fade transition={{ type: "spring", stiffness: 100, damping: 20 }}>
          <GlassPanel variant="primary" className="p-6">
            <div className="mb-6 flex items-center gap-3">
              <Logo size={32} />
              <div>
                <h1 className="font-heading text-lg font-bold text-white">Kalpa</h1>
                <p className="text-sm text-muted-foreground">Set up your AddOns folder</p>
              </div>
            </div>

            {hasOneDriveWarning && (
              <div className="mb-4 rounded-lg border border-amber-400/20 bg-amber-400/[0.04] px-3 py-2 text-xs text-amber-400">
                One or more detected folders are inside OneDrive. Cloud sync can sometimes cause
                missing or outdated addons — consider disabling sync for this folder if you see
                issues.
              </div>
            )}

            {instances.length === 0 ? (
              <NoCandidates onSelect={onSelect} />
            ) : instances.length === 1 ? (
              <SingleInstance instance={instances[0]} onSelect={onSelect} />
            ) : (
              <MultipleInstances instances={instances} onSelect={onSelect} />
            )}
          </GlassPanel>
        </Fade>
      </Blur>
    </div>
  );
}

function SingleInstance({
  instance,
  onSelect,
}: {
  instance: GameInstance;
  onSelect: (path: string) => void;
}) {
  return (
    <div className="space-y-4">
      <SectionHeader>Detected AddOns folder</SectionHeader>
      <InstanceCard instance={instance} recommended />
      <div className="flex gap-2">
        <Button className="flex-1" onClick={() => onSelect(instance.addonsPath)}>
          Use this folder
        </Button>
        <BrowseButton onSelect={onSelect} />
      </div>
    </div>
  );
}

function MultipleInstances({
  instances,
  onSelect,
}: {
  instances: GameInstance[];
  onSelect: (path: string) => void;
}) {
  return (
    <div className="space-y-4">
      <SectionHeader>Multiple folders detected</SectionHeader>
      <p className="text-xs text-muted-foreground">
        Select which ESO AddOns folder to manage. The recommended folder is shown first.
      </p>
      <div className="space-y-2">
        {instances.map((instance, i) => (
          <button
            key={instance.id}
            type="button"
            className="w-full text-left"
            onClick={() => onSelect(instance.addonsPath)}
          >
            <InstanceCard instance={instance} recommended={i === 0} />
          </button>
        ))}
      </div>
      <div className="flex justify-end">
        <BrowseButton onSelect={onSelect} />
      </div>
    </div>
  );
}

function NoCandidates({ onSelect }: { onSelect: (path: string) => void }) {
  return (
    <div className="space-y-4">
      <SectionHeader>No AddOns folder found</SectionHeader>
      <p className="text-sm text-muted-foreground">
        We couldn&apos;t automatically detect your ESO AddOns folder. It&apos;s usually located at:
      </p>
      <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] px-3 py-2 font-mono text-xs text-muted-foreground">
        Documents\Elder Scrolls Online\live\AddOns
      </div>
      <p className="text-xs text-muted-foreground">
        Use the button below to browse to your AddOns folder manually.
      </p>
      <BrowseButton onSelect={onSelect} fullWidth />
    </div>
  );
}

function InstanceCard({
  instance,
  recommended,
}: {
  instance: GameInstance;
  recommended?: boolean;
}) {
  const regionLabel = instance.region === "na" ? "NA" : instance.region === "eu" ? "EU" : "PTS";
  const clientLabel = instance.clientType === "steam" ? "Steam" : "Native";

  return (
    <GlassPanel
      variant="subtle"
      className="p-3 transition-colors duration-150 hover:border-white/[0.12]"
    >
      <div className="mb-1 flex items-center gap-2">
        {recommended && <InfoPill color="gold">Recommended</InfoPill>}
        <InfoPill color="sky">{regionLabel}</InfoPill>
        <InfoPill color={instance.clientType === "steam" ? "violet" : "muted"}>
          {clientLabel}
        </InfoPill>
        {instance.isOnedrive && <InfoPill color="amber">OneDrive</InfoPill>}
      </div>
      <p className="truncate font-mono text-xs text-white/80">{instance.addonsPath}</p>
      <p className="mt-1 text-xs text-muted-foreground">
        {instance.addonCount > 0
          ? `${instance.addonCount} addon${instance.addonCount !== 1 ? "s" : ""} detected`
          : "No addons detected"}
        {instance.hasAddonSettings && " · game has been run"}
      </p>
    </GlassPanel>
  );
}

function BrowseButton({
  onSelect,
  fullWidth,
}: {
  onSelect: (path: string) => void;
  fullWidth?: boolean;
}) {
  const [browsing, setBrowsing] = useState(false);

  const handleBrowse = async () => {
    setBrowsing(true);
    try {
      const selected = await open({
        directory: true,
        title: "Select ESO AddOns Folder",
      });
      if (selected) {
        onSelect(selected);
      }
    } catch (e) {
      toast.error(`Failed to open folder picker: ${e}`);
    } finally {
      setBrowsing(false);
    }
  };

  return (
    <Button
      variant="outline"
      className={fullWidth ? "w-full" : undefined}
      disabled={browsing}
      onClick={handleBrowse}
    >
      {browsing ? "Browsing..." : "Choose a different folder..."}
    </Button>
  );
}
