import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronDownIcon, ChevronRightIcon, SlidersHorizontalIcon } from "lucide-react";

import { GlassPanel } from "@/components/ui/glass-panel";
import { SectionHeader } from "@/components/ui/section-header";
import { InfoPill } from "@/components/ui/info-pill";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";
import { approveClientWrites } from "@/components/client-stack/approve";
import { cn } from "@/lib/utils";
import type { StackPanelProps } from "@/components/client-stack/panel-props";
import type {
  TuningApplyOutcome,
  TuningEdit,
  TuningField,
  TuningForm,
  TuningGroup,
} from "@/components/client-stack/types";

/** The add-on's own grouping order — not alphabetical, not by importance as
 *  Kalpa sees it, but the order the RenoDX DLSS 5 UI itself uses. */
const GROUP_ORDER: TuningGroup[] = ["neural_rendering", "detail", "color", "keys", "advanced"];

const GROUP_LABEL: Record<TuningGroup, string> = {
  neural_rendering: "Neural Rendering",
  detail: "Detail",
  color: "Color",
  keys: "Keys",
  // The add-on's own section header for these settings, verbatim — see the
  // module doc in `client_tuning.rs`.
  advanced: "Guide overrides (leave at defaults unless diagnostics require them)",
};

/** Draft state keyed by field key. `null` means "not yet touched" — the
 *  loaded value is used until the user changes it, and an untouched field is
 *  never part of the diff sent to `apply_client_tuning`. */
type Draft = Record<string, string | null>;

function buildDraft(fields: TuningField[]): Draft {
  const draft: Draft = {};
  for (const field of fields) draft[field.key] = null;
  return draft;
}

function draftValue(field: TuningField, draft: Draft): string | null {
  const edited = draft[field.key];
  return edited === null || edited === undefined ? field.current : edited;
}

function isDirty(field: TuningField, draft: Draft): boolean {
  const edited = draft[field.key];
  return edited !== null && edited !== undefined && edited !== field.current;
}

/** The slider's live max: the display range from the backend, widened to
 *  cover whatever the numeric box currently holds. Never used to clamp —
 *  only to keep the thumb reachable while the box holds a larger value. */
function liveSliderMax(field: TuningField, current: string | null): number {
  const base = field.slider_max ?? 1;
  const parsed = current === null ? NaN : Number.parseFloat(current);
  if (Number.isFinite(parsed) && parsed > base) return parsed;
  return base;
}

function liveSliderMin(field: TuningField, current: string | null): number {
  const base = field.slider_min ?? 0;
  const parsed = current === null ? NaN : Number.parseFloat(current);
  if (Number.isFinite(parsed) && parsed < base) return parsed;
  return base;
}

/**
 * The RenoDX DLSS 5 tuning form: a draft the user edits freely, applied in one
 * write when they ask for it.
 *
 * See `src-tauri/src/client_tuning.rs`'s module doc for why this is
 * draft-then-apply rather than per-control save: ReShade rewrites the whole
 * INI file itself, and a save on every slider drag would race it.
 *
 * Kalpa does not compete with the RenoDX Home-key overlay on tuning — the
 * overlay is live, knows the real float ranges, and needs no restart. This
 * panel is a recovery tool: it opens read-only, and editing is one explicit
 * click away, for the case where a bad value is blocking the overlay itself.
 */
