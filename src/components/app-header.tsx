import { memo, useState, useRef, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Check,
  ChevronDown,
  CloudUpload,
  DownloadIcon,
  FileSliders,
  Layers,
  Loader2Icon,
  MinusIcon,
  Monitor,
  PackageIcon,
  Plus,
  Power,
  RefreshCwIcon,
  SettingsIcon,
  SquareIcon,
  Tag,
  Trash2Icon,
  XIcon,
} from "lucide-react";
import { AccountChip } from "@/components/account-chip";
import { Button } from "@/components/ui/button";
import { Logo } from "@/components/ui/logo";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { isMac, modKeyLabel } from "@/lib/platform";
import { PRESET_TAGS, type AuthUser, type GameInstance } from "@/types";
import { cn } from "@/lib/utils";
import { CountingNumber } from "@/components/animate-ui/primitives/texts/counting-number";

interface AppHeaderProps {
  addonsCount: number;
  batchMode: boolean;
  batchDisabling: boolean;
  checkingUpdates: boolean;
  loading: boolean;
  selectedCount: number;
  updatingAll: boolean;
  isOffline?: boolean;
  authUser: AuthUser | null;
  authVerifying: boolean;
  /** Detected ESO instances; the badge renders when at least one is known. */
  instances: GameInstance[];
  /** The AddOns path currently being managed (identifies the active instance). */
  activeAddonsPath: string;
  onBatchCancel: () => void;
  onBatchDisable: () => void;
  onBatchRemove: () => void;
  onBatchTag: (tag: string) => void;
  onBatchUpdate: () => void;
  onOpenPacks: () => void;
  onOpenProfiles: () => void;
  onOpenSavedVars: () => void;
  onOpenSettings: () => void;
  onOpenLogUpload: () => void;
  onAuthChange: (user: AuthUser | null) => void;
  onRefresh: () => void;
  onSwitchInstance: (path: string) => void;
}

/** Compare AddOns folders the way App does. The active path comes from settings
 * as a bare trim, so the same physical folder can arrive with different casing,
 * a '/' instead of '\', or a trailing separator; raw equality would demote a
 * detected instance to "Custom folder" and drop the switcher's checkmark. */
function sameAddonsFolder(a: string, b: string) {
  const normalize = (value: string) =>
    value.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return normalize(a) === normalize(b);
}

/** Header badge showing which ESO install is being managed, with a
 * quick-switch menu when more than one instance exists. A user running
 * live + PTS can otherwise silently install into the wrong game. */
