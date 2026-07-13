import { memo, useState, useRef, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  Check,
  ChevronDown,
  CloudUpload,
  FileSliders,
  MinusIcon,
  Monitor,
  PackageIcon,
  Plus,
  Power,
  RefreshCwIcon,
  SettingsIcon,
  SquareIcon,
  Tag,
  XIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Logo } from "@/components/ui/logo";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { PRESET_TAGS, type GameInstance } from "@/types";
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
  onOpenSavedVars: () => void;
  onOpenSettings: () => void;
  onOpenLogUpload: () => void;
  onRefresh: () => void;
  onSwitchInstance: (path: string) => void;
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

  const active = instances.find((inst) => inst.addonsPath === activeAddonsPath);
  // A manually-browsed folder won't match any detected instance; still show
  // where installs are going rather than guessing a region label.
  const label = active?.displayLabel ?? "Custom folder";
  const switchable = instances.length > 1;

  return (
    <div className="relative" ref={menuRef}>
      <SimpleTooltip content={switchable ? "Switch ESO instance" : activeAddonsPath} side="bottom">
        <button
          type="button"
          onClick={() => switchable && setOpen((v) => !v)}
          aria-label={`Managing ${label}${switchable ? " — switch instance" : ""}`}
          aria-expanded={open}
          className={cn(
            "inline-flex items-center gap-1.5 rounded-full border border-white/[0.08] bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] font-medium tracking-wider text-white/50 backdrop-blur-sm transition-colors duration-300",
            switchable
              ? "cursor-pointer hover:border-accent-sky/20 hover:text-white/70"
              : "cursor-default"
          )}
        >
          <Monitor className="size-3" />
          {label}
          {switchable && (
            <ChevronDown className={cn("size-2.5 transition-transform", open && "rotate-180")} />
          )}
        </button>
      </SimpleTooltip>
      {open && (
        <div className="absolute left-0 top-full z-50 mt-1 min-w-[200px] rounded-xl border border-white/[0.06] bg-surface-overlay p-1 shadow-lg backdrop-blur-xl">
          {instances.map((inst) => {
            const isActive = inst.addonsPath === activeAddonsPath;
            return (
              <button
                key={inst.id}
                type="button"
                onClick={() => {
                  setOpen(false);
                  if (!isActive) onSwitchInstance(inst.addonsPath);
                }}
                className={cn(
                  "flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-left text-xs font-medium transition-colors hover:bg-white/[0.06]",
                  isActive ? "text-sky-300" : "text-white/80"
                )}
              >
                <Monitor className="size-3 shrink-0 text-muted-foreground" />
                <span className="flex-1 truncate">{inst.displayLabel}</span>
                <span className="text-[10px] text-muted-foreground">
                  {inst.addonCount} addon{inst.addonCount !== 1 ? "s" : ""}
                </span>
                {isActive && <Check className="size-3 shrink-0 text-sky-400" />}
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
  instances,
  activeAddonsPath,
  onSwitchInstance,
  onBatchCancel,
  onBatchDisable,
  onBatchRemove,
  onBatchTag,
  onBatchUpdate,
  onOpenPacks,
  onOpenSavedVars,
  onOpenSettings,
  onOpenLogUpload,
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
      className="relative z-20 flex items-center border-b border-white/[0.06] bg-[color-mix(in_oklab,var(--bg-base)_85%,transparent)] px-4 py-2 select-none shadow-[0_4px_24px_rgba(0,0,0,0.4),inset_0_1px_0_rgba(255,255,255,0.05)] backdrop-blur-xl backdrop-saturate-[1.2]"
    >
      <div className="absolute right-0 bottom-0 left-0 h-px bg-gradient-to-r from-transparent via-primary/30 to-transparent" />
      <div className="flex items-center gap-2.5">
        <Logo size={20} className="text-[#4dc2e6]" />
        <div className="flex items-center gap-2">
          <h1 className="bg-gradient-to-r from-primary via-primary-hover to-primary bg-clip-text font-heading text-[13px] font-bold uppercase tracking-[0.15em] text-transparent">
            Kalpa
          </h1>
          <div className="h-3 w-px bg-white/[0.12]" />
          <button
            onClick={() => void openUrl("https://esotk.com")}
            className="inline-flex items-center rounded-full border border-white/[0.08] bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] font-medium tracking-wider text-white/40 backdrop-blur-sm transition-colors duration-300 hover:border-accent-sky/20 hover:text-white/60 cursor-pointer"
          >
            esotk.com
          </button>
          <InstanceBadge
            instances={instances}
            activeAddonsPath={activeAddonsPath}
            onSwitchInstance={onSwitchInstance}
          />
        </div>
      </div>
      <div className="flex-1" data-tauri-drag-region />
      <div className="flex items-center gap-2">
        {batchMode ? (
          <>
            <span className="mr-2 text-xs font-medium text-primary">
              <CountingNumber
                number={selectedCount}
                transition={{ stiffness: 200, damping: 25 }}
                initiallyStable
              />{" "}
              selected
            </span>
            <SimpleTooltip content={isOffline ? "Updates require an internet connection" : ""}>
              <Button
                size="sm"
                variant="outline"
                onClick={onBatchUpdate}
                disabled={updatingAll || isOffline}
              >
                {updatingAll ? "Updating..." : "Update"}
              </Button>
            </SimpleTooltip>
            <Button
              size="sm"
              variant="outline"
              onClick={onBatchDisable}
              disabled={batchDisabling}
              className="border-amber-500/25 text-amber-400 hover:bg-amber-500/10"
            >
              <Power className="size-3.5 mr-1" />
              {batchDisabling ? "Working..." : "Disable"}
            </Button>
            <div className="relative" ref={tagMenuRef}>
              <Button size="sm" variant="outline" onClick={() => setTagMenuOpen((v) => !v)}>
                <Tag className="size-3.5 mr-1" />
                Tag
              </Button>
              {tagMenuOpen && (
                <div className="absolute right-0 top-full mt-1 z-50 min-w-[160px] rounded-xl border border-white/[0.06] bg-surface-overlay backdrop-blur-xl p-1 shadow-lg">
                  {PRESET_TAGS.map((tag) => (
                    <button
                      key={tag}
                      onClick={() => {
                        onBatchTag(tag);
                        setTagMenuOpen(false);
                        setCustomTagInput("");
                      }}
                      className={cn(
                        "w-full text-left rounded px-2.5 py-1.5 text-xs font-medium transition-colors hover:bg-white/[0.06]",
                        tag === "favorite"
                          ? "text-primary"
                          : tag === "broken"
                            ? "text-red-400"
                            : tag === "testing"
                              ? "text-amber-400"
                              : tag === "essential"
                                ? "text-emerald-400"
                                : "text-violet-400"
                      )}
                    >
                      {tag}
                    </button>
                  ))}
                  <div className="border-t border-white/[0.06] mt-1 pt-1">
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
                        className="flex-1 min-w-0 rounded-[10px] bg-white/[0.04] border border-white/[0.06] hover:border-white/[0.15] px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground/50 outline-none focus:border-accent-sky/40"
                        autoFocus
                      />
                      <button
                        type="submit"
                        disabled={!customTagInput.trim()}
                        className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-white/[0.06] hover:text-foreground disabled:opacity-30 disabled:pointer-events-none"
                        aria-label="Add custom tag"
                      >
                        <Plus className="size-3.5" />
                      </button>
                    </form>
                  </div>
                </div>
              )}
            </div>
            <Button size="sm" variant="destructive" onClick={onBatchRemove}>
              Remove
            </Button>
            <Button size="sm" variant="outline" onClick={onBatchCancel}>
              Cancel
            </Button>
          </>
        ) : (
          <>
            <span
              className="mr-1 text-xs text-muted-foreground/50"
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
                  <span className="inline-block size-3 animate-spin rounded-full border-2 border-white/[0.1] border-t-primary" />
                </span>
              )}
            </span>
            <SimpleTooltip content="Refresh (Ctrl+R)" side="bottom">
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
                size="icon-sm"
                onClick={onOpenLogUpload}
                aria-label="Upload to ESO Logs"
              >
                <CloudUpload />
              </Button>
            </SimpleTooltip>
            <SimpleTooltip content="Settings" side="bottom">
              <Button variant="ghost" size="icon-sm" onClick={onOpenSettings} aria-label="Settings">
                <SettingsIcon />
              </Button>
            </SimpleTooltip>
          </>
        )}
      </div>
      <div className="ml-3 -mr-2 flex items-center">
        <SimpleTooltip content="Minimize" side="bottom">
          <button
            onClick={() => void getCurrentWindow().minimize()}
            className="flex h-8 w-8 items-center justify-center text-muted-foreground/60 transition-colors hover:bg-white/[0.06] hover:text-foreground"
            aria-label="Minimize"
          >
            <MinusIcon className="size-3.5" />
          </button>
        </SimpleTooltip>
        <SimpleTooltip content="Maximize" side="bottom">
          <button
            onClick={() => void getCurrentWindow().toggleMaximize()}
            className="flex h-8 w-8 items-center justify-center text-muted-foreground/60 transition-colors hover:bg-white/[0.06] hover:text-foreground"
            aria-label="Maximize"
          >
            <SquareIcon className="size-3" />
          </button>
        </SimpleTooltip>
        <SimpleTooltip content="Close" side="bottom">
          <button
            onClick={() => void getCurrentWindow().close()}
            className="flex h-8 w-8 items-center justify-center rounded-tr-sm text-muted-foreground/60 transition-colors hover:bg-red-500/20 hover:text-foreground"
            aria-label="Close"
          >
            <XIcon className="size-3.5" />
          </button>
        </SimpleTooltip>
      </div>
    </header>
  );
}

// Memoized: App re-renders on every keystroke and update-progress event; the
// header's props are primitives and stable callbacks, so it bails out of those.
export const AppHeader = memo(AppHeaderBase);
