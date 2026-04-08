import { useState, useRef, useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  FileSliders,
  MinusIcon,
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
import { PRESET_TAGS } from "@/types";
import { cn } from "@/lib/utils";

interface AppHeaderProps {
  addonsCount: number;
  batchMode: boolean;
  batchRemoving: boolean;
  batchDisabling: boolean;
  checkingUpdates: boolean;
  loading: boolean;
  selectedCount: number;
  updatingAll: boolean;
  isOffline?: boolean;
  onBatchCancel: () => void;
  onBatchDisable: () => void;
  onBatchRemove: () => void;
  onBatchTag: (tag: string) => void;
  onBatchUpdate: () => void;
  onOpenPacks: () => void;
  onOpenSavedVars: () => void;
  onOpenSettings: () => void;
  onRefresh: () => void;
}

export function AppHeader({
  addonsCount,
  batchMode,
  batchRemoving,
  batchDisabling,
  checkingUpdates,
  loading,
  selectedCount,
  updatingAll,
  isOffline,
  onBatchCancel,
  onBatchDisable,
  onBatchRemove,
  onBatchTag,
  onBatchUpdate,
  onOpenPacks,
  onOpenSavedVars,
  onOpenSettings,
  onRefresh,
}: AppHeaderProps) {
  const [tagMenuOpen, setTagMenuOpen] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [customTagInput, setCustomTagInput] = useState("");
  const tagMenuRef = useRef<HTMLDivElement>(null);
  const customTagInputRef = useRef<HTMLInputElement>(null);

  // Reset confirm/tag state when leaving batch mode
  useEffect(() => {
    if (!batchMode) {
      setConfirmRemove(false); // eslint-disable-line react-hooks/set-state-in-effect -- reset derived state on prop change
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
      className="relative z-20 flex items-center border-b border-white/[0.06] bg-[rgba(10,18,36,0.85)] px-4 py-2 select-none shadow-[0_4px_24px_rgba(0,0,0,0.4),inset_0_1px_0_rgba(255,255,255,0.05)] backdrop-blur-xl backdrop-saturate-[1.2]"
    >
      <div className="absolute right-0 bottom-0 left-0 h-px bg-gradient-to-r from-transparent via-[#c4a44a]/30 to-transparent" />
      <div className="flex items-center gap-2.5">
        <Logo size={20} className="text-[#4dc2e6]" />
        <div className="flex items-center gap-2">
          <h1 className="bg-gradient-to-r from-[#c4a44a] via-[#d4b45a] to-[#c4a44a] bg-clip-text font-heading text-[13px] font-bold uppercase tracking-[0.15em] text-transparent">
            Kalpa
          </h1>
          <div className="h-3 w-px bg-white/[0.12]" />
          <button
            onClick={() => void openUrl("https://esotk.com")}
            className="inline-flex items-center rounded-full border border-white/[0.08] bg-white/[0.04] px-2 py-0.5 font-mono text-[10px] font-medium tracking-wider text-white/40 backdrop-blur-sm transition-colors duration-300 hover:border-[#38bdf8]/20 hover:text-white/60 cursor-pointer"
          >
            esotk.com
          </button>
        </div>
      </div>
      <div className="flex-1" data-tauri-drag-region />
      <div className="flex items-center gap-2">
        {batchMode ? (
          <>
            <span className="mr-2 text-xs font-medium text-primary">{selectedCount} selected</span>
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
                <div className="absolute right-0 top-full mt-1 z-50 min-w-[160px] rounded-md border border-white/[0.06] bg-[rgba(10,18,36,0.95)] backdrop-blur-xl p-1 shadow-lg">
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
                          ? "text-[#c4a44a]"
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
                        className="flex-1 min-w-0 rounded bg-white/[0.04] border border-white/[0.06] px-2 py-1 text-xs text-foreground placeholder:text-muted-foreground/50 outline-none focus:border-[#38bdf8]/40"
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
            {confirmRemove ? (
              <>
                <span className="text-xs text-red-400 font-medium">
                  Remove {selectedCount} addon{selectedCount !== 1 ? "s" : ""}? This cannot be
                  undone.
                </span>
                <Button
                  size="sm"
                  variant="destructive"
                  onClick={onBatchRemove}
                  disabled={batchRemoving}
                >
                  {batchRemoving ? "Removing..." : "Confirm Remove"}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setConfirmRemove(false)}
                  disabled={batchRemoving}
                >
                  Back
                </Button>
              </>
            ) : (
              <>
                <Button size="sm" variant="destructive" onClick={() => setConfirmRemove(true)}>
                  Remove
                </Button>
                <Button size="sm" variant="outline" onClick={onBatchCancel}>
                  Cancel
                </Button>
              </>
            )}
          </>
        ) : (
          <>
            <span
              className="mr-1 text-xs text-muted-foreground/50"
              aria-live="polite"
              aria-atomic="true"
            >
              {addonsCount} addons
              {checkingUpdates && (
                <span className="ml-1 inline-flex items-center gap-1">
                  ·
                  <span className="inline-block size-3 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
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
