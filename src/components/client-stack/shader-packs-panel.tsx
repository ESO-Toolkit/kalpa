import { useCallback, useEffect, useState } from "react";
import {
  CheckCircle2Icon,
  DownloadIcon,
  ExternalLinkIcon,
  LinkIcon,
  PackageIcon,
} from "lucide-react";
import { GlassPanel } from "@/components/ui/glass-panel";
import { Button } from "@/components/ui/button";
import { approveClientWrites } from "@/components/client-stack/approve";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type { StackPanelProps } from "@/components/client-stack/panel-props";
import type {
  PackInstallOutcome,
  PackStatus,
  ShaderLibrary,
} from "@/components/client-stack/types";

const Spinner = ({ className }: { className?: string }) => (
  <span
    aria-hidden
    className={cn(
      "inline-block size-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary",
      className
    )}
  />
);

/** Open an external page through the Tauri opener plugin — never `window.open`
 *  or a bare anchor, both of which would navigate the webview itself. There is
 *  no shared helper reachable from this panel (`client-health.tsx` keeps its
 *  own `openGuide` local and does not export it), so this copies that file's
 *  shape: dynamic import, swallow a rejection rather than let it take down the
 *  panel, and keep the URL on screen so it can still be copied by hand. */
async function openExternal(url: string): Promise<void> {
  try {
    const m = await import("@tauri-apps/plugin-opener");
    await m.openUrl(url);
  } catch {
    // Swallowed — the URL text is still rendered next to the link.
  }
}

function shortCommit(commit: string): string {
  return commit.length > 10 ? commit.slice(0, 10) : commit;
}

/**
 * One row in the shader-pack library: name, author, a one-line summary, the
 * licence, and — depending on `source.kind` and `installed` — either nothing
 * more, an "Open page" link, or a two-step Install control.
 *
 * The armed state is keyed on `armedId` from the parent rather than a local
 * boolean, because a boolean consent latch surviving a swap to a different
 * pack's row is the exact defect class this panel has shipped before.
 */
/**
 * One row in the shader-pack library.
 *
 * A **divided list row**, not a card. Five `GlassPanel`s each carried a 2px
 * border, 8px of gap and their own inset shadow — 50px of pure box before a
 * word of content, in a pane that had 267px to give. One panel with dividers
 * says the same thing and leaves the height for the packs.
 *
 * The rule that lets installed, fetchable and link-only read as one list rather
 * than three visual languages: every row is
 * `[glyph] [name · author / one qualifying line] [licence] [action]`, and only
 * the glyph and the action change. Line two is whichever sentence you most need
 * before pressing the button on the right — the summary usually, but for a
 * link-only pack the *reason* it cannot be fetched, because that is the sentence
 * that stops someone filing "the Install button is missing" as a bug.
 *
 * Gold is reserved. The left border was `primary/40` for fetchable, which is
 * what the rail uses for *selected* — so "Kalpa can fetch this" looked like
 * "you have this highlighted". Only installed gets a status colour; the glyph
 * and the button carry the rest.
 */
