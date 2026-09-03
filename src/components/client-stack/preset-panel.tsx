import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowRightIcon,
  CheckCircle2Icon,
  ListOrderedIcon,
  ShieldAlertIcon,
  SlidersHorizontalIcon,
} from "lucide-react";
import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Button } from "@/components/ui/button";
import { approveClientWrites } from "@/components/client-stack/approve";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import type { StackPanelProps } from "@/components/client-stack/panel-props";
import type {
  OrderFix,
  PresetChangeOutcome,
  PresetChoice,
  PresetOptions,
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

/** How ReShade actually notices a `PresetPath` or `Techniques` edit — never
 *  instantly, so every successful write below repeats this rather than
 *  implying the change is already live in the running game. */
const RELOAD_NOTE =
  "ReShade picks up this change the next time it launches, or when the overlay reloads the current preset.";

/** The `Techniques=` value split into individual entries, each still in the
 *  preset's own `name@source.fx` spelling. */
function splitTechniques(value: string): string[] {
  return value
    .split(",")
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);
}

/** The `name` half of a `name@source.fx` technique entry, for comparing
 *  against `fix.provider_technique` / `fix.feed_technique`. */
function techniqueName(entry: string): string {
  return entry.split("@")[0]?.trim() ?? entry;
}

/**
 * Switch the active ReShade preset, and fix the technique order when the
 * DLSS 5 feed runs before whatever supplies its motion vectors.
 *
 * Both writes are gated behind a two-step confirm: either one changes what
 * the game actually renders, so picking a row (or requesting the reorder)
 * only arms the action — a second, explicit click applies it.
 */
