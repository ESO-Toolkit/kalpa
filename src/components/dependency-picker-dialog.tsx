import { useCallback, useId, useMemo, useState } from "react";
import { AlertTriangleIcon, DownloadIcon, PackageIcon } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import type { PendingDependency } from "@/types";

/** Keep in lockstep with `MAX_SELECTED_DEPENDENCIES` in `src-tauri/src/commands.rs`,
 * which rejects a larger selection outright rather than installing a prefix. */
const MAX_SELECTED_DEPENDENCIES = 50;

interface DependencyPickerDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The dependencies the backend held back under the "ask" policy. */
  pending: PendingDependency[];
  /**
   * The user's decision. `selectedNames` are the dependencies to install (a
   * subset of `pending`, possibly empty); `alwaysAutoInstall` is the footer
   * opt-in that should flip the stored policy to "auto"; `rememberDeclines` is
   * the separate opt-in for persisting the unticked ones to the skip list.
   * Declines are only ever remembered when that is true — a plain untick means
   * "not this time", not "never again". The dialog closes itself before this
   * fires.
   */
  onConfirm: (
    selectedNames: string[],
    alwaysAutoInstall: boolean,
    rememberDeclines: boolean
  ) => void;
}

/**
 * Prompts for the dependencies an install/update wanted to pull in, when the
 * `dependencyPolicy` preference is "ask".
 *
 * Required dependencies (manifest `DependsOn`) arrive ticked, optional ones
 * (`OptionalDependsOn` — which Kalpa has never installed automatically) arrive
 * unticked. Declining a required dependency warns but is never blocked: the
 * user's AddOns folder is theirs, and Kalpa's other destructive-ish flows
 * (removal warnings) already warn-don't-block.
 */
export function DependencyPickerDialog({
  open,
  onOpenChange,
  pending,
  onConfirm,
}: DependencyPickerDialogProps) {
  // Identity of THIS prompt. Remounting the body on a new prompt re-seeds the
  // pre-ticked selection through useState's initializer, which is why nothing
  // here has to sync state from an effect (ESLint forbids setState in effects).
  // A repeat prompt for the same dependency set keeps the user's last ticks,
  // which is the behaviour you want when an install is retried.
  //
  // `required` and `minVersion` are part of that identity, not decoration. On a
  // name-only key, a queued prompt where LibX is OPTIONAL would remount against
  // the previous prompt's state where LibX was REQUIRED, and inherit its tick —
  // surfacing an optional dependency pre-selected, which is exactly what the
  // unticked-by-default rule exists to prevent.
  //
  // Built with JSON rather than joined separators: the manifest parser splits
  // dependency tokens on whitespace only, so a name may legitimately contain
  // ':' or '|' and could otherwise be crafted to collide with a different
  // prompt's key — which would remount against unrelated selection state.
  const promptKey = JSON.stringify(
    pending.map((dep) => [dep.name, dep.required, dep.minVersion ?? null])
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DependencyPickerBody key={promptKey} pending={pending} onConfirm={onConfirm} />
      </DialogContent>
    </Dialog>
  );
}

