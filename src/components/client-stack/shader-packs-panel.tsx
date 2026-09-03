import { useCallback, useEffect, useState } from "react";
import {
  CheckCircle2Icon,
  DownloadIcon,
  ExternalLinkIcon,
  LinkIcon,
  PackageIcon,
} from "lucide-react";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
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
  const isFetchable = pack.source.kind === "fetchable";

  return (
    <li>
      <GlassPanel
        variant="subtle"
        className={cn(
          "space-y-1.5 border-l-[3px] p-2.5",
          pack.installed
            ? "border-l-status-success"
            : isFetchable
              ? "border-l-primary/40"
              : "border-l-structure-10"
        )}
      >
        <div className="flex flex-wrap items-start justify-between gap-x-3 gap-y-1">
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="truncate font-heading text-[13px] font-semibold">{pack.name}</span>
              <span className="shrink-0 text-[11px] text-muted-foreground">by {pack.author}</span>
            </div>
            <p className="truncate text-xs text-muted-foreground" title={pack.summary}>
              {pack.summary}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            {pack.installed && (
              <InfoPill color="emerald">
                <CheckCircle2Icon aria-hidden className="size-3" />
                Installed
              </InfoPill>
            )}
            <InfoPill color="muted">{pack.licence}</InfoPill>
          </div>
        </div>

        {pack.installed && pack.found.length > 0 && (
          <p
            className="truncate font-mono text-[11px] text-muted-foreground"
            title={pack.found.join(", ")}
          >
            Found: {pack.found.join(", ")}
          </p>
        )}

        {!pack.installed &&
          pack.source.kind === "link_only" &&
          (() => {
            const source = pack.source;
            return (
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs text-muted-foreground" title={source.reason}>
                    {source.reason}
                  </p>
                  {/* The URL on screen, not only behind the button. `openExternal`
                      swallows an opener-scope rejection so a failure cannot take
                      down the panel — which means that without this line a failed
                      open is completely silent and leaves the user nothing to go
                      on. It doubles as the provenance column: where a pack comes
                      from is the thing that makes this a directory. */}
                  <p className="truncate font-mono text-[11px] text-muted-foreground">
                    {source.url.replace(/^https:\/\//, "")}
                  </p>
                </div>
                <Button size="sm" variant="outline" onClick={() => void openExternal(source.url)}>
                  <ExternalLinkIcon />
                  Open page
                </Button>
              </div>
            );
          })()}

        {!pack.installed && pack.source.kind === "fetchable" && !outcome && (
          <>
            {!armed ? (
              <div className="flex justify-end">
                <Button size="sm" variant="outline" onClick={onArm}>
                  <DownloadIcon />
                  Install
                </Button>
              </div>
            ) : (
              <div className="space-y-1.5 border-t border-structure-06 pt-1.5">
                <p className="text-xs leading-relaxed text-muted-foreground">
                  Downloads from{" "}
                  <span className="font-mono text-[11px]">
                    github.com/{pack.source.owner}/{pack.source.repo}
                  </span>{" "}
                  and writes shader files into reshade-shaders.
                </p>
                <div className="flex items-center gap-2">
                  <Button size="sm" disabled={installing} onClick={onConfirm}>
                    {installing ? <Spinner className="size-3.5" /> : <DownloadIcon />}
                    {installing ? "Installing..." : "Confirm install"}
                  </Button>
                  <Button size="sm" variant="outline" disabled={installing} onClick={onCancel}>
                    Cancel
                  </Button>
                </div>
              </div>
            )}
          </>
        )}

        {installError && (
          <p className="text-xs text-status-danger" role="alert">
            {installError}
          </p>
        )}

        {outcome && (
          <div className="space-y-0.5 border-t border-structure-06 pt-1.5 text-xs">
            <p className="text-status-success">
              Installed {outcome.pack_name} — {outcome.files.length} file
              {outcome.files.length === 1 ? "" : "s"}.
            </p>
            <p className="text-muted-foreground">
              Commit <span className="font-mono text-[11px]">{shortCommit(outcome.commit)}</span>
            </p>
          </div>
        )}
      </GlassPanel>
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
export function ShaderPacksPanel({ clientDir, onChanged }: StackPanelProps) {
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
        await approveClientWrites(clientDir);
        const result = await invokeOrThrow<PackInstallOutcome>("install_shader_pack", {
          clientDir,
          packId,
        });
        setOutcome(result);
        setArmedId(null);
        await onChanged();
        await load();
      } catch (e) {
        setInstallError(getTauriErrorMessage(e));
      } finally {
        setInstallingId(null);
      }
    },
    [clientDir, load, onChanged]
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
    <div className="space-y-2">
      <ul className="space-y-2">
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
    </div>
  );
}