function PackRow({
  pack,
  armed,
  installing,
  installError,
  outcome,
  onArm,
  onCancel,
  onConfirm,
}: {
  pack: PackStatus;
  armed: boolean;
  installing: boolean;
  installError: string | null;
  outcome: PackInstallOutcome | null;
  onArm: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const linkOnly = pack.source.kind === "link_only";
  const Glyph = pack.installed ? CheckCircle2Icon : linkOnly ? ExternalLinkIcon : DownloadIcon;
  const glyphTone = pack.installed
    ? "text-status-success"
    : linkOnly
      ? "text-muted-foreground"
      : "text-primary";
  // The reason a pack cannot be fetched is the sentence that stops someone
  // filing "the Install button is missing" as a bug — but only while they do
  // not have it. iMMERSE is link-only AND present in this user's shader tree,
  // and telling them Kalpa will not download something they are already
  // running is answering a question nobody asked.
  const line2 =
    !pack.installed && pack.source.kind === "link_only" ? pack.source.reason : pack.summary;
  const expanded = armed || Boolean(outcome) || Boolean(installError);

  return (
    <li
      className={cn(
        "border-t border-structure-06 first:border-t-0",
        // Severity owns the left bar, and "installed" is the one state on this
        // list that is a status rather than a provenance. Fetchable used to
        // draw a gold bar, which is what the rail uses for *selected* — so
        // "Kalpa can fetch this" read as "you have this highlighted".
        pack.installed && "border-l-[3px] border-l-status-success"
      )}
    >
      {/* A fixed action column is what gives the list its spine. Two
          variable-width pills were competing for the right edge, and because
          the licence pill was the same height, radius and border as the button
          beside it, the licence read as something you could press. */}
      <div
        className={cn(
          "grid h-12 items-center gap-3 px-3",
          "grid-cols-[20px_minmax(0,1fr)_104px]",
          pack.installed && "pl-2.5"
        )}
      >
        {/* The same node ring the rail uses, so the two columns rhyme. */}
        <span className="grid size-5 place-items-center rounded-full bg-card ring-1 ring-structure-12">
          <Glyph aria-hidden className={cn("size-3", glyphTone)} />
        </span>

        <div className="min-w-0">
          <div className="flex items-baseline gap-1.5">
            <span
              className={cn(
                "truncate font-heading text-sm",
                pack.installed ? "font-semibold" : "font-medium"
              )}
            >
              {pack.name}
            </span>
            <span className="shrink-0 text-[11px] text-muted-foreground">by {pack.author}</span>
          </div>
          {/* The licence leads line two rather than sitting in the right
              margin: it qualifies the pack, it is not an action. */}
          <p className="truncate text-[11px] text-muted-foreground" title={line2}>
            <span className="font-medium">{pack.licence}</span> &middot; {line2}
          </p>
        </div>

        {/* Done / do / go elsewhere, encoded in shape as well as in the word:
            plain text, outline button, ghost button. */}
        <div className="justify-self-end">
          {pack.installed ? (
            <span className="inline-flex items-center gap-1 text-[12px] font-medium text-status-success">
              <CheckCircle2Icon aria-hidden className="size-3.5" />
              Installed
            </span>
          ) : linkOnly && pack.source.kind === "link_only" ? (
            <Button
              size="xs"
              variant="ghost"
              onClick={() => void openExternal((pack.source as { url: string }).url)}
            >
              Open page
              <ExternalLinkIcon />
            </Button>
          ) : (
            !armed && (
              <Button size="xs" variant="outline" onClick={onArm}>
                Install
              </Button>
            )
          )}
        </div>
      </div>

      {expanded && (
        <div
          className={cn(
            "space-y-1.5 border-t border-structure-06 bg-structure-02 px-3 py-2 text-xs",
            // The only gold bar in the panel, and it exists only while
            // Kalpa is about to put bytes in the game folder.
            armed && "border-l-[3px] border-l-primary"
          )}
        >
          {armed && pack.source.kind === "fetchable" && (
            <>
              <p className="leading-relaxed text-muted-foreground">
                Downloads from{" "}
                <span className="font-mono text-[11px]">
                  github.com/{pack.source.owner}/{pack.source.repo}
                </span>{" "}
                and writes shader files into reshade-shaders.
              </p>
              <div className="flex items-center gap-2">
                <Button size="xs" disabled={installing} onClick={onConfirm}>
                  {installing ? <Spinner className="size-3.5" /> : <DownloadIcon />}
                  {installing ? "Installing..." : "Confirm install"}
                </Button>
                <Button size="xs" variant="outline" disabled={installing} onClick={onCancel}>
                  Cancel
                </Button>
              </div>
            </>
          )}
          {installError && (
            <p className="text-status-danger" role="alert">
              {installError}
            </p>
          )}
          {outcome && (
            <p className="text-status-success">
              Installed {outcome.files.length} file{outcome.files.length === 1 ? "" : "s"} · commit{" "}
              <span className="font-mono text-[11px]">{shortCommit(outcome.commit)}</span>
            </p>
          )}
        </div>
      )}
    </li>
  );
}

/**
 * The shader-pack library for the "Shader packs" slot: what is on disk, what
 * Kalpa knows about and can fetch, and what it can only point at.
 *
 * Rows are grouped installed-first, then fetchable, then link-only, keeping
 * the backend's order within each group — that order is deliberate, not a
 * quality ranking this component should re-sort.
 */
export function ShaderPacksPanel({ clientDir, mutation }: StackPanelProps) {
  const [library, setLibrary] = useState<ShaderLibrary | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [armedId, setArmedId] = useState<string | null>(null);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [installError, setInstallError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<PackInstallOutcome | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setLoadError(null);
    try {
      const next = await invokeOrThrow<ShaderLibrary>("list_shader_packs", { clientDir });
      setLibrary(next);
    } catch (e) {
      setLibrary(null);
      setLoadError(getTauriErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }, [clientDir]);

  useEffect(() => {
    // Reset the armed pack and any stale outcome/error on mount and whenever
    // `clientDir` changes `load`, so an armed confirm can never survive onto a
    // different folder's pack of the same id. `load` itself flips the loading
    // flag before its first await, which the rule also reads as a synchronous
    // setState — the intended behaviour for an on-mount/on-clientDir-change
    // fetch, same as the pattern in `preset-panel.tsx`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setArmedId(null);
    setInstallError(null);
    setOutcome(null);
    void load();
  }, [load]);

  const handleArm = useCallback((packId: string) => {
    setInstallError(null);
    setOutcome(null);
    setArmedId(packId);
  }, []);

  const handleCancel = useCallback(() => {
    setArmedId(null);
    setInstallError(null);
  }, []);

  const handleConfirm = useCallback(
    async (packId: string) => {
      setInstallingId(packId);
      setInstallError(null);
      try {
        const result = await mutation.run("Installing shader pack", clientDir, async () => {
          await approveClientWrites(clientDir);
          return invokeOrThrow<PackInstallOutcome>("install_shader_pack", {
            clientDir,
            packId,
          });
        });
        if (result.status !== "committed") return;

        setOutcome(result.value);
        setArmedId(null);
        await load();
      } catch (e) {
        setInstallError(getTauriErrorMessage(e));
      } finally {
        setInstallingId(null);
      }
    },
    [clientDir, load, mutation]
  );

  if (loading && !library) {
    return (
      <div className="flex items-center gap-3 py-6 text-sm text-muted-foreground" role="status">
        <Spinner />
        <span>Reading the shader-pack library...</span>
      </div>
    );
  }

  if (loadError) {
    return (
      <GlassPanel variant="subtle" className="flex items-start gap-2 p-3 text-sm" role="alert">
        <PackageIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-danger" />
        <div>
          <p className="font-heading text-[13px] font-semibold text-status-danger">
            Could not read the shader-pack library
          </p>
          <p className="mt-0.5 text-xs text-muted-foreground">{loadError}</p>
        </div>
      </GlassPanel>
    );
  }

  if (!library) return null;

  const installed = library.packs.filter((p) => p.installed);
  const fetchable = library.packs.filter((p) => !p.installed && p.source.kind === "fetchable");
  const linkOnly = library.packs.filter((p) => !p.installed && p.source.kind === "link_only");
  const ordered = [...installed, ...fetchable, ...linkOnly];

  if (ordered.length === 0) {
    return (
      <GlassPanel
        variant="subtle"
        className="flex items-center gap-2 p-3 text-xs text-muted-foreground"
      >
        <LinkIcon aria-hidden className="size-4 shrink-0" />
        No shader packs known.
      </GlassPanel>
    );
  }

  // No heading of its own: `SlotPane` already titles this slot "Shader packs",
  // and a second identical header inside it is 24px of the ~274px pane spent
  // saying the same thing twice.
  return (
    <GlassPanel variant="subtle" className="overflow-hidden p-0">
      <ul>
        {ordered.map((pack) => (
          <PackRow
            key={pack.id}
            pack={pack}
            armed={armedId === pack.id}
            installing={installingId === pack.id}
            installError={installingId === pack.id || armedId === pack.id ? installError : null}
            outcome={outcome?.pack_id === pack.id ? outcome : null}
            onArm={() => handleArm(pack.id)}
            onCancel={handleCancel}
            onConfirm={() => void handleConfirm(pack.id)}
          />
        ))}
      </ul>
    </GlassPanel>
  );
}