// No `onOpenChange` here: the body never closes the dialog, so that closing is
// unambiguously a dismissal. Only the Dialog root wires it.
function DependencyPickerBody({
  pending,
  onConfirm,
}: Omit<DependencyPickerDialogProps, "open" | "onOpenChange">) {
  const required = useMemo(() => pending.filter((dep) => dep.required), [pending]);
  const optional = useMemo(() => pending.filter((dep) => !dep.required), [pending]);

  // Required pre-ticked, optional unticked. Seeded once — see promptKey above.
  const [selected, setSelected] = useState<ReadonlySet<string>>(
    () => new Set(pending.filter((dep) => dep.required).map((dep) => dep.name))
  );
  const [alwaysAutoInstall, setAlwaysAutoInstall] = useState(false);
  // Opt-in, and deliberately off by default. Turning a library down once must
  // not silently mean "never offer this again" — optional entries in particular
  // arrive unticked, so persisting every untick would bury them permanently
  // without the user ever actively declining anything.
  const [rememberDeclines, setRememberDeclines] = useState(false);

  const toggle = useCallback((name: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }, []);

  const selectedCount = selected.size;
  const allSelected = pending.length > 0 && selectedCount === pending.length;
  const someSelected = selectedCount > 0 && !allSelected;
  // Mirrors MAX_SELECTED_DEPENDENCIES in commands.rs. The backend rejects an
  // oversized list outright, which would install nothing at all, so say so here
  // rather than letting the user hit a failure after choosing.
  const overLimit = selectedCount > MAX_SELECTED_DEPENDENCIES;
  const declinedRequiredCount = required.filter((dep) => !selected.has(dep.name)).length;

  const toggleAll = useCallback(() => {
    setSelected((prev) =>
      prev.size === pending.length ? new Set<string>() : new Set(pending.map((dep) => dep.name))
    );
  }, [pending]);

  // Neither decision path closes the dialog itself. The owner does that when it
  // handles the decision, which leaves `onOpenChange(false)` meaning exactly one
  // thing — the user dismissed without deciding (X, Escape, backdrop). Closing
  // here as well made those indistinguishable, and the dismissal path then
  // skipped required libraries with no warning at all.
  const confirm = () => {
    onConfirm([...selected], alwaysAutoInstall, rememberDeclines);
  };

  const skipAll = () => {
    // Never carry the "always install" opt-in through this path: skipping every
    // dependency and asking to always install them are contradictory intents,
    // and silently flipping the policy to "auto" here would be a nasty surprise.
    // `rememberDeclines` does carry, so "skip all" plus the checkbox is how a
    // user says "stop offering me these" in one go.
    onConfirm([], false, rememberDeclines);
  };

  return (
    <>
      <DialogHeader>
        <DialogTitle className="flex items-center gap-2">
          <PackageIcon className="size-5 text-primary" />
          Install dependencies?
        </DialogTitle>
        <DialogDescription>
          {pending.length === 1
            ? "This addon depends on another addon that isn't installed yet."
            : "These addons depend on other addons that aren't installed yet."}{" "}
          Anything you tick is installed along with the libraries it needs in turn.
        </DialogDescription>
      </DialogHeader>

      <div className="flex max-h-[55vh] flex-col gap-3 overflow-y-auto pr-1">
        {pending.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">Nothing to install.</p>
        ) : (
          <>
            <label className="flex cursor-pointer select-none items-center gap-2 self-start">
              <Checkbox
                checked={allSelected}
                indeterminate={someSelected}
                onCheckedChange={toggleAll}
                aria-label={allSelected ? "Select no dependencies" : "Select all dependencies"}
              />
              <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                {selectedCount > 0
                  ? `${selectedCount} of ${pending.length} selected`
                  : "Select all"}
              </span>
            </label>

            {required.length > 0 && (
              <section className="flex flex-col gap-2">
                <SectionHeader>Required ({required.length})</SectionHeader>
                {required.map((dep) => (
                  <DependencyRow
                    key={dep.name}
                    dep={dep}
                    checked={selected.has(dep.name)}
                    onToggle={toggle}
                  />
                ))}
              </section>
            )}

            {optional.length > 0 && (
              <section className="flex flex-col gap-2">
                <SectionHeader>Optional ({optional.length})</SectionHeader>
                {optional.map((dep) => (
                  <DependencyRow
                    key={dep.name}
                    dep={dep}
                    checked={selected.has(dep.name)}
                    onToggle={toggle}
                  />
                ))}
              </section>
            )}
          </>
        )}
        {overLimit && (
          <Alert className="border-status-warning/20 bg-status-warning/[0.06] text-status-warning-soft shadow-none">
            <AlertTriangleIcon />
            <AlertDescription className="text-status-warning-soft/90">
              {selectedCount} selected, but only {MAX_SELECTED_DEPENDENCIES} can be installed at
              once. Untick a few and run the rest afterwards.
            </AlertDescription>
          </Alert>
        )}

        {/* Required rows arrive pre-ticked and their per-row warning only shows
            after a manual untick — so "Skip all", Escape or clicking away would
            otherwise decline every required library with nothing said. State the
            consequence up front whenever any required entry is unticked, which
            includes the skip-everything case. */}
        {declinedRequiredCount > 0 && (
          <Alert className="border-status-warning/20 bg-status-warning/[0.06] text-status-warning-soft shadow-none">
            <AlertTriangleIcon />
            <AlertDescription className="text-status-warning-soft/90">
              Skipping {declinedRequiredCount} required{" "}
              {declinedRequiredCount === 1 ? "library" : "libraries"}. The addons that need{" "}
              {declinedRequiredCount === 1 ? "it" : "them"} won&apos;t load until{" "}
              {declinedRequiredCount === 1 ? "it is" : "they are"} installed.
            </AlertDescription>
          </Alert>
        )}
      </div>

      <DialogFooter className="sm:items-center sm:justify-between">
        <div className="flex flex-col gap-1.5">
          <label className="flex cursor-pointer select-none items-center gap-2">
            <Checkbox
              checked={alwaysAutoInstall}
              onCheckedChange={(checked) => setAlwaysAutoInstall(checked === true)}
            />
            <span className="text-xs text-muted-foreground">
              Always install dependencies automatically
            </span>
          </label>
          <label className="flex cursor-pointer select-none items-center gap-2">
            <Checkbox
              checked={rememberDeclines}
              onCheckedChange={(checked) => setRememberDeclines(checked === true)}
            />
            <span className="text-xs text-muted-foreground">
              Don&apos;t offer the ones I skip again
            </span>
          </label>
        </div>
        <div className="flex gap-2 sm:justify-end">
          <Button variant="outline" onClick={skipAll}>
            Skip all
          </Button>
          <Button onClick={confirm} disabled={selectedCount === 0 || overLimit}>
            <DownloadIcon data-icon="inline-start" />
            Install {selectedCount} {selectedCount === 1 ? "dependency" : "dependencies"}
          </Button>
        </div>
      </DialogFooter>
    </>
  );
}

