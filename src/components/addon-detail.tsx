import { useState, useMemo, useRef, useEffect } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";
import type { AddonManifest, UpdateCheckResult, InstallResult } from "../types";
import { PRESET_TAGS } from "../types";
import { Button } from "@/components/ui/button";
import { Alert } from "@/components/ui/alert";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { RichDescription } from "@/components/ui/rich-description";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { ExternalLink, Trash2, Check, Power } from "lucide-react";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { AnimatedCheckmark } from "@/components/ui/animated-checkmark";

function relativeDate(ts: number): string {
  const diff = Date.now() - ts;
  if (diff < 0) return "Today"; // future timestamp (clock skew)
  const days = Math.floor(diff / 86400000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 30) return `${days} days ago`;
  return new Date(ts).toLocaleDateString();
}

interface AddonDetailProps {
  addon: AddonManifest | null;
  installedAddons: AddonManifest[];
  addonsPath: string;
  onRemove: () => void;
  onRemoveAddon: (folderName: string) => void;
  onToggleDisable: (folderName: string, currentlyDisabled: boolean) => void;
  updateResult: UpdateCheckResult | null;
  onAddonUpdated: (esouiId: number) => void;
  onTagsChange: (folderName: string, tags: string[]) => void;
  isOffline?: boolean;
}

