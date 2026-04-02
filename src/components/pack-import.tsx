import { useState } from "react";
import type { PackAddonEntry, SharedPack } from "../types";
import { ImportMode, TYPE_LABELS, TAG_COLORS, PACK_TYPE_PILL_COLOR } from "./pack-constants";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { cn, formatRelativeDate } from "@/lib/utils";
import {
  ImportIcon,
  SearchIcon,
  AlertCircleIcon,
  Loader2Icon,
  XIcon,
  PackageIcon,
  CheckIcon,
  DownloadIcon,
  FileUpIcon,
} from "lucide-react";

export function PackImportView({
  shareCodeInput,
  onShareCodeInputChange,
  resolvingCode,
  importedPack,
  importError,
  installing,
  installProgress,
  installedEsouiIds,
  importedPackAddonsToInstall,
  onResolveCode,
  onImportFile,
  onInstall,
  onClear,
}: {
  shareCodeInput: string;
  onShareCodeInputChange: (value: string) => void;
  resolvingCode: boolean;
  importedPack: SharedPack | null;
  importError: string | null;
  installing: boolean;
  installProgress: { completed: number; failed: number; total: number } | null;
  installedEsouiIds: Set<number>;
  importedPackAddonsToInstall: PackAddonEntry[];
  onResolveCode: (code: string) => void;
  onImportFile: () => void;
  onInstall: () => void;
  onClear: () => void;
}) {
  const [importMode, setImportMode] = useState<ImportMode>("enter-code");

  if (importedPack) {
    const requiredAddons = importedPack.addons.filter((a) => a.required);
    const optionalAddons = importedPack.addons.filter((a) => !a.required);
    const allInstalled = importedPackAddonsToInstall.length === 0;

    return (
      <div className="flex flex-col gap-3 overflow-y-auto max-h-[400px]">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold">{importedPack.title}</h3>
          <Button variant="ghost" size="sm" onClick={onClear}>
            <XIcon className="size-3.5 mr-1" />
            Clear
          </Button>
        </div>

        {importedPack.description && (
          <p className="text-sm text-muted-foreground">{importedPack.description}</p>
        )}

        {/* Preview metadata */}
        <div className="flex items-center gap-2 flex-wrap">
          <InfoPill color={PACK_TYPE_PILL_COLOR[importedPack.packType] ?? "muted"}>
            {TYPE_LABELS[importedPack.packType] ?? importedPack.packType}
          </InfoPill>
          {importedPack.tags.map((tag) => (
            <InfoPill key={tag} color={TAG_COLORS[tag] ?? "muted"}>
              {tag}
            </InfoPill>
          ))}
          <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50">
            <PackageIcon className="size-3" />
            {importedPack.addons.length} addon{importedPack.addons.length !== 1 ? "s" : ""}
          </span>
          {importedPack.sharedBy && (
            <span className="text-[11px] text-muted-foreground/40">
              shared by {importedPack.sharedBy}
            </span>
          )}
          {importedPack.sharedAt && formatRelativeDate(importedPack.sharedAt) && (
            <span className="text-[10px] text-muted-foreground/30">
              {formatRelativeDate(importedPack.sharedAt)}
            </span>
          )}
        </div>

        {/* All installed state */}
        {allInstalled && !installing && (
          <div className="flex items-center gap-2 rounded-lg border border-emerald-400/20 bg-emerald-400/[0.04] p-3">
            <CheckIcon className="size-4 text-emerald-400" />
            <span className="text-sm text-emerald-400 font-medium">
              All addons already installed
            </span>
          </div>
        )}

        {/* Install progress */}
        {installing && installProgress && (
          <div className="rounded-lg border border-[#c4a44a]/20 bg-[#c4a44a]/[0.04] p-3">
            <div className="flex items-center justify-between text-sm mb-2">
              <span className="text-[#c4a44a] font-medium">
                Installing {installProgress.completed + installProgress.failed}/
                {installProgress.total}
              </span>
              {installProgress.failed > 0 && (
                <span className="text-red-400 text-xs">{installProgress.failed} failed</span>
              )}
            </div>
            <div className="h-1 rounded-full bg-white/[0.06]">
              <div
                className="h-full rounded-full bg-[#c4a44a] transition-all duration-300 ease-out"
                style={{
                  width: `${((installProgress.completed + installProgress.failed) / installProgress.total) * 100}%`,
                }}
              />
            </div>
          </div>
        )}

        {/* Addon list */}
        {requiredAddons.length > 0 && (
          <div>
            <SectionHeader>Required ({requiredAddons.length})</SectionHeader>
            <div className="mt-1.5 space-y-1">
              {requiredAddons.map((addon) => (
                <div
                  key={addon.esouiId}
                  className="flex items-center justify-between px-3 py-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02]"
                >
                  <span className="text-sm">{addon.name}</span>
                  {installedEsouiIds.has(addon.esouiId) ? (
                    <span className="text-[10px] text-emerald-400/60 font-medium">Installed</span>
                  ) : (
                    <span className="text-[10px] text-[#c4a44a]/60 font-medium">New</span>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        {optionalAddons.length > 0 && (
          <div>
            <SectionHeader>Optional ({optionalAddons.length})</SectionHeader>
            <div className="mt-1.5 space-y-1">
              {optionalAddons.map((addon) => (
                <div
                  key={addon.esouiId}
                  className="flex items-center justify-between px-3 py-1.5 rounded-lg border border-white/[0.06] bg-white/[0.02]"
                >
                  <span className="text-sm text-muted-foreground">{addon.name}</span>
                  {installedEsouiIds.has(addon.esouiId) && (
                    <span className="text-[10px] text-emerald-400/60 font-medium">Installed</span>
                  )}
                </div>
              ))}
            </div>
          </div>
        )}

        <Button onClick={onInstall} disabled={installing || allInstalled} className="w-full">
          {installing ? (
            <>
              <Loader2Icon className="size-4 animate-spin mr-1.5" />
              Installing...
            </>
          ) : allInstalled ? (
            <>
              <CheckIcon className="size-4 mr-1.5" />
              All Installed
            </>
          ) : (
            <>
              <DownloadIcon className="size-4 mr-1.5" />
              Install {importedPackAddonsToInstall.length} New Addon
              {importedPackAddonsToInstall.length !== 1 ? "s" : ""}
            </>
          )}
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-4 py-4">
      <div className="text-center space-y-1">
        <ImportIcon className="size-8 mx-auto text-muted-foreground/30" />
        <p className="text-sm text-muted-foreground">Import a pack shared by a friend</p>
      </div>

      {/* Import mode toggle */}
      <div className="relative flex p-0.5 rounded-lg bg-white/[0.03] border border-white/[0.06]">
        <div
          className="absolute top-0.5 bottom-0.5 rounded-md bg-white/[0.08] shadow-sm transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)]"
          style={{
            left: importMode === "enter-code" ? "2px" : "calc(50% + 2px)",
            width: "calc(50% - 4px)",
          }}
        />
        {(["enter-code", "import-file"] as ImportMode[]).map((mode) => (
          <button
            key={mode}
            onClick={() => setImportMode(mode)}
            className={cn(
              "relative z-10 flex-1 px-3 py-1.5 rounded-md text-xs font-semibold transition-colors duration-200",
              importMode === mode
                ? "text-foreground"
                : "text-muted-foreground/60 hover:text-muted-foreground"
            )}
          >
            {mode === "enter-code" ? "Enter Code" : "Import File"}
          </button>
        ))}
      </div>

      {importMode === "enter-code" ? (
        <div className="space-y-3">
          <div className="flex gap-2">
            <Input
              placeholder="e.g. HK7M3P"
              value={shareCodeInput}
              onChange={(e) => onShareCodeInputChange(e.target.value.toUpperCase())}
              maxLength={6}
              className="font-mono tracking-widest text-center uppercase"
              autoFocus
            />
            <Button
              onClick={() => onResolveCode(shareCodeInput)}
              disabled={resolvingCode || shareCodeInput.trim().length < 6}
            >
              {resolvingCode ? (
                <Loader2Icon className="size-4 animate-spin" />
              ) : (
                <SearchIcon className="size-4" />
              )}
            </Button>
          </div>
          {resolvingCode && (
            <div className="flex items-center justify-center py-4">
              <div className="inline-block size-5 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-2">
          <p className="text-[11px] text-muted-foreground/50">
            Open a .esopack file shared with you on Discord, forums, or elsewhere.
          </p>
          <Button variant="outline" onClick={onImportFile} className="w-full">
            <FileUpIcon className="size-4 mr-1.5" />
            Open .esopack File
          </Button>
        </div>
      )}

      {importError && (
        <div className="flex items-start gap-2 rounded-lg border border-red-500/20 bg-red-500/[0.04] p-3">
          <AlertCircleIcon className="size-4 text-red-400 shrink-0 mt-0.5" />
          <p className="text-sm text-red-300">{importError}</p>
        </div>
      )}
    </div>
  );
}
