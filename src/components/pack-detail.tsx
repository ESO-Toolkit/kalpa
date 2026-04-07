import { useState } from "react";
import type { Pack, PackAddonEntry, AuthUser, ShareCodeResponse } from "../types";
import { ShareMode, TYPE_LABELS, TAG_COLORS } from "./pack-constants";
import { Button } from "@/components/ui/button";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { cn, decodeHtml, formatRelativeDate, formatRelativeExpiry } from "@/lib/utils";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  PencilIcon,
  TrashIcon,
  ShareIcon,
  CopyIcon,
  CheckIcon,
  ArrowUpIcon,
  Loader2Icon,
  FileDownIcon,
  ClockIcon,
  PlusIcon,
  ExternalLinkIcon,
} from "lucide-react";

// ── Detail View ───────────────────────────────────────────────────────────

export function PackDetailView({
  pack,
  loading,
  installing,
  installProgress,
  installSucceeded,
  selectedAddons,
  installedEsouiIds,
  votingPacks,
  onToggleAddon,
  onSelectAllOptional,
  onVote,
  authUser,
  canEdit,
  onEdit,
  onDelete,
  deletingPack,
  showShareSection,
  onToggleShare,
  shareResult,
  generatingShare,
  copiedField,
  onGenerateShareCode,
  onRegenerateShareCode,
  onCopyToClipboard,
  onExportFile,
}: {
  pack: Pack | null;
  loading: boolean;
  installing: boolean;
  installProgress: { completed: number; failed: number; total: number } | null;
  installSucceeded: boolean;
  selectedAddons: Set<number>;
  installedEsouiIds: Set<number>;
  votingPacks: Set<string>;
  onToggleAddon: (esouiId: number, required: boolean) => void;
  onSelectAllOptional: (select: boolean) => void;
  onVote: (packId: string) => void;
  authUser: AuthUser | null;
  canEdit: boolean;
  onEdit: () => void;
  onDelete: () => void;
  deletingPack: boolean;
  showShareSection: boolean;
  onToggleShare: () => void;
  shareResult: ShareCodeResponse | null;
  generatingShare: boolean;
  copiedField: "code" | "link" | null;
  onGenerateShareCode: () => void;
  onRegenerateShareCode: () => void;
  onCopyToClipboard: (text: string, field: "code" | "link") => Promise<void>;
  onExportFile: () => void;
}) {
  const [shareMode, setShareMode] = useState<ShareMode>("private-link");
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  if (loading || !pack) {
    return (
      <div className="flex items-center justify-center py-12">
        <div className="inline-block size-6 animate-spin rounded-full border-2 border-white/[0.1] border-t-[#c4a44a]" />
      </div>
    );
  }

  const requiredAddons = pack.addons.filter((a) => a.required);
  const optionalAddons = pack.addons.filter((a) => !a.required);

  return (
    <div className="flex flex-col gap-3 overflow-y-auto max-h-[400px]">
      {pack.description && (
        <p className="text-sm text-muted-foreground">{decodeHtml(pack.description)}</p>
      )}

      <div className="flex items-center gap-2 flex-wrap">
        <InfoPill color="muted">{TYPE_LABELS[pack.packType] ?? pack.packType}</InfoPill>
        {pack.tags.map((tag) => (
          <InfoPill key={tag} color={TAG_COLORS[tag] ?? "muted"}>
            {tag}
          </InfoPill>
        ))}
        {!pack.isAnonymous && (
          <span className="text-xs text-muted-foreground/50">by {decodeHtml(pack.authorName)}</span>
        )}
        {/* Relative dates */}
        {(() => {
          const dateIso =
            pack.updatedAt && pack.updatedAt !== pack.createdAt ? pack.updatedAt : pack.createdAt;
          const label = pack.updatedAt && pack.updatedAt !== pack.createdAt ? "Updated" : "Created";
          const relative = dateIso ? formatRelativeDate(dateIso) : "";
          return relative ? (
            <SimpleTooltip content={dateIso}>
              <span className="text-[10px] text-muted-foreground/40">
                {label} {relative}
              </span>
            </SimpleTooltip>
          ) : null;
        })()}
        <div className="flex items-center gap-1.5 ml-auto">
          {canEdit && (
            <Button variant="outline" size="sm" onClick={onEdit}>
              <PencilIcon className="size-3.5 mr-1.5" />
              Edit
            </Button>
          )}
          {canEdit && (
            <>
              {showDeleteConfirm ? (
                <div className="flex items-center gap-1.5 rounded-lg border border-red-500/20 bg-red-500/[0.06] px-2.5 py-1">
                  <span className="text-[11px] text-red-400 font-medium">Delete this pack?</span>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setShowDeleteConfirm(false)}
                    className="h-6 px-2 text-[10px]"
                  >
                    Cancel
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      setShowDeleteConfirm(false);
                      onDelete();
                    }}
                    disabled={deletingPack}
                    className="h-6 px-2 text-[10px] border-red-500/30 text-red-400 hover:bg-red-500/10"
                  >
                    {deletingPack ? <Loader2Icon className="size-3 animate-spin" /> : "Delete"}
                  </Button>
                </div>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setShowDeleteConfirm(true)}
                  className="text-red-400/60 hover:text-red-400 hover:border-red-500/30"
                >
                  <TrashIcon className="size-3.5 mr-1.5" />
                  Delete
                </Button>
              )}
            </>
          )}
          <Button
            variant="outline"
            size="sm"
            onClick={onToggleShare}
            className={cn(showShareSection && "border-[#c4a44a]/30 bg-[#c4a44a]/[0.06]")}
          >
            <ShareIcon className="size-3.5 mr-1.5" />
            Share
          </Button>
        </div>
        <SimpleTooltip content={authUser ? (pack.userVoted ? "Remove vote" : "Upvote this pack") : "Sign in to vote"}>
        <button
          onClick={() => onVote(pack.id)}
          disabled={votingPacks.has(pack.id)}
          className={cn(
            "group/vote relative flex items-center gap-1.5 text-sm font-medium rounded-full px-3 py-1.5 transition-all duration-200 border",
            votingPacks.has(pack.id) && "opacity-60 pointer-events-none",
            pack.userVoted
              ? "text-[#c4a44a] bg-[#c4a44a]/[0.12] border-[#c4a44a]/30 hover:bg-[#c4a44a]/[0.2] shadow-[0_0_12px_rgba(196,164,74,0.15)]"
              : "text-muted-foreground/60 bg-white/[0.03] border-white/[0.08] hover:text-[#c4a44a] hover:border-[#c4a44a]/20 hover:bg-[#c4a44a]/[0.06]"
          )}
        >
          <ArrowUpIcon
            className={cn(
              "size-4 transition-all duration-200",
              pack.userVoted ? "-translate-y-[1px]" : "group-hover/vote:-translate-y-[1px]"
            )}
            strokeWidth={pack.userVoted ? 2.5 : 2}
          />
          <span>{pack.voteCount > 0 ? pack.voteCount : 0}</span>
        </button>
        </SimpleTooltip>
      </div>

      {/* Share section — Task C: two-mode toggle */}
      {showShareSection && (
        <div className="rounded-lg border border-white/[0.06] bg-white/[0.02] p-3 space-y-3">
          {/* Segmented control: Private Link vs Export File */}
          <div className="relative flex p-0.5 rounded-lg bg-white/[0.03] border border-white/[0.06]">
            <div
              className="absolute top-0.5 bottom-0.5 rounded-md bg-white/[0.08] shadow-sm transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)]"
              style={{
                left: shareMode === "private-link" ? "2px" : "calc(50% + 2px)",
                width: "calc(50% - 4px)",
              }}
            />
            {(["private-link", "export-file"] as ShareMode[]).map((mode) => (
              <button
                key={mode}
                onClick={() => setShareMode(mode)}
                className={cn(
                  "relative z-10 flex-1 px-3 py-1.5 rounded-md text-xs font-semibold transition-colors duration-200",
                  shareMode === mode
                    ? "text-foreground"
                    : "text-muted-foreground/60 hover:text-muted-foreground"
                )}
              >
                {mode === "private-link" ? "Private Link" : "Export File"}
              </button>
            ))}
          </div>

          {shareMode === "private-link" ? (
            <div className="space-y-2">
              <p className="text-[11px] text-muted-foreground/50">
                Share privately — only people with this code can import this pack.
              </p>
              {shareResult ? (
                <div className="space-y-2">
                  <div className="flex items-center gap-2">
                    <code className="flex-1 rounded-md bg-white/[0.05] px-3 py-2 text-center font-mono text-lg font-bold tracking-[0.3em] text-[#c4a44a]">
                      {shareResult.code}
                    </code>
                    <SimpleTooltip content="Copy share code">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => onCopyToClipboard(shareResult.code, "code")}
                      >
                        {copiedField === "code" ? (
                          <CheckIcon className="size-3.5 text-emerald-400" />
                        ) : (
                          <CopyIcon className="size-3.5" />
                        )}
                      </Button>
                    </SimpleTooltip>
                  </div>
                  <div className="flex items-center gap-2">
                    <code className="flex-1 truncate rounded-md bg-white/[0.05] px-3 py-1.5 text-xs text-muted-foreground/60">
                      {shareResult.deepLink}
                    </code>
                    <SimpleTooltip content="Copy deep link">
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => onCopyToClipboard(shareResult.deepLink, "link")}
                      >
                        {copiedField === "link" ? (
                          <CheckIcon className="size-3.5 text-emerald-400" />
                        ) : (
                          <CopyIcon className="size-3.5" />
                        )}
                      </Button>
                    </SimpleTooltip>
                  </div>
                  <div className="flex items-center justify-between">
                    <p className="flex items-center gap-1.5 text-[10px] text-muted-foreground/40">
                      <ClockIcon className="size-3" />
                      {shareResult.expiresAt
                        ? formatRelativeExpiry(shareResult.expiresAt)
                        : "Expires in ~7 days"}
                    </p>
                    <button
                      onClick={onRegenerateShareCode}
                      className="text-[10px] text-muted-foreground/40 hover:text-muted-foreground transition-colors"
                    >
                      Regenerate
                    </button>
                  </div>
                </div>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={onGenerateShareCode}
                  disabled={generatingShare}
                  className="w-full"
                >
                  {generatingShare ? (
                    <Loader2Icon className="size-3.5 animate-spin mr-1.5" />
                  ) : (
                    <ShareIcon className="size-3.5 mr-1.5" />
                  )}
                  Generate Code
                </Button>
              )}
            </div>
          ) : (
            <div className="space-y-2">
              <p className="text-[11px] text-muted-foreground/50">
                Save as a .esopack file to share on Discord, forums, or privately.
              </p>
              <Button variant="outline" size="sm" onClick={onExportFile} className="w-full">
                <FileDownIcon className="size-3.5 mr-1.5" />
                Export .esopack File
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Install progress bar */}
      {(installing && installProgress) || installSucceeded ? (
        <div
          className={cn(
            "rounded-lg border p-3",
            installSucceeded
              ? "border-emerald-400/20 bg-emerald-400/[0.04]"
              : "border-[#c4a44a]/20 bg-[#c4a44a]/[0.04]"
          )}
        >
          {installProgress && (
            <div className="flex items-center justify-between text-sm mb-2">
              <span className="text-[#c4a44a] font-medium">
                Installing {installProgress.completed + installProgress.failed}/
                {installProgress.total}
              </span>
              {installProgress.failed > 0 && (
                <span className="text-red-400 text-xs">{installProgress.failed} failed</span>
              )}
            </div>
          )}
          {installSucceeded && !installProgress && (
            <div className="flex items-center gap-2 text-sm mb-2">
              <CheckIcon className="size-4 text-emerald-400" />
              <span className="text-emerald-400 font-medium">Installed successfully</span>
            </div>
          )}
          <div className="h-1 rounded-full bg-white/[0.06]">
            <div
              className={cn(
                "h-full rounded-full transition-all duration-300 ease-out",
                installSucceeded ? "bg-emerald-400" : "bg-[#c4a44a]"
              )}
              style={{
                width: installSucceeded
                  ? "100%"
                  : installProgress
                    ? `${((installProgress.completed + installProgress.failed) / installProgress.total) * 100}%`
                    : "0%",
              }}
            />
          </div>
        </div>
      ) : null}

      {/* Required addons — always installed */}
      {requiredAddons.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <SectionHeader>Required</SectionHeader>
            <span className="text-[10px] text-[#c4a44a]/60 font-medium">Always included</span>
          </div>
          <div className="space-y-1">
            {requiredAddons.map((addon) => (
              <AddonRow
                key={addon.esouiId}
                addon={addon}
                checked={selectedAddons.has(addon.esouiId)}
                locked
                isInstalled={installedEsouiIds.has(addon.esouiId)}
                onToggle={() => onToggleAddon(addon.esouiId, true)}
              />
            ))}
          </div>
        </div>
      )}

      {/* Optional addons — toggle to include */}
      {optionalAddons.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <SectionHeader>Optional</SectionHeader>
            {(() => {
              const allSelected = optionalAddons.every((a) => selectedAddons.has(a.esouiId));
              return (
                <button
                  onClick={() => onSelectAllOptional(!allSelected)}
                  className="text-[10px] text-sky-400/60 font-medium hover:text-sky-400 transition-colors"
                >
                  {allSelected ? "Deselect all" : "Select all"}
                </button>
              );
            })()}
          </div>
          <div className="space-y-1">
            {optionalAddons.map((addon) => (
              <AddonRow
                key={addon.esouiId}
                addon={addon}
                checked={selectedAddons.has(addon.esouiId)}
                locked={false}
                isInstalled={installedEsouiIds.has(addon.esouiId)}
                onToggle={() => onToggleAddon(addon.esouiId, false)}
              />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function AddonRow({
  addon,
  checked,
  locked,
  isInstalled,
  onToggle,
}: {
  addon: PackAddonEntry;
  checked: boolean;
  locked: boolean;
  isInstalled: boolean;
  onToggle: () => void;
}) {
  return (
    <div
      role={locked ? undefined : "button"}
      tabIndex={locked ? undefined : 0}
      onClick={() => {
        if (!locked) onToggle();
      }}
      onKeyDown={(e) => {
        if (!locked && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onToggle();
        }
      }}
      className={cn(
        "group w-full text-left rounded-lg transition-all duration-150",
        !locked && "cursor-pointer",
        // Unchecked optional: prominent interactive appearance
        !locked && !checked && "hover:bg-sky-400/[0.06] hover:ring-1 hover:ring-sky-400/20",
        // Checked: gold tint
        !locked && checked && "hover:bg-[#c4a44a]/[0.06]",
        !locked && "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-sky-400/50"
      )}
    >
      <GlassPanel
        variant="subtle"
        className={cn(
          "flex items-center gap-3 p-2.5 transition-all duration-150 rounded-lg",
          "border-l-[3px]",
          locked
            ? "border-l-[#c4a44a]/60"
            : checked
              ? "border-l-[#c4a44a]/60 bg-[#c4a44a]/[0.03]"
              : "border-l-sky-400/30"
        )}
      >
        {/* Checkbox — larger, more visible */}
        <div
          className={cn(
            "flex items-center justify-center size-5 rounded-md border-2 shrink-0 transition-all duration-150",
            locked
              ? "bg-[#c4a44a]/15 border-[#c4a44a]/40"
              : checked
                ? "bg-[#c4a44a]/20 border-[#c4a44a]/50 shadow-[0_0_6px_rgba(196,164,74,0.15)]"
                : "border-white/20 bg-white/[0.03] group-hover:border-sky-400/40 group-hover:bg-sky-400/[0.06]"
          )}
        >
          {(checked || locked) && (
            <CheckIcon
              className={cn("size-3.5", locked ? "text-[#c4a44a]/70" : "text-[#c4a44a]")}
            />
          )}
        </div>
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <span
              className={cn(
                "text-sm font-medium truncate transition-colors duration-150",
                locked
                  ? "text-foreground"
                  : checked
                    ? "text-foreground"
                    : "text-muted-foreground group-hover:text-foreground"
              )}
            >
              {addon.name}
            </span>
            {locked && (
              <span className="text-[9px] font-semibold uppercase tracking-wider text-[#c4a44a]/50 shrink-0">
                Required
              </span>
            )}
            {isInstalled && (
              <span className="text-[9px] font-semibold uppercase tracking-wider text-emerald-400/60 shrink-0">
                Installed
              </span>
            )}
            {!locked && !checked && (
              <PlusIcon className="size-3.5 text-sky-400/0 group-hover:text-sky-400/60 transition-all duration-150 shrink-0" />
            )}
          </div>
          {addon.note && (
            <p className="mt-0.5 text-xs text-muted-foreground/60 truncate">{addon.note}</p>
          )}
        </div>
        <span className="flex items-center gap-1 text-xs text-muted-foreground/40 tabular-nums shrink-0">
          #{addon.esouiId}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              openUrl(`https://www.esoui.com/downloads/info${addon.esouiId}.html`);
            }}
            aria-label={`Open ${addon.name} on ESOUI`}
            className="text-muted-foreground/30 hover:text-[#c4a44a] transition-colors"
          >
            <ExternalLinkIcon className="size-3" />
          </button>
        </span>
      </GlassPanel>
    </div>
  );
}
