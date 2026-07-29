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
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { PackDetailSkeleton } from "@/components/ui/skeletons";
import { Slide } from "@/components/animate-ui/primitives/effects/slide";
import { CountingNumber } from "@/components/animate-ui/primitives/texts/counting-number";

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
  exportIncludeSettings,
  onToggleExportSettings,
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
  exportIncludeSettings: boolean;
  onToggleExportSettings: () => void;
}) {
  const [shareMode, setShareMode] = useState<ShareMode>("private-link");
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);

  if (loading || !pack) {
    return <PackDetailSkeleton />;
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
          <span className="text-xs text-muted-foreground">by {decodeHtml(pack.authorName)}</span>
        )}
        {/* Relative dates */}
        {(() => {
          const dateIso =
            pack.updatedAt && pack.updatedAt !== pack.createdAt ? pack.updatedAt : pack.createdAt;
          const label = pack.updatedAt && pack.updatedAt !== pack.createdAt ? "Updated" : "Created";
          const relative = dateIso ? formatRelativeDate(dateIso) : "";
          return relative ? (
            <SimpleTooltip content={dateIso}>
              <span className="text-xs text-muted-foreground">
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
                <Slide
                  direction="right"
                  offset={8}
                  transition={{ type: "spring", stiffness: 400, damping: 30 }}
                >
                  <div className="flex items-center gap-1.5 rounded-lg border border-status-danger-strong/25 bg-status-danger-strong/[0.08] px-2.5 py-1 shadow-[0_0_12px_color-mix(in_oklab,var(--status-danger-strong)_6%,transparent),inset_0_1px_0_color-mix(in_oklab,var(--status-danger-strong)_4%,transparent)]">
                    <span className="text-[11px] text-status-danger font-medium">
                      Delete this pack?
                    </span>
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
                      className="h-6 px-2 text-[10px] border-status-danger-strong/30 text-status-danger hover:bg-status-danger-strong/10"
                    >
                      {deletingPack ? <Loader2Icon className="size-3 animate-spin" /> : "Delete"}
                    </Button>
                  </div>
                </Slide>
              ) : (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setShowDeleteConfirm(true)}
                  className="text-status-danger hover:text-status-danger-soft hover:border-status-danger-strong/30"
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
            className={cn(showShareSection && "border-primary/30 bg-primary/[0.06]")}
          >
            <ShareIcon className="size-3.5 mr-1.5" />
            Share
          </Button>
        </div>
        <SimpleTooltip
          content={
            authUser ? (pack.userVoted ? "Remove vote" : "Upvote this pack") : "Sign in to vote"
          }
        >
          <button
            onClick={() => onVote(pack.id)}
            disabled={votingPacks.has(pack.id)}
            className={cn(
              "group/vote relative flex items-center gap-1.5 text-sm font-medium rounded-full px-3 py-1.5 transition-all duration-200 border",
              votingPacks.has(pack.id) && "opacity-60 pointer-events-none",
              pack.userVoted
                ? "text-primary bg-primary/[0.15] border-primary/40 hover:bg-primary/[0.22] shadow-[0_0_14px_color-mix(in_oklab,var(--primary)_25%,transparent),inset_0_1px_0_color-mix(in_oklab,var(--primary)_12%,transparent)]"
                : "text-muted-foreground bg-structure-03 border-structure-08 hover:text-primary hover:border-primary/25 hover:bg-primary/[0.08] hover:shadow-[0_0_8px_color-mix(in_oklab,var(--primary)_8%,transparent)]"
            )}
          >
            <ArrowUpIcon
              className={cn(
                "size-4 transition-all duration-200",
                pack.userVoted ? "-translate-y-[1px]" : "group-hover/vote:-translate-y-[1px]"
              )}
              strokeWidth={pack.userVoted ? 2.5 : 2}
            />
            <CountingNumber
              number={pack.voteCount > 0 ? pack.voteCount : 0}
              transition={{ stiffness: 200, damping: 25 }}
              initiallyStable
            />
          </button>
        </SimpleTooltip>
      </div>

      {/* Share section — Task C: two-mode toggle */}
      {showShareSection && (
        <Slide
          direction="down"
          offset={12}
          transition={{ type: "spring", stiffness: 300, damping: 25 }}
        >
          <div className="rounded-xl border border-structure-08 bg-[color-mix(in_oklab,var(--card)_50%,transparent)] backdrop-blur-md p-3 space-y-3 shadow-[inset_0_1px_0_var(--structure-04),0_4px_16px_var(--scrim-15)]">
            {/* Segmented control: Private Link vs Export File */}
            <div className="relative flex p-0.5 rounded-lg bg-structure-03 border border-structure-06">
              <div
                className="absolute top-0.5 bottom-0.5 rounded-md bg-structure-08 shadow-sm transition-all duration-300 ease-[cubic-bezier(0.34,1.56,0.64,1)]"
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
                      : "text-muted-foreground hover:text-foreground"
                  )}
                >
                  {mode === "private-link" ? "Private Link" : "Export File"}
                </button>
              ))}
            </div>

            {shareMode === "private-link" ? (
              <div className="space-y-2">
                <p className="text-xs text-muted-foreground">
                  Share privately — only people with this code can import this pack.
                </p>
                {shareResult ? (
                  <div className="space-y-2">
                    <div className="flex items-center gap-2">
                      <code className="flex-1 rounded-lg bg-primary/[0.06] border border-primary/[0.15] px-3 py-2 text-center font-mono text-lg font-bold tracking-[0.3em] text-primary shadow-[0_0_16px_color-mix(in_oklab,var(--primary)_8%,transparent),inset_0_1px_0_color-mix(in_oklab,var(--primary)_6%,transparent)]">
                        {shareResult.code}
                      </code>
                      <SimpleTooltip content="Copy share code">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => onCopyToClipboard(shareResult.code, "code")}
                        >
                          {copiedField === "code" ? (
                            <CheckIcon className="size-3.5 text-status-success" />
                          ) : (
                            <CopyIcon className="size-3.5" />
                          )}
                        </Button>
                      </SimpleTooltip>
                    </div>
                    <div className="flex items-center gap-2">
                      <code className="flex-1 truncate rounded-md bg-structure-05 px-3 py-1.5 text-xs text-muted-foreground">
                        {shareResult.deepLink}
                      </code>
                      <SimpleTooltip content="Copy deep link">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => onCopyToClipboard(shareResult.deepLink, "link")}
                        >
                          {copiedField === "link" ? (
                            <CheckIcon className="size-3.5 text-status-success" />
                          ) : (
                            <CopyIcon className="size-3.5" />
                          )}
                        </Button>
                      </SimpleTooltip>
                    </div>
                    <div className="flex items-center justify-between">
                      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                        <ClockIcon className="size-3" />
                        {shareResult.expiresAt
                          ? formatRelativeExpiry(shareResult.expiresAt)
                          : "Expires in ~7 days"}
                      </p>
                      <button
                        onClick={onRegenerateShareCode}
                        className="text-xs text-muted-foreground hover:text-muted-foreground transition-colors"
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
                <p className="text-xs text-muted-foreground">
                  Save as a .esopack file to share on Discord, forums, or privately.
                </p>
                <label className="flex items-center gap-2 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={exportIncludeSettings}
                    onChange={onToggleExportSettings}
                    className="size-3.5 accent-primary cursor-pointer"
                  />
                  <span className="text-xs text-muted-foreground">Include addon settings (v2)</span>
                </label>
                <Button variant="outline" size="sm" onClick={onExportFile} className="w-full">
                  <FileDownIcon className="size-3.5 mr-1.5" />
                  Export .esopack File
                </Button>
              </div>
            )}
          </div>
        </Slide>
      )}

      {/* Install progress bar */}
      {(installing && installProgress) || installSucceeded ? (
        <Fade transition={{ type: "spring", stiffness: 200, damping: 25 }}>
          <div
            className={cn(
              "rounded-xl border p-3",
              installSucceeded
                ? "border-status-success/25 bg-status-success/[0.06] shadow-[0_0_16px_color-mix(in_oklab,var(--status-success-strong)_8%,transparent),inset_0_1px_0_color-mix(in_oklab,var(--status-success-strong)_6%,transparent)]"
                : "border-primary/25 bg-primary/[0.06] shadow-[0_0_16px_color-mix(in_oklab,var(--primary)_8%,transparent),inset_0_1px_0_color-mix(in_oklab,var(--primary)_6%,transparent)]"
            )}
          >
            {installProgress && (
              <div className="flex items-center justify-between text-sm mb-2">
                <span className="text-primary font-medium">
                  Installing{" "}
                  <CountingNumber
                    number={installProgress.completed + installProgress.failed}
                    transition={{ stiffness: 200, damping: 25 }}
                  />
                  /
                  <CountingNumber
                    number={installProgress.total}
                    initiallyStable
                    transition={{ stiffness: 200, damping: 25 }}
                  />
                </span>
                {installProgress.failed > 0 && (
                  <span className="text-status-danger text-xs">
                    {installProgress.failed} failed
                  </span>
                )}
              </div>
            )}
            {installSucceeded && !installProgress && (
              <div className="flex items-center gap-2 text-sm mb-2">
                <CheckIcon className="size-4 text-status-success" />
                <span className="text-status-success font-medium">Installed successfully</span>
              </div>
            )}
            <div className="h-1.5 rounded-full bg-structure-06">
              <div
                className={cn(
                  "h-full rounded-full transition-all duration-300 ease-out",
                  installSucceeded
                    ? "bg-status-success shadow-[0_0_8px_color-mix(in_oklab,var(--status-success-strong)_50%,transparent)]"
                    : "bg-primary shadow-[0_0_8px_color-mix(in_oklab,var(--primary)_50%,transparent)]"
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
        </Fade>
      ) : null}

      {/* Required addons — always installed */}
      {requiredAddons.length > 0 && (
        <div>
          <div className="flex items-center justify-between mb-2">
            <SectionHeader>Required</SectionHeader>
            <span className="text-xs text-primary font-medium">Always included</span>
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
                  className="text-xs text-accent-sky font-medium hover:text-accent-sky transition-colors"
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
        !locked && !checked && "hover:bg-accent-sky/[0.06] hover:ring-1 hover:ring-accent-sky/20",
        // Checked: gold tint
        !locked && checked && "hover:bg-primary/[0.06]",
        !locked &&
          "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-accent-sky/50"
      )}
    >
      <GlassPanel
        variant="subtle"
        className={cn(
          "flex items-center gap-3 p-2.5 transition-all duration-150 rounded-lg",
          "border-l-[3px]",
          locked
            ? "border-l-primary/60"
            : checked
              ? "border-l-primary/60 bg-primary/[0.03]"
              : "border-l-accent-sky/30"
        )}
      >
        {/* Checkbox — larger, more visible */}
        <div
          className={cn(
            "flex items-center justify-center size-5 rounded-md border-2 shrink-0 transition-all duration-150",
            locked
              ? "bg-primary/15 border-primary/40"
              : checked
                ? "bg-primary/20 border-primary/50 shadow-[0_0_6px_color-mix(in_oklab,var(--primary)_15%,transparent)]"
                : "border-structure-20 bg-structure-03 group-hover:border-accent-sky/40 group-hover:bg-accent-sky/[0.06]"
          )}
        >
          {(checked || locked) && (
            <CheckIcon className={cn("size-3.5", locked ? "text-primary/70" : "text-primary")} />
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
              <span className="text-[11px] font-semibold uppercase tracking-wider text-primary shrink-0">
                Required
              </span>
            )}
            {isInstalled && (
              <span className="text-[11px] font-semibold uppercase tracking-wider text-status-success shrink-0">
                Installed
              </span>
            )}
            {!locked && !checked && (
              <PlusIcon className="size-3.5 text-accent-sky/0 group-hover:text-accent-sky/60 transition-all duration-150 shrink-0" />
            )}
          </div>
          {addon.note && (
            <p className="mt-0.5 text-xs text-muted-foreground truncate">{addon.note}</p>
          )}
        </div>
        <span className="flex items-center gap-1 text-xs text-muted-foreground tabular-nums shrink-0">
          #{addon.esouiId}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              openUrl(`https://www.esoui.com/downloads/info${addon.esouiId}.html`);
            }}
            aria-label={`Open ${addon.name} on ESOUI`}
            className="text-muted-foreground/30 hover:text-primary transition-colors"
          >
            <ExternalLinkIcon className="size-3" />
          </button>
        </span>
      </GlassPanel>
    </div>
  );
}