export function PresetPanel({ clientDir, mutation }: StackPanelProps) {
  const [options, setOptions] = useState<PresetOptions | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [pendingChoice, setPendingChoice] = useState<PresetChoice | null>(null);
  const [switching, setSwitching] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [switchOutcome, setSwitchOutcome] = useState<PresetChangeOutcome | null>(null);

  const [fixConfirming, setFixConfirming] = useState(false);
  const [fixing, setFixing] = useState(false);
  const [fixError, setFixError] = useState<string | null>(null);
  const [fixOutcome, setFixOutcome] = useState<PresetChangeOutcome | null>(null);
  const requestToken = useRef(0);
  const mutationToken = useRef(0);

  const load = useCallback(async () => {
    const token = ++requestToken.current;
    setLoading(true);
    setLoadError(null);
    try {
      const next = await invokeOrThrow<PresetOptions>("list_client_presets", { clientDir });
      if (requestToken.current !== token) return;
      setOptions(next);
    } catch (e) {
      if (requestToken.current !== token) return;
      setOptions(null);
      setLoadError(getTauriErrorMessage(e));
    } finally {
      if (requestToken.current === token) setLoading(false);
    }
  }, [clientDir]);

  useEffect(() => {
    // Reset every per-preset confirm/outcome latch on mount and whenever
    // `clientDir` changes `load`, so a stale confirm cannot survive onto a
    // different folder. `load` itself flips the loading flag before its
    // first await, which the rule also reads as a synchronous setState —
    // the intended behaviour for an on-mount/on-clientDir-change fetch, same
    // as the pattern in `client-health.tsx`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setPendingChoice(null);
    setSwitchError(null);
    setSwitchOutcome(null);
    setFixConfirming(false);
    setFixError(null);
    setFixOutcome(null);
    void load();
    return () => {
      requestToken.current += 1;
      mutationToken.current += 1;
    };
  }, [load]);

  const handleRequestSwitch = useCallback((choice: PresetChoice) => {
    if (choice.is_active) return;
    setSwitchError(null);
    setSwitchOutcome(null);
    setPendingChoice(choice);
  }, []);

  const handleCancelSwitch = useCallback(() => {
    setPendingChoice(null);
    setSwitchError(null);
  }, []);

  const handleConfirmSwitch = useCallback(async () => {
    if (!pendingChoice) return;
    const token = ++mutationToken.current;
    setSwitching(true);
    setSwitchError(null);
    try {
      const result = await mutation.run("Switching the active preset", clientDir, async () => {
        await approveClientWrites(clientDir);
        return invokeOrThrow<PresetChangeOutcome>("set_client_preset", {
          clientDir,
          relativePath: pendingChoice.relative_path,
        });
      });
      if (mutationToken.current !== token || result.status !== "committed") return;
      setSwitchOutcome(result.value);
      setPendingChoice(null);
      await load();
    } catch (e) {
      if (mutationToken.current !== token) return;
      setSwitchError(getTauriErrorMessage(e));
    } finally {
      if (mutationToken.current === token) setSwitching(false);
    }
  }, [clientDir, load, mutation, pendingChoice]);

  const handleRequestFix = useCallback(() => {
    setFixError(null);
    setFixOutcome(null);
    setFixConfirming(true);
  }, []);

  const handleCancelFix = useCallback(() => {
    setFixConfirming(false);
    setFixError(null);
  }, []);

  const handleConfirmFix = useCallback(async () => {
    const token = ++mutationToken.current;
    setFixing(true);
    setFixError(null);
    try {
      const result = await mutation.run("Fixing preset technique order", clientDir, async () => {
        await approveClientWrites(clientDir);
        return invokeOrThrow<PresetChangeOutcome>("fix_client_technique_order", {
          clientDir,
        });
      });
      if (mutationToken.current !== token || result.status !== "committed") return;
      setFixOutcome(result.value);
      setFixConfirming(false);
      await load();
    } catch (e) {
      if (mutationToken.current !== token) return;
      setFixError(getTauriErrorMessage(e));
    } finally {
      if (mutationToken.current === token) setFixing(false);
    }
  }, [clientDir, load, mutation]);

  if (loading && !options) {
    return (
      <div className="flex items-center gap-3 py-6 text-sm text-muted-foreground" role="status">
        <Spinner />
        <span>Reading the presets in this folder...</span>
      </div>
    );
  }

  if (loadError) {
    return (
      <GlassPanel variant="subtle" className="flex items-start gap-2 p-3 text-sm" role="alert">
        <ShieldAlertIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-danger" />
        <div>
          <p className="font-heading text-[13px] font-semibold text-status-danger">
            Could not read presets
          </p>
          <p className="mt-0.5 text-xs text-muted-foreground">{loadError}</p>
        </div>
      </GlassPanel>
    );
  }

  if (!options) return null;

  return (
    <div className="space-y-3">
      <SwitchPresetCard
        options={options}
        pendingChoice={pendingChoice}
        switching={switching}
        mutationPending={mutation.pending}
        switchError={switchError}
        switchOutcome={switchOutcome}
        onRequest={handleRequestSwitch}
        onCancel={handleCancelSwitch}
        onConfirm={() => void handleConfirmSwitch()}
      />
      {options.fix && (
        <FixOrderCard
          fix={options.fix}
          confirming={fixConfirming}
          fixing={fixing}
          mutationPending={mutation.pending}
          fixError={fixError}
          fixOutcome={fixOutcome}
          onRequest={handleRequestFix}
          onCancel={handleCancelFix}
          onConfirm={() => void handleConfirmFix()}
        />
      )}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Switch preset                                                              */
/* -------------------------------------------------------------------------- */

function SwitchPresetCard({
  options,
  pendingChoice,
  switching,
  mutationPending,
  switchError,
  switchOutcome,
  onRequest,
  onCancel,
  onConfirm,
}: {
  options: PresetOptions;
  pendingChoice: PresetChoice | null;
  switching: boolean;
  mutationPending: boolean;
  switchError: string | null;
  switchOutcome: PresetChangeOutcome | null;
  onRequest: (choice: PresetChoice) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { choices } = options;

  // One preset is a fact, not an empty state. Rendering a "Switch preset"
  // header over "nothing to switch to" was the panel's clearest tell that it
  // was a stub — it announced a chooser and then withdrew it, which reads as a
  // feature that has not landed rather than as a folder with one file in it.
  // The slot above already says which preset is active and where it lives, so
  // there is nothing left for this card to add until a second one exists.
  if (choices.length <= 1) return null;

  return (
    <GlassPanel variant="subtle" className="space-y-3 p-4">
      <SectionHeader>Switch preset</SectionHeader>

      {
        <ul className="space-y-2">
          {choices.map((choice) => {
            const isPending = pendingChoice?.relative_path === choice.relative_path;
            return (
              <li key={choice.relative_path}>
                <button
                  type="button"
                  disabled={choice.is_active || switching || mutationPending}
                  onClick={() => onRequest(choice)}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-colors duration-150",
                    "focus-visible:outline-none focus-visible:ring-3 focus-visible:ring-accent-sky/20",
                    "disabled:cursor-default disabled:opacity-100",
                    isPending
                      ? "border-primary/30 border-l-[3px] border-l-primary bg-primary/[0.04]"
                      : choice.is_active
                        ? "border-status-success/20 border-l-[3px] border-l-status-success bg-status-success/[0.04]"
                        : "border-structure-06 bg-structure-02 hover:border-structure-10"
                  )}
                >
                  {choice.is_active ? (
                    <CheckCircle2Icon aria-hidden className="size-4 shrink-0 text-status-success" />
                  ) : (
                    <SlidersHorizontalIcon
                      aria-hidden
                      className="size-4 shrink-0 text-muted-foreground"
                    />
                  )}
                  <div className="min-w-0 flex-1">
                    <p className="truncate font-mono text-[13px] font-semibold">
                      {choice.relative_path}
                    </p>
                    <p className="text-xs text-muted-foreground">
                      {choice.technique_count} technique{choice.technique_count === 1 ? "" : "s"}{" "}
                      enabled
                    </p>
                  </div>
                  {choice.is_active && <InfoPill color="emerald">Active</InfoPill>}
                </button>
              </li>
            );
          })}
        </ul>
      }

      {pendingChoice && (
        <div className="space-y-2 border-t border-structure-06 pt-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            Switching to <span className="font-mono">{pendingChoice.relative_path}</span> changes
            what ReShade renders in this game. {RELOAD_NOTE}
          </p>
          <div className="flex items-center gap-2">
            <Button size="sm" disabled={switching || mutationPending} onClick={onConfirm}>
              {switching ? <Spinner className="size-3.5" /> : <SlidersHorizontalIcon />}
              {switching ? "Switching..." : "Confirm switch"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={switching || mutationPending}
              onClick={onCancel}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}

      {switchError && (
        <p className="text-xs text-status-danger" role="alert">
          {switchError}
        </p>
      )}

      {switchOutcome && (
        <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs" role="status">
          <p className="text-status-success">{switchOutcome.summary}</p>
          <p className="text-muted-foreground">{RELOAD_NOTE}</p>
        </GlassPanel>
      )}
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* Fix technique order                                                       */
/* -------------------------------------------------------------------------- */

function TechniqueList({
  entries,
  provider,
  feed,
}: {
  entries: string[];
  provider: string;
  feed: string;
}) {
  return (
    <ol className="list-decimal space-y-1 pl-4">
      {entries.map((entry, i) => {
        const name = techniqueName(entry);
        const isProvider = name === provider;
        const isFeed = name === feed;
        return (
          <li key={`${entry}-${i}`} className="flex flex-wrap items-center gap-2">
            <span className="font-mono text-[12px]">{entry}</span>
            {isProvider && <InfoPill color="sky">motion vector provider</InfoPill>}
            {isFeed && <InfoPill color="amber">DLSS 5 feed</InfoPill>}
          </li>
        );
      })}
    </ol>
  );
}

function FixOrderCard({
  fix,
  confirming,
  fixing,
  mutationPending,
  fixError,
  fixOutcome,
  onRequest,
  onCancel,
  onConfirm,
}: {
  fix: OrderFix;
  confirming: boolean;
  fixing: boolean;
  mutationPending: boolean;
  fixError: string | null;
  fixOutcome: PresetChangeOutcome | null;
  onRequest: () => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const before = splitTechniques(fix.before);
  const after = splitTechniques(fix.after);

  return (
    <GlassPanel
      variant="subtle"
      className="space-y-3 border-status-warning/20 border-l-[3px] border-l-status-warning p-4"
    >
      <div className="flex items-start gap-2">
        <ListOrderedIcon aria-hidden className="mt-0.5 size-4 shrink-0 text-status-warning" />
        <div>
          <SectionHeader>Technique order</SectionHeader>
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
            <span className="font-mono">{fix.feed_technique}</span> runs before{" "}
            <span className="font-mono">{fix.provider_technique}</span>, the technique that supplies
            its motion vectors. ReShade runs techniques in the order the preset lists them, so the
            feed is reading last frame&apos;s motion vectors instead of this frame&apos;s. Nothing
            errors — the image is just quietly wrong.
          </p>
        </div>
      </div>

      {!fixOutcome && (
        <div className="grid gap-3 sm:grid-cols-2">
          <div>
            <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              Now
            </p>
            <TechniqueList
              entries={before}
              provider={fix.provider_technique}
              feed={fix.feed_technique}
            />
          </div>
          <div>
            <p className="mb-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
              After the fix
            </p>
            <TechniqueList
              entries={after}
              provider={fix.provider_technique}
              feed={fix.feed_technique}
            />
          </div>
        </div>
      )}

      {!confirming && !fixOutcome && (
        <Button
          size="sm"
          variant="outline"
          disabled={fixing || mutationPending}
          onClick={onRequest}
        >
          <ArrowRightIcon />
          Fix technique order
        </Button>
      )}

      {confirming && (
        <div className="space-y-2 border-t border-structure-06 pt-3">
          <p className="text-xs leading-relaxed text-muted-foreground">
            {fix.summary} This changes how this preset renders. {RELOAD_NOTE}
          </p>
          <div className="flex items-center gap-2">
            <Button size="sm" disabled={fixing || mutationPending} onClick={onConfirm}>
              {fixing ? <Spinner className="size-3.5" /> : <ListOrderedIcon />}
              {fixing ? "Fixing..." : "Confirm fix"}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={fixing || mutationPending}
              onClick={onCancel}
            >
              Cancel
            </Button>
          </div>
        </div>
      )}

      {fixError && (
        <p className="text-xs text-status-danger" role="alert">
          {fixError}
        </p>
      )}

      {fixOutcome && (
        <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs" role="status">
          <p className="text-status-success">{fixOutcome.summary}</p>
          <p className="text-muted-foreground">{RELOAD_NOTE}</p>
        </GlassPanel>
      )}
    </GlassPanel>
  );
}