function InstanceBadge({
  instances,
  activeAddonsPath,
  onSwitchInstance,
}: {
  instances: GameInstance[];
  activeAddonsPath: string;
  onSwitchInstance: (path: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  if (instances.length === 0 || !activeAddonsPath) return null;

  const active = instances.find((inst) => sameAddonsFolder(inst.addonsPath, activeAddonsPath));
  // A manually-browsed folder won't match any detected instance; still show
  // where installs are going rather than guessing a region label.
  const label = active?.displayLabel ?? "Custom folder";
  const switchable = instances.length > 1;

  return (
    <div className="relative min-w-0" ref={menuRef}>
      <SimpleTooltip content={switchable ? "Switch ESO instance" : activeAddonsPath} side="bottom">
        <button
          type="button"
          onClick={() => switchable && setOpen((v) => !v)}
          aria-label={`Managing ${label}${switchable ? " — switch instance" : ""}`}
          aria-expanded={open}
          className={cn(
            "inline-flex w-full max-w-40 min-w-0 items-center gap-1.5 rounded-full border border-structure-08 bg-structure-04 px-2 py-0.5 font-mono text-[11px] font-medium tracking-wider whitespace-nowrap text-foreground backdrop-blur-sm transition-colors duration-300",
            switchable
              ? "cursor-pointer hover:border-accent-sky/20 hover:text-foreground"
              : "cursor-default"
          )}
        >
          <Monitor className="size-3 shrink-0" />
          <span className="min-w-0 truncate">{label}</span>
          {switchable && (
            <ChevronDown
              className={cn("size-2.5 shrink-0 transition-transform", open && "rotate-180")}
            />
          )}
        </button>
      </SimpleTooltip>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 min-w-[200px] rounded-xl border border-structure-06 bg-surface-overlay p-1 shadow-lg backdrop-blur-xl">
          {instances.map((inst) => {
            const isActive = sameAddonsFolder(inst.addonsPath, activeAddonsPath);
            return (
              <button
                key={inst.id}
                type="button"
                onClick={() => {
                  setOpen(false);
                  if (!isActive) onSwitchInstance(inst.addonsPath);
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-xs font-medium transition-colors hover:bg-structure-06",
                  isActive ? "text-status-info-soft" : "text-muted-foreground"
                )}
              >
                <Monitor className="size-3 shrink-0 text-muted-foreground" />
                <span className="flex-1 truncate">{inst.displayLabel}</span>
                <span className="text-[10px] text-muted-foreground">
                  {inst.addonCount} addon{inst.addonCount !== 1 ? "s" : ""}
                </span>
                {isActive && <Check className="size-3 shrink-0 text-status-info" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}

function AppHeaderBase({
  addonsCount,
  batchMode,
  batchDisabling,
  checkingUpdates,
  loading,
  selectedCount,
  updatingAll,
  isOffline,
  authUser,
  authVerifying,
  instances,
  activeAddonsPath,
  onSwitchInstance,
  onBatchCancel,
  onBatchDisable,
  onBatchRemove,
  onBatchTag,
  onBatchUpdate,
  onOpenPacks,
  onOpenProfiles,
  onOpenSavedVars,
  onOpenSettings,
  onOpenLogUpload,
  onAuthChange,
  onRefresh,
}: AppHeaderProps) {
  const [tagMenuOpen, setTagMenuOpen] = useState(false);
  const [customTagInput, setCustomTagInput] = useState("");
  const tagMenuRef = useRef<HTMLDivElement>(null);
  const customTagInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!batchMode) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setTagMenuOpen(false);
      setCustomTagInput("");
    }
  }, [batchMode]);

  useEffect(() => {
    if (!tagMenuOpen) return;
    const handler = (e: MouseEvent) => {
      if (tagMenuRef.current && !tagMenuRef.current.contains(e.target as Node)) {
        setTagMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [tagMenuOpen]);
  return (
    <header
      data-tauri-drag-region
      onDoubleClick={(e) => {
        if ((e.target as HTMLElement).closest('button, a, input, [role="button"]')) return;
        void getCurrentWindow().toggleMaximize();
      }}
      className={`relative z-20 flex items-center border-b border-structure-06 bg-[color-mix(in_oklab,var(--bg-base)_85%,transparent)] py-2 select-none shadow-[0_4px_24px_var(--scrim-40),inset_0_1px_0_var(--structure-05)] backdrop-blur-xl backdrop-saturate-[1.2] ${
        // Clear the macOS traffic-light overlay on the left.
        isMac() ? "pr-4 pl-20" : "px-4"
      }`}
    >
      <div className="absolute right-0 bottom-0 left-0 h-px bg-gradient-to-r from-transparent via-primary/30 to-transparent" />
      <div className="flex min-w-0 items-center gap-2.5">
        {/* The brand doubles as the esotk.com link — a separate promo pill
            next to it squeezed the instance badge into truncation on narrow
            windows. An <a> (not <button>) so the h1 stays valid content. */}
        <SimpleTooltip content="Open esotk.com" side="bottom">
          <a
            href="https://esotk.com"
            onClick={(e) => {
              e.preventDefault();
              void openUrl("https://esotk.com");
            }}
            // Middle/ctrl-click must not navigate the webview either.
            onAuxClick={(e) => e.preventDefault()}
            className="flex shrink-0 cursor-pointer items-center gap-2.5 rounded-md outline-none transition-opacity hover:opacity-80 focus-visible:ring-3 focus-visible:ring-accent-sky/20"
          >
            <Logo size={20} className="shrink-0 text-[#4dc2e6]" />
            <h1 className="whitespace-nowrap bg-gradient-to-r from-primary via-primary-hover to-primary bg-clip-text font-heading text-[13px] font-bold uppercase tracking-[0.15em] text-transparent">
              Kalpa
            </h1>
          </a>
        </SimpleTooltip>
        <InstanceBadge
          instances={instances}
          activeAddonsPath={activeAddonsPath}
          onSwitchInstance={onSwitchInstance}
        />
      </div>
      <div className="flex-1" data-tauri-drag-region />
      <div className="flex shrink-0 items-center gap-2">
        {batchMode ? (
          <>
            <span className="mr-2 text-xs font-medium whitespace-nowrap text-primary">
              <CountingNumber
                number={selectedCount}
                transition={{ stiffness: 200, damping: 25 }}
                initiallyStable
              />{" "}
              selected
            </span>
            {/* Labels collapse to icon-only below 960px so the whole batch bar
                fits the minimum window width; tooltips + aria-labels carry the
                names in compact mode. */}
            <SimpleTooltip
              content={
                isOffline ? "Updates require an internet connection" : "Update selected addons"
              }
              side="bottom"
            >
              <Button
                size="sm"
                variant="outline"
                onClick={onBatchUpdate}
                disabled={updatingAll || isOffline}
                aria-label="Update selected addons"
                aria-busy={updatingAll}
              >
                {updatingAll ? (
                  <Loader2Icon className="size-3.5 animate-spin" />
                ) : (
                  <DownloadIcon className="size-3.5" />
                )}
                <span className="hidden min-[960px]:inline">
                  {updatingAll ? "Updating..." : "Update"}
                </span>
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="Disable selected addons" side="bottom">
              <Button
                size="sm"
                variant="outline"
                onClick={onBatchDisable}
                disabled={batchDisabling}
                aria-label="Disable selected addons"
                aria-busy={batchDisabling}
                className="border-status-warning-strong/25 text-status-warning hover:bg-status-warning-strong/10"
              >
                {batchDisabling ? (
                  <Loader2Icon className="size-3.5 animate-spin" />
                ) : (
                  <Power className="size-3.5" />
                )}
                <span className="hidden min-[960px]:inline">
                  {batchDisabling ? "Working..." : "Disable"}
                </span>
              </Button>
            </SimpleTooltip>
            <div className="relative" ref={tagMenuRef}>
              <SimpleTooltip content="Tag selected addons" side="bottom">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setTagMenuOpen((v) => !v)}
                  aria-label="Tag selected addons"
                  aria-haspopup={true}
                  aria-expanded={tagMenuOpen}
                >
                  <Tag className="size-3.5" />
                  <span className="hidden min-[960px]:inline">Tag</span>
                </Button>
              </SimpleTooltip>
              {tagMenuOpen && (
                <div className="absolute right-0 top-full mt-1 z-50 min-w-[160px] rounded-xl border border-structure-06 bg-surface-overlay backdrop-blur-xl p-1 shadow-lg">
                  {PRESET_TAGS.map((tag) => (
                    <button
                      key={tag}
                      onClick={() => {
                        onBatchTag(tag);
                        setTagMenuOpen(false);
                        setCustomTagInput("");
                      }}
                      className={cn(
                        "w-full text-left rounded px-2.5 py-1.5 text-xs font-medium transition-colors hover:bg-structure-06",
                        tag === "favorite"
                          ? "text-primary"
                          : tag === "broken"
                            ? "text-status-danger"
                            : tag === "testing"
                              ? "text-status-warning"
                              : tag === "essential"
                                ? "text-status-success"
                                : "text-status-library"
                      )}
                    >
                      {tag}
                    </button>
                  ))}
                  <div className="border-t border-structure-06 mt-1 pt-1">
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        const tag = customTagInput.trim().toLowerCase();
                        if (!tag) return;
                        onBatchTag(tag);
                        setTagMenuOpen(false);
                        setCustomTagInput("");
                      }}
                      className="flex items-center gap-1 px-1"
                    >
                      <input
                        ref={customTagInputRef}
                        value={customTagInput}
                        onChange={(e) => setCustomTagInput(e.target.value)}
                        placeholder="Custom tag..."
                        className="flex-1 min-w-0 rounded-[10px] bg-structure-04 border border-structure-06 hover:border-structure-15 px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground outline-none focus:border-accent-sky/40"
                        autoFocus
                      />
                      <button
                        type="submit"
                        disabled={!customTagInput.trim()}
                        className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-structure-06 hover:text-foreground disabled:opacity-30 disabled:pointer-events-none"
                        aria-label="Add custom tag"
                      >
                        <Plus className="size-3.5" />
                      </button>
                    </form>
                  </div>
                </div>
              )}
            </div>
            <SimpleTooltip content="Remove selected addons" side="bottom">
              <Button
                size="sm"
                variant="destructive"
                onClick={onBatchRemove}
                aria-label="Remove selected addons"
              >
                <Trash2Icon className="size-3.5" />
                <span className="hidden min-[960px]:inline">Remove</span>
              </Button>
            </SimpleTooltip>
            {/* Cancel keeps its word at every width: an icon here would read as
                a duplicate of the window close button sitting right beside it. */}
            <Button size="sm" variant="outline" onClick={onBatchCancel}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <span
              className="mr-1 text-xs whitespace-nowrap text-muted-foreground"
              aria-live="polite"
              aria-atomic="true"
            >
              <CountingNumber
                number={addonsCount}
                transition={{ stiffness: 200, damping: 25 }}
                initiallyStable
              />{" "}
              addons
              {checkingUpdates && (
                <span className="ml-1 inline-flex items-center gap-1">
                  ·
                  <span className="inline-block size-3 animate-spin rounded-full border-2 border-structure-10 border-t-primary" />
                </span>
              )}
            </span>
            <SimpleTooltip content={`Refresh (${modKeyLabel()}+R)`} side="bottom">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onRefresh}
                disabled={loading}
                aria-label="Refresh addons"
              >
                <RefreshCwIcon className={loading ? "animate-spin" : ""} />
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="Addon Packs" side="bottom">
              <Button variant="ghost" size="icon-sm" onClick={onOpenPacks} aria-label="Addon Packs">
                <PackageIcon />
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="Addon Profiles" side="bottom">
              <Button variant="ghost" size="icon-sm" onClick={onOpenProfiles} aria-label="Profiles">
                <Layers />
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="SavedVariables Manager" side="bottom">
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onOpenSavedVars}
                aria-label="Saved Vars"
              >
                <FileSliders />
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="Upload to ESO Logs" side="bottom">
              <Button
                variant="ghost"
                size="sm"
                onClick={onOpenLogUpload}
                aria-label="Upload to ESO Logs"
              >
                <CloudUpload className="size-3.5" />
                <span className="hidden min-[860px]:inline">Upload logs</span>
              </Button>
            </SimpleTooltip>
            <AccountChip
              authUser={authUser}
              authVerifying={authVerifying}
              onAuthChange={onAuthChange}
              onOpenLogUpload={onOpenLogUpload}
            />
            <SimpleTooltip content="Settings" side="bottom">
              <Button variant="ghost" size="icon-sm" onClick={onOpenSettings} aria-label="Settings">
                <SettingsIcon />
              </Button>
            </SimpleTooltip>
          </>
        )}
      </div>
      {/* macOS renders native traffic-light controls (titleBarStyle: Overlay),
          so the custom Windows/Linux window buttons are hidden there. */}
      {!isMac() && (
        <div className="ml-3 -mr-2 flex shrink-0 items-center">
          <SimpleTooltip content="Minimize" side="bottom">
            <button
              onClick={() => void getCurrentWindow().minimize()}
              className="flex h-8 w-8 items-center justify-center text-muted-foreground/60 transition-colors hover:bg-structure-06 hover:text-foreground"
              aria-label="Minimize"
            >
              <MinusIcon className="size-3.5" />
            </button>
          </SimpleTooltip>
          <SimpleTooltip content="Maximize" side="bottom">
            <button
              onClick={() => void getCurrentWindow().toggleMaximize()}
              className="flex h-8 w-8 items-center justify-center text-muted-foreground/60 transition-colors hover:bg-structure-06 hover:text-foreground"
              aria-label="Maximize"
            >
              <SquareIcon className="size-3" />
            </button>
          </SimpleTooltip>
          <SimpleTooltip content="Close" side="bottom">
            <button
              onClick={() => void getCurrentWindow().close()}
              className="flex h-8 w-8 items-center justify-center rounded-tr-sm text-muted-foreground/60 transition-colors hover:bg-status-danger-strong/20 hover:text-foreground"
              aria-label="Close"
            >
              <XIcon className="size-3.5" />
            </button>
          </SimpleTooltip>
        </div>
      )}
    </header>
  );
}

// Memoized: App re-renders on every keystroke and update-progress event; the
// header's props are primitives and stable callbacks, so it bails out of those.
export const AppHeader = memo(AppHeaderBase);