function DependencyRow({
  dep,
  checked,
  onToggle,
}: {
  dep: PendingDependency;
  checked: boolean;
  onToggle: (name: string) => void;
}) {
  const warningId = useId();
  // Warn only for required dependencies the user has just unticked; the checkbox
  // itself stays enabled so the warning informs rather than blocks.
  const showWarning = dep.required && !checked;

  return (
    <GlassPanel variant="subtle" className="flex flex-col gap-2 px-3 py-2">
      <label className="flex cursor-pointer select-none items-center gap-3">
        <Checkbox
          checked={checked}
          onCheckedChange={() => onToggle(dep.name)}
          aria-describedby={showWarning ? warningId : undefined}
        />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm font-medium text-foreground">{dep.name}</span>
          {dep.requiredBy.length > 0 && (
            <span className="truncate text-xs text-muted-foreground">
              Needed by {joinNames(dep.requiredBy)}
            </span>
          )}
        </span>
        {dep.minVersion !== null && (
          <InfoPill color="muted" className="shrink-0 text-[10px]">
            v{dep.minVersion}+
          </InfoPill>
        )}
      </label>

      {showWarning && (
        <Alert
          id={warningId}
          className="border-status-warning/20 bg-status-warning/[0.06] text-status-warning-soft shadow-none"
        >
          <AlertTriangleIcon />
          <AlertDescription className="text-status-warning-soft/90">
            {/* Deliberately not naming requiredBy here: that list aggregates
                every addon that mentions this library, including any that
                declared it OPTIONAL, so naming them would claim breakage for
                addons that will load fine. The "Needed by" line above still
                shows the full list, where it is accurate. */}
            An addon that requires this won&apos;t load without it.
          </AlertDescription>
        </Alert>
      )}
    </GlassPanel>
  );
}

/** "A", "A and B", "A, B and C" — used in prose, so an Oxford-less join reads
 *  better than a bare comma list. */
function joinNames(names: readonly string[]): string {
  const list = names.filter((name) => name.trim() !== "");
  if (list.length === 0) return "";
  if (list.length === 1) return list[0]!;
  return `${list.slice(0, -1).join(", ")} and ${list[list.length - 1]!}`;
}