export function TuningPanel({ clientDir, onChanged }: StackPanelProps) {
  const [form, setForm] = useState<TuningForm | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [draft, setDraft] = useState<Draft>({});
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [editorOpen, setEditorOpen] = useState(false);

  const [applying, setApplying] = useState(false);
  const [applyError, setApplyError] = useState<string | null>(null);
  const [applied, setApplied] = useState(false);
  const [lastBackupId, setLastBackupId] = useState<string | null>(null);

  const load = useCallback(async (dir: string) => {
    setLoading(true);
    setLoadError(null);
    setApplyError(null);
    setApplied(false);
    try {
      const next = await invokeOrThrow<TuningForm>("read_client_tuning", { clientDir: dir });
      setForm(next);
      setDraft(buildDraft(next.fields));
    } catch (e) {
      setForm(null);
      setLoadError(getTauriErrorMessage(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // `load` flips the loading flag before its first await, which the rule
    // reads as a synchronous setState. That is the intended behaviour here —
    // the spinner has to appear on the same commit `clientDir` changes — and
    // matches the existing pattern in `client-health.tsx`.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load(clientDir);
    setLastBackupId(null);
    setEditorOpen(false);
  }, [clientDir, load]);

  const setFieldValue = useCallback((key: string, value: string) => {
    setApplied(false);
    setDraft((prev) => ({ ...prev, [key]: value }));
  }, []);

  const dirtyEdits = useMemo<TuningEdit[]>(() => {
    if (!form) return [];
    return form.fields
      .filter((field) => isDirty(field, draft))
      .map((field) => ({ key: field.key, value: draftValue(field, draft) ?? "" }));
  }, [form, draft]);

  const handleDiscard = useCallback(() => {
    if (!form) return;
    setDraft(buildDraft(form.fields));
    setApplyError(null);
    setApplied(false);
  }, [form]);

  const handleApply = useCallback(async () => {
    if (dirtyEdits.length === 0) return;
    setApplying(true);
    setApplyError(null);
    setApplied(false);
    try {
      await approveClientWrites(clientDir);
      const outcome = await invokeOrThrow<TuningApplyOutcome>("apply_client_tuning", {
        clientDir,
        edits: dirtyEdits,
      });
      setApplied(true);
      setLastBackupId(outcome.backup_id);
      await onChanged();
      await load(clientDir);
    } catch (e) {
      setApplyError(getTauriErrorMessage(e));
    } finally {
      setApplying(false);
    }
  }, [clientDir, dirtyEdits, load, onChanged]);

  if (loading) {
    return (
      <div className="flex items-center gap-3 py-6 text-sm text-muted-foreground" role="status">
        <span
          aria-hidden
          className="inline-block size-4 animate-spin rounded-full border-2 border-structure-10 border-t-primary"
        />
        <span>Reading ReShade.ini...</span>
      </div>
    );
  }

  if (loadError) {
    return (
      <GlassPanel variant="subtle" className="p-3 text-sm" role="alert">
        <p className="font-heading text-[13px] font-semibold text-status-danger">
          Could not read tuning settings
        </p>
        <p className="mt-0.5 text-xs text-muted-foreground">{loadError}</p>
      </GlassPanel>
    );
  }

  if (!form) return null;

  if (!form.section_present) {
    return (
      <GlassPanel variant="subtle" className="p-3 text-xs text-muted-foreground">
        RenoDX DLSS 5 has never run on this client — <code>ReShade.ini</code> has no{" "}
        <code>[RenoDX.DLSS5]</code> section yet. Launch the game once with the add-on active, then
        come back here to tune it.
      </GlassPanel>
    );
  }

  const groups = GROUP_ORDER.map((group) => ({
    group,
    fields: form.fields.filter((f) => f.group === group),
  })).filter((g) => g.fields.length > 0);

  return (
    <div className="space-y-3">
      {/* A measure, not the full 700px of the pane. Nine or ten words a line is
          the point at which the eye finds the next line without hunting; this
          paragraph was running to about a hundred and ten characters. */}
      <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
        Press Home in ESO to tune Neural Rendering. The overlay is live, knows the real ranges, and
        needs no restart — it is better at this than Kalpa. Use this page when you can&apos;t get
        there: a setting that blacks the screen or stops the overlay opening, or to put the block
        back the way it was.
      </p>

      {/* Hidden once the editor is open: the controls below show the same
          values as live fields, and in a ~348px pane two readings of eighteen
          settings is most of the height for none of the information. */}
      {!editorOpen && (
        <>
          <ReadOnlyValues form={form} />
          <Button variant="ghost" size="xs" onClick={() => setEditorOpen(true)}>
            Edit anyway…
          </Button>
        </>
      )}

      {editorOpen && (
        <>
          <div className="border-t border-structure-06" />

          {groups.map(({ group, fields }) =>
            group === "advanced" ? (
              <AdvancedGroup
                key={group}
                open={advancedOpen}
                onToggle={() => setAdvancedOpen((v) => !v)}
                fields={fields}
                draft={draft}
                onFieldChange={setFieldValue}
              />
            ) : (
              <GlassPanel key={group} variant="subtle" className="space-y-3 p-3">
                <SectionHeader>{GROUP_LABEL[group]}</SectionHeader>
                <div className="space-y-3">
                  {fields.map((field) => (
                    <FieldRow
                      key={field.key}
                      field={field}
                      value={draftValue(field, draft)}
                      dirty={isDirty(field, draft)}
                      onChange={(value) => setFieldValue(field.key, value)}
                    />
                  ))}
                </div>
              </GlassPanel>
            )
          )}

          {form.unknown.length > 0 && <UnknownSettings unknown={form.unknown} />}

          <div className="border-t border-structure-06" />

          <div className="flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              disabled={dirtyEdits.length === 0 || applying}
              onClick={() => void handleApply()}
            >
              {applying ? (
                <span
                  aria-hidden
                  className="inline-block size-3.5 animate-spin rounded-full border-2 border-structure-10 border-t-primary"
                />
              ) : (
                <SlidersHorizontalIcon />
              )}
              {applying ? "Applying..." : "Apply"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={dirtyEdits.length === 0 || applying}
              onClick={handleDiscard}
            >
              Discard changes
            </Button>
            {dirtyEdits.length > 0 && !applying && (
              <span className="text-xs text-muted-foreground">
                {dirtyEdits.length} unsaved change{dirtyEdits.length === 1 ? "" : "s"}
              </span>
            )}
            {applied && dirtyEdits.length === 0 && !applying && (
              <InfoPill color="sky">Applies at next launch</InfoPill>
            )}
          </div>

          {applied && lastBackupId && dirtyEdits.length === 0 && !applying && (
            <p className="text-xs text-muted-foreground">
              The previous <code>ReShade.ini</code> was kept in Kalpa&apos;s backups.
            </p>
          )}

          {applyError && (
            <p className="text-xs text-status-danger" role="alert">
              {applyError}
            </p>
          )}
        </>
      )}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Read-only value view — the default state                                   */
/* -------------------------------------------------------------------------- */

/** Compact key/value listing of every known and unknown setting currently in
 *  the file. This is the panel's default view — editing is a deliberate
 *  extra click (`Edit anyway…`), since the Home-key overlay is the better
 *  place to tune. Labels are shown only where the backend has a confirmed
 *  one (see `client_tuning.rs`); otherwise the raw key is the label. */
function ReadOnlyValues({ form }: { form: TuningForm }) {
  const orderedFields = GROUP_ORDER.flatMap((group) =>
    form.fields.filter((f) => f.group === group)
  );
  // Two columns. Eighteen settings stacked singly is ~450px of a pane that has
  // ~505px for everything including the overlay lead and the editor button;
  // paired, the same values are ~225px. Each row is short — a label, a mono key
  // and a mono value — so the width was going spare in a dialog that is 1000px
  // wide and vertically starved.
  //
  // The rows sit on a 25px baseline grid (`leading-6` plus a hairline gap)
  // rather than on whatever line height the pane happened to inherit, and the
  // columns are 40px apart, because at 24px a value in the left column and a
  // label in the right one read as a single run of text.
  return (
    <GlassPanel
      variant="subtle"
      className="grid grid-cols-1 gap-x-10 gap-y-px p-3 text-[12px] sm:grid-cols-2"
    >
      {orderedFields.map((field) => (
        <ReadOnlyRow key={field.key} rawKey={field.key} label={field.label} value={field.current} />
      ))}
      {form.unknown.map(([key, value]) => (
        <ReadOnlyRow key={key} rawKey={key} label={key} value={value} />
      ))}
    </GlassPanel>
  );
}

function ReadOnlyRow({
  rawKey,
  label,
  value,
}: {
  rawKey: string;
  label: string;
  value: string | null;
}) {
  const hasConfirmedLabel = label !== rawKey;
  // Two faces per row, one rank each. The label was 12px sans and the raw INI
  // key 11px mono directly beside it — a one-pixel difference between two
  // typefaces of near-equal colour, so every row asked to be read twice. The
  // key is the *provenance* of the value, not a second name for it, so it drops
  // a size and a step of contrast and stops competing.
  //
  // `tabular-nums` is the reason the value column exists at all: proportional
  // digits put `0`, `1.05`, `1.62` and `0.48` on four different right edges
  // even though every one of them is right-aligned.
  return (
    <div className="flex items-baseline justify-between gap-3 leading-6">
      <span className="min-w-0 truncate">
        {hasConfirmedLabel && <span className="mr-1.5 truncate font-medium">{label}</span>}
        <span className="font-mono text-[10px] text-muted-foreground/75">{rawKey}</span>
      </span>
      <span className="shrink-0 font-mono tabular-nums">{value ?? "—"}</span>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Field rows, by control                                                    */
/* -------------------------------------------------------------------------- */

function FieldLabel({ field, dirty }: { field: TuningField; dirty: boolean }) {
  return (
    <div className="flex min-w-0 flex-1 items-center gap-2">
      <span className="truncate text-sm font-medium">{field.label}</span>
      {dirty && <InfoPill color="amber">Unsaved</InfoPill>}
    </div>
  );
}

function FieldHelp({ field }: { field: TuningField }) {
  if (!field.help) return null;
  return <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">{field.help}</p>;
}

function FieldRow({
  field,
  value,
  dirty,
  onChange,
}: {
  field: TuningField;
  value: string | null;
  dirty: boolean;
  onChange: (value: string) => void;
}) {
  const isMasterSwitch = field.key === "NeuralUplift";

  if (field.control === "toggle") {
    const checked = value === "1";
    return (
      <label
        className={cn(
          "group/field flex cursor-pointer items-start gap-3 rounded-lg",
          isMasterSwitch && "border border-primary/20 bg-primary/[0.04] p-2"
        )}
      >
        <Checkbox
          checked={checked}
          onCheckedChange={(next) => onChange(next ? "1" : "0")}
          className="mt-0.5"
        />
        <div className="min-w-0 flex-1">
          <FieldLabel field={field} dirty={dirty} />
          <FieldHelp field={field} />
        </div>
      </label>
    );
  }

  if (field.control === "choice") {
    return (
      <div>
        <FieldLabel field={field} dirty={dirty} />
        <FieldHelp field={field} />
        <Select value={value ?? undefined} onValueChange={(next) => next && onChange(next)}>
          <SelectTrigger className="mt-1.5 w-full">
            <SelectValue placeholder="Not set" />
          </SelectTrigger>
          <SelectContent>
            {field.choices.map((choice) => (
              <SelectItem key={choice.value} value={String(choice.value)}>
                {choice.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    );
  }

  if (field.control === "float") {
    const numeric = value === null ? NaN : Number.parseFloat(value);
    const min = liveSliderMin(field, value);
    const max = liveSliderMax(field, value);
    const displayValue = value ?? "";
    return (
      <div>
        <FieldLabel field={field} dirty={dirty} />
        <FieldHelp field={field} />
        <div className="mt-1.5 flex items-center gap-3">
          <input
            type="range"
            className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-structure-08 accent-primary"
            min={min}
            max={max}
            step={Math.pow(10, -field.decimals)}
            value={Number.isFinite(numeric) ? numeric : min}
            onChange={(e) => onChange(e.target.value)}
          />
          <Input
            type="text"
            inputMode="decimal"
            value={displayValue}
            onChange={(e) => onChange(e.target.value)}
            className="w-24 shrink-0 text-right"
          />
        </div>
      </div>
    );
  }

  // key_code
  return (
    <div>
      <FieldLabel field={field} dirty={dirty} />
      <FieldHelp field={field} />
      <div className="mt-1.5 flex items-center gap-2">
        <Input
          type="text"
          inputMode="numeric"
          value={value ?? ""}
          onChange={(e) => onChange(e.target.value)}
          className="w-24"
        />
        <span className="text-xs text-muted-foreground">key code</span>
      </div>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Advanced group — collapsed by default                                     */
/* -------------------------------------------------------------------------- */

function AdvancedGroup({
  open,
  onToggle,
  fields,
  draft,
  onFieldChange,
}: {
  open: boolean;
  onToggle: () => void;
  fields: TuningField[];
  draft: Draft;
  onFieldChange: (key: string, value: string) => void;
}) {
  const dirtyCount = fields.filter((f) => isDirty(f, draft)).length;
  return (
    <GlassPanel variant="subtle" className="p-0">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center justify-between gap-2 p-3 text-left"
      >
        {/* Same rank as the `SectionHeader` on every other group, so it takes
            that face, size and weight. It does not take the uppercasing: this
            label is a sentence with a parenthetical, and set in caps it becomes
            a shout nobody reads. */}
        <span className="min-w-0 flex-1 font-heading text-[11px] font-bold text-muted-foreground">
          {GROUP_LABEL.advanced}
        </span>
        <span className="flex items-center gap-2">
          {dirtyCount > 0 && <InfoPill color="amber">{dirtyCount} unsaved</InfoPill>}
          {open ? (
            <ChevronDownIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
          ) : (
            <ChevronRightIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
          )}
        </span>
      </button>
      {open && (
        <div className="space-y-3 border-t border-structure-06 p-3">
          {fields.map((field) => (
            <FieldRow
              key={field.key}
              field={field}
              value={draftValue(field, draft)}
              dirty={isDirty(field, draft)}
              onChange={(value) => onFieldChange(field.key, value)}
            />
          ))}
        </div>
      )}
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* Unknown keys — read-only                                                  */
/* -------------------------------------------------------------------------- */

function UnknownSettings({ unknown }: { unknown: [string, string][] }) {
  return (
    <GlassPanel variant="subtle" className="space-y-2 p-3">
      <SectionHeader>Not recognised by this Kalpa build</SectionHeader>
      <p className="text-xs text-muted-foreground">
        These keys are in the <code>[RenoDX.DLSS5]</code> section but this build does not know what
        they do. Kalpa leaves them untouched.
      </p>
      <ul className="space-y-1">
        {unknown.map(([key, value]) => (
          <li key={key} className="flex items-center justify-between gap-3 font-mono text-[11px]">
            <span className="truncate text-muted-foreground">{key}</span>
            <span className="truncate">{value}</span>
          </li>
        ))}
      </ul>
    </GlassPanel>
  );
}