export function AddonDetail({
  addon,
  installedAddons,
  addonsPath,
  onRemove,
  onRemoveAddon,
  onToggleDisable,
  updateResult,
  onAddonUpdated,
  onTagsChange,
  isOffline,
}: AddonDetailProps) {
  const [updating, setUpdating] = useState(false);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [updateSuccess, setUpdateSuccess] = useState(false);
  const [installingDep, setInstallingDep] = useState<string | null>(null);
  const [justInstalledDeps, setJustInstalledDeps] = useState<Set<string>>(new Set());
  const [removingDep, setRemovingDep] = useState<string | null>(null);
  const [customTagInput, setCustomTagInput] = useState("");
  const customTagRef = useRef<HTMLInputElement>(null);

  const installedSet = useMemo(
    () => new Set(installedAddons.map((a) => a.folderName)),
    [installedAddons]
  );

  const dependents = useMemo(
    () =>
      addon
        ? installedAddons.filter((a) => a.dependsOn.some((dep) => dep.name === addon.folderName))
        : [],
    [installedAddons, addon]
  );

  // Auto-dismiss update success banner after 5 seconds
  useEffect(() => {
    if (!updateSuccess) return;
    const timer = setTimeout(() => setUpdateSuccess(false), 5000);
    return () => clearTimeout(timer);
  }, [updateSuccess]);

  if (!addon) {
    return (
      <Fade
        transition={{ type: "spring", stiffness: 120, damping: 20 }}
        className="relative flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground px-8"
      >
        {/* Ambient glow behind icon */}
        <div className="absolute top-1/2 left-1/2 -translate-x-1/2 -translate-y-1/2 h-[200px] w-[200px] rounded-full bg-[#c4a44a]/[0.04] blur-[60px]" />
        <div className="relative rounded-2xl bg-white/[0.03] border border-white/[0.06] p-5 shadow-[0_0_30px_rgba(196,164,74,0.03)]">
          <svg
            xmlns="http://www.w3.org/2000/svg"
            width="40"
            height="40"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
            strokeLinecap="round"
            strokeLinejoin="round"
            className="text-muted-foreground/30"
          >
            <path d="M14.5 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7.5L14.5 2z" />
            <polyline points="14 2 14 8 20 8" />
            <line x1="16" y1="13" x2="8" y2="13" />
            <line x1="16" y1="17" x2="8" y2="17" />
            <line x1="10" y1="9" x2="8" y2="9" />
          </svg>
        </div>
        <div className="relative text-center">
          <p className="font-heading text-sm font-medium text-foreground/70">No addon selected</p>
          <p className="mt-1 text-xs text-muted-foreground/40">
            Select an addon from the list to view details
          </p>
        </div>
      </Fade>
    );
  }

  const handleUpdate = async () => {
    if (!updateResult) return;
    setUpdating(true);
    setUpdateError(null);
    try {
      await invokeOrThrow<InstallResult>("update_addon", {
        addonsPath,
        esouiId: updateResult.esouiId,
      });
      setUpdateSuccess(true);
      toast.success(`Updated ${addon.title}`);
      onAddonUpdated(updateResult.esouiId);
    } catch (e) {
      setUpdateError(getTauriErrorMessage(e));
    } finally {
      setUpdating(false);
    }
  };

  const handleInstallDep = async (depName: string) => {
    setInstallingDep(depName);
    try {
      await invokeOrThrow<InstallResult>("install_dependency", {
        addonsPath,
        depName,
      });
      toast.success(`Installed ${depName}`);
      setJustInstalledDeps((prev) => new Set(prev).add(depName));
      onRemove(); // refresh addon list
    } catch (e) {
      toast.error(`Failed to install ${depName}: ${getTauriErrorMessage(e)}`);
    } finally {
      setInstallingDep(null);
    }
  };

  const handleRemoveDep = async (depName: string) => {
    setRemovingDep(depName);
    try {
      await invokeOrThrow("remove_addon", {
        addonsPath,
        folderName: depName,
      });
      toast.success(`Removed ${depName}`);
      onRemove(); // refresh addon list
    } catch (e) {
      toast.error(`Failed to remove ${depName}: ${getTauriErrorMessage(e)}`);
    } finally {
      setRemovingDep(null);
    }
  };

  const submitCustomTag = () => {
    const tag = customTagInput.trim().toLowerCase();
    if (!tag) return;
    if (addon.tags.includes(tag)) {
      toast.info("Tag already added");
      setCustomTagInput("");
      return;
    }
    onTagsChange(addon.folderName, [...addon.tags, tag]);
    setCustomTagInput("");
  };

  return (
    <div className="flex-1 overflow-y-auto p-6">
      <h2 className="font-heading text-xl font-semibold bg-gradient-to-r from-[#c4a44a] to-[#d4b45a] bg-clip-text text-transparent">
        {addon.title}
      </h2>
      <div className="mt-1 mb-4 flex items-center gap-2 flex-wrap">
        <InfoPill color="muted">{addon.folderName}/</InfoPill>
        {addon.esouiId && <InfoPill color="sky">ESOUI #{addon.esouiId}</InfoPill>}
      </div>

      {updateSuccess ? (
        <GlassPanel
          variant="subtle"
          className="mb-4 flex items-center gap-2 border-emerald-500/20! bg-emerald-500/[0.04]! p-3"
        >
          <AnimatedCheckmark size={18} color="#34d399" />
          <span className="text-sm text-emerald-400">Updated successfully</span>
        </GlassPanel>
      ) : updateResult?.hasUpdate ? (
        <GlassPanel
          variant="subtle"
          className="mb-4 flex items-center justify-between gap-3 border-amber-500/20! bg-amber-500/[0.04]! p-3"
        >
          <span className="text-sm text-amber-400">
            Update available: {updateResult.currentVersion} &rarr; {updateResult.remoteVersion}
          </span>
          <SimpleTooltip content={isOffline ? "Updates require an internet connection" : ""}>
            <Button onClick={handleUpdate} disabled={updating || isOffline} size="sm">
              {updating ? "Updating..." : "Update"}
            </Button>
          </SimpleTooltip>
        </GlassPanel>
      ) : null}

      {updateError && (
        <Alert variant="destructive" className="mb-4">
          {updateError}
        </Alert>
      )}

      <GlassPanel variant="subtle" className="mb-6 p-3">
        <dl className="grid grid-cols-[120px_1fr] gap-x-4 gap-y-2 text-sm">
          {addon.author && (
            <>
              <dt className="text-muted-foreground/60 font-heading text-xs uppercase tracking-wider">
                Author
              </dt>
              <dd>{addon.author}</dd>
            </>
          )}
          <dt className="text-muted-foreground/60 font-heading text-xs uppercase tracking-wider">
            Version
          </dt>
          <dd>{addon.version || addon.addonVersion || "Unknown"}</dd>
          {addon.apiVersion.length > 0 && (
            <>
              <dt className="text-muted-foreground/60 font-heading text-xs uppercase tracking-wider">
                API Version
              </dt>
              <dd>{addon.apiVersion.join(", ")}</dd>
            </>
          )}
          <dt className="text-muted-foreground/60 font-heading text-xs uppercase tracking-wider">
            Type
          </dt>
          <dd>
            {addon.isLibrary ? (
              <InfoPill color="emerald">Library</InfoPill>
            ) : (
              <InfoPill color="gold">Addon</InfoPill>
            )}
          </dd>
          {addon.esouiLastUpdate > 0 && (
            <>
              <dt className="text-muted-foreground/60 font-heading text-xs uppercase tracking-wider">
                Last Updated
              </dt>
              <dd>{relativeDate(addon.esouiLastUpdate)}</dd>
            </>
          )}
        </dl>
        {addon.esouiId && (
          <div className="mt-3 pt-3 border-t border-white/[0.06]">
            <button
              onClick={() => openUrl(`https://www.esoui.com/downloads/info${addon.esouiId}`)}
              className="inline-flex items-center gap-1.5 rounded-md bg-sky-500/10 px-3 py-1.5 text-xs font-medium text-sky-400 hover:bg-sky-500/20 transition-colors"
            >
              <ExternalLink className="size-3" />
              View on ESOUI
            </button>
          </div>
        )}
      </GlassPanel>

      {/* Tags */}
      <div className="mb-5">
        <SectionHeader className="mb-2">Tags</SectionHeader>
        <div className="flex flex-wrap gap-1.5">
          {PRESET_TAGS.map((tag) => {
            const active = addon.tags.includes(tag);
            return (
              <button
                key={tag}
                aria-label={`${active ? "Remove" : "Add"} tag: ${tag}`}
                aria-pressed={active}
                onClick={() => {
                  const next = active ? addon.tags.filter((t) => t !== tag) : [...addon.tags, tag];
                  onTagsChange(addon.folderName, next);
                }}
                className={cn(
                  "cursor-pointer rounded-md px-2.5 py-1 text-xs font-medium transition-all duration-150 border",
                  active
                    ? tag === "favorite"
                      ? "bg-[#c4a44a]/15 text-[#c4a44a] border-[#c4a44a]/25"
                      : tag === "broken"
                        ? "bg-red-500/15 text-red-400 border-red-500/25"
                        : tag === "testing"
                          ? "bg-amber-500/15 text-amber-400 border-amber-500/25"
                          : tag === "essential"
                            ? "bg-emerald-500/15 text-emerald-400 border-emerald-500/25"
                            : "bg-violet-500/15 text-violet-400 border-violet-500/25"
                    : "bg-white/[0.03] text-muted-foreground/50 border-white/[0.06] hover:bg-white/[0.06] hover:text-muted-foreground"
                )}
              >
                {tag === "favorite" && (active ? "\u2605 " : "\u2606 ")}
                {tag}
              </button>
            );
          })}
          {/* Custom tags */}
          {addon.tags
            .filter((t) => !(PRESET_TAGS as readonly string[]).includes(t))
            .map((tag) => (
              <span
                key={tag}
                className="inline-flex items-center gap-1 rounded-md bg-sky-500/15 text-sky-400 border border-sky-500/25 px-2.5 py-1 text-xs font-medium"
              >
                {tag}
                <button
                  onClick={() =>
                    onTagsChange(
                      addon.folderName,
                      addon.tags.filter((t) => t !== tag)
                    )
                  }
                  className="cursor-pointer ml-0.5 text-sky-400/60 hover:text-sky-400 transition-colors"
                  aria-label={`Remove tag ${tag}`}
                >
                  &times;
                </button>
              </span>
            ))}
          {/* Add custom tag */}
          <form
            className="inline-flex items-center gap-1"
            onSubmit={(e) => {
              e.preventDefault();
              submitCustomTag();
            }}
          >
            <input
              ref={customTagRef}
              type="text"
              value={customTagInput}
              onChange={(e) => setCustomTagInput(e.target.value)}
              placeholder="+ tag"
              className="w-16 focus:w-24 transition-all duration-150 rounded-md bg-white/[0.03] border border-white/[0.06] px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground/30 outline-none focus:border-sky-400/30 focus:bg-white/[0.05]"
            />
            {customTagInput.trim() && (
              <button
                type="submit"
                className="flex items-center justify-center size-6 rounded-md bg-sky-500/10 text-sky-400 hover:bg-sky-500/20 transition-colors text-xs font-bold"
                aria-label="Add tag"
              >
                +
              </button>
            )}
          </form>
        </div>
      </div>

      {addon.description && (
        <div className="mb-5">
          <SectionHeader className="mb-2">Description</SectionHeader>
          <RichDescription text={addon.description} />
        </div>
      )}

      {addon.dependsOn.length > 0 && (
        <div className="mb-5">
          <SectionHeader className="mb-2">Required Dependencies</SectionHeader>
          <div className="space-y-0.5">
            {addon.dependsOn.map((dep) => {
              const installed = installedSet.has(dep.name);
              const justInstalled = justInstalledDeps.has(dep.name);
              return (
                <div
                  key={dep.name}
                  className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-white/[0.03] transition-colors"
                >
                  <span
                    className={cn(
                      "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[10px] font-bold",
                      installed
                        ? "bg-emerald-500/15 text-emerald-400"
                        : "bg-red-500/15 text-red-400"
                    )}
                  >
                    {installed ? "\u2713" : "!"}
                  </span>
                  <div className="flex-1 min-w-0">
                    <span className="truncate block">{dep.name}</span>
                    {dep.min_version !== null && (
                      <span className="text-[11px] text-muted-foreground/50">
                        v{dep.min_version}+
                      </span>
                    )}
                  </div>
                  {installed ? (
                    <SimpleTooltip content={`Remove ${dep.name}`}>
                      <button
                        className="shrink-0 cursor-pointer rounded p-1 text-muted-foreground/30 hover:bg-red-500/10 hover:text-red-400 transition-colors disabled:opacity-50"
                        onClick={() => handleRemoveDep(dep.name)}
                        disabled={removingDep === dep.name}
                      >
                        {removingDep === dep.name ? (
                          <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-red-400" />
                        ) : (
                          <Trash2 className="size-3.5" />
                        )}
                      </button>
                    </SimpleTooltip>
                  ) : (
                    <SimpleTooltip
                      content={
                        isOffline
                          ? "Installs require an internet connection"
                          : `Install ${dep.name}`
                      }
                    >
                      <button
                        className="shrink-0 cursor-pointer rounded bg-sky-500/10 px-2 py-1 text-xs font-medium text-sky-400 hover:bg-sky-500/20 transition-colors disabled:opacity-50"
                        onClick={() => handleInstallDep(dep.name)}
                        disabled={installingDep === dep.name || isOffline}
                      >
                        {installingDep === dep.name ? (
                          <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-sky-400" />
                        ) : justInstalled ? (
                          <span className="flex items-center gap-1 text-emerald-400">
                            <Check className="size-3" />
                            Installed
                          </span>
                        ) : (
                          "Install"
                        )}
                      </button>
                    </SimpleTooltip>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {addon.optionalDependsOn.length > 0 && (
        <div className="mb-5">
          <SectionHeader className="mb-2">Optional Dependencies</SectionHeader>
          <div className="space-y-0.5">
            {addon.optionalDependsOn.map((dep) => {
              const installed = installedSet.has(dep.name);
              const justInstalled = justInstalledDeps.has(dep.name);
              return (
                <div
                  key={dep.name}
                  className="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-white/[0.03] transition-colors"
                >
                  <span
                    className={cn(
                      "flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-[10px]",
                      installed
                        ? "bg-emerald-500/15 text-emerald-400 font-bold"
                        : "bg-white/[0.04] text-muted-foreground/40"
                    )}
                  >
                    {installed ? "\u2713" : "\u2013"}
                  </span>
                  <div className={cn("flex-1 min-w-0", !installed && "text-muted-foreground/60")}>
                    <span className="truncate block">{dep.name}</span>
                    {dep.min_version !== null && (
                      <span className="text-[11px] text-muted-foreground/50">
                        v{dep.min_version}+
                      </span>
                    )}
                  </div>
                  {installed ? (
                    <SimpleTooltip content={`Remove ${dep.name}`}>
                      <button
                        className="shrink-0 cursor-pointer rounded p-1 text-muted-foreground/30 hover:bg-red-500/10 hover:text-red-400 transition-colors disabled:opacity-50"
                        onClick={() => handleRemoveDep(dep.name)}
                        disabled={removingDep === dep.name}
                      >
                        {removingDep === dep.name ? (
                          <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-red-400" />
                        ) : (
                          <Trash2 className="size-3.5" />
                        )}
                      </button>
                    </SimpleTooltip>
                  ) : (
                    <SimpleTooltip
                      content={
                        isOffline
                          ? "Installs require an internet connection"
                          : `Install ${dep.name}`
                      }
                    >
                      <button
                        className="shrink-0 cursor-pointer rounded bg-sky-500/10 px-2 py-1 text-xs font-medium text-sky-400 hover:bg-sky-500/20 transition-colors disabled:opacity-50"
                        onClick={() => handleInstallDep(dep.name)}
                        disabled={installingDep === dep.name || isOffline}
                      >
                        {installingDep === dep.name ? (
                          <span className="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-white/[0.1] border-t-sky-400" />
                        ) : justInstalled ? (
                          <span className="flex items-center gap-1 text-emerald-400">
                            <Check className="size-3" />
                            Installed
                          </span>
                        ) : (
                          "Install"
                        )}
                      </button>
                    </SimpleTooltip>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      <div className="mt-6 border-t border-white/[0.06] pt-4 space-y-3">
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            onClick={() => onToggleDisable(addon.folderName, addon.disabled)}
            className={cn(
              addon.disabled
                ? "border-emerald-500/25 text-emerald-400 hover:bg-emerald-500/10"
                : "border-amber-500/25 text-amber-400 hover:bg-amber-500/10"
            )}
          >
            <Power className="size-4 mr-1.5" />
            {addon.disabled ? "Enable Addon" : "Disable Addon"}
          </Button>
        </div>
        {addon.disabled && dependents.length > 0 && (
          <p className="text-xs text-amber-400/80">
            {dependents.map((d) => d.title).join(", ")}{" "}
            {dependents.length === 1 ? "depends" : "depend"} on this addon and may not work.
          </p>
        )}

        {dependents.length > 0 && (
          <p className="text-xs text-amber-400/70">
            {dependents.map((d) => d.title).join(", ")}{" "}
            {dependents.length === 1 ? "depends" : "depend"} on this addon.
          </p>
        )}
        <Button variant="destructive" onClick={() => onRemoveAddon(addon.folderName)}>
          Remove Addon
        </Button>
      </div>
    </div>
  );
}
