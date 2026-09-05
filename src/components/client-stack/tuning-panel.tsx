import { useCallback, useEffect, useMemo, useState } from "react";
import {
  ArchiveIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  CircleCheckIcon,
  CircleHelpIcon,
  LockIcon,
  SlidersHorizontalIcon,
} from "lucide-react";

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
  ActivePath,
  TuningApplyOutcome,
  TuningEdit,
  TuningEntry,
  TuningField,
  TuningForm,
  TuningGroup,
  TuningProvenance,
  TuningSection,
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

/**
 * How a section's provenance is announced.
 *
 * Colour is never the only signal: three shipped themes are light, two are
 * high-contrast, and `status-*` is reseeded per theme — so every state carries
 * an icon *and* a word. The word is the user's rather than the type's:
 * "fossil" is the vocabulary of `TuningProvenance`, while "Not in force" is
 * what it means to somebody looking at a number on screen and deciding whether
 * it explains what their game is doing.
 */
const PROVENANCE_META: Record<
  TuningProvenance,
  { label: string; Icon: typeof CircleCheckIcon; color: "emerald" | "amber" | "muted" }
> = {
  live: { label: "In force", Icon: CircleCheckIcon, color: "emerald" },
  fossil: { label: "Not in force", Icon: ArchiveIcon, color: "amber" },
  unknown: { label: "Can't tell", Icon: CircleHelpIcon, color: "muted" },
};

/**
 * The one-line answer to "which setup is this client actually running?".
 *
 * Stated before any value is shown, because every value below is only
 * meaningful relative to it. `neither` is deliberately unalarming — a plain
 * ReShade install with no Neural Rendering add-on is the ordinary case and
 * nothing about it is wrong.
 */
const ACTIVE_PATH_LABEL: Record<ActivePath, string> = {
  direct: "Direct path — renodx-dlss.addon64 is loaded.",
  feed: "Feed path — renodx-dlss5.addon64 is loaded.",
  both: "Both paths — both add-ons are loaded, so both sets of settings are in force.",
  neither: "Neither path — no Neural Rendering add-on is loaded here.",
  unknown: "Unknown — Kalpa could not read the client folder.",
};

/**
 * Draft state keyed by field key. `null` means "not yet touched" — the loaded
 * value is used until the user changes it, and an untouched field is never part
 * of the diff sent to `apply_client_tuning`.
 *
 * Keyed by field key alone rather than by section-plus-key because typed fields
 * exist for exactly one section (`[RenoDX.DLSS5]`, the only one with a verified
 * table) and `apply_client_tuning` is scoped to that same section. If a second
 * section ever gains a field table, this key has to gain its section with it.
 */
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

/** Every typed field across the form, in section order. Only `[RenoDX.DLSS5]`
 *  contributes any; see `SectionCard` for why that asymmetry is deliberate. */
function allFields(form: TuningForm): TuningField[] {
  return form.sections.flatMap((section) => section.fields);
}

function groupFields(fields: TuningField[]): { group: TuningGroup; fields: TuningField[] }[] {
  return GROUP_ORDER.map((group) => ({
    group,
    fields: fields.filter((f) => f.group === group),
  })).filter((g) => g.fields.length > 0);
}

/**
 * The RenoDX tuning form: every `ReShade.ini` section the two RenoDX add-ons
 * own, each labelled with whether it is configuration **in force** or leftovers
 * from a path this client is not running.
 *
 * # Why this is not one flat list of settings
 *
 * It used to be. The panel read `[RenoDX.DLSS5]` and nothing else and presented
 * it as *the* tuning — but that section belongs to `renodx-dlss5.addon64`, the
 * **feed** path's add-on. On a direct-path install (`renodx-dlss.addon64`, the
 * arrangement that actually works and the one Kalpa's own load-order guidance
 * points at) that add-on sits on disk renamed aside, and the section is a
 * fossil. The panel showed `NeuralUplift=0` out of it, and a user *and* a
 * debugging session both concluded Neural Rendering was switched off while it
 * was in fact running fine on the other path. A stale value presented as
 * current is worse than no value at all, because it is actionable.
 *
 * So: say "this belongs to a parked add-on" rather than showing a fossil as
 * current. Nothing is hidden and nothing is deleted — the user may well switch
 * paths back, and silently dropping their saved settings would be the worse
 * failure — fossils are **labelled**, and they are not writable while they are
 * fossils.
 *
 * # The asymmetry between the sections is deliberate
 *
 * `[RenoDX.DLSS5]` has typed, labelled, editable fields because every label and
 * enum value was read out of the add-on binary's own string table.
 * `[RENODX-DLSS]` and `[RENODX-DLSS-preset*]` come back with `fields: []` and
 * all thirty of their keys as raw `entries`, because `renodx-dlss.addon64` is
 * closed source and no label, range or enum meaning for
 * `DirectNeuralRenderingEncoding=2` has been recovered. They are presented
 * read-only, as key and raw value, with no invented label — see the module doc
 * in `client_tuning.rs`. **This is finished work, not half-finished work.**
 * Attaching a guessed label to a *writable* control is how a working install
 * gets corrupted: the user trusts the label, moves the control, and Kalpa
 * writes a number whose real effect nobody here knows.
 *
 * There is also no master switch on the direct path. Nothing in `[RENODX-DLSS]`
 * is a verified enable flag, so this panel offers no "Neural Rendering: on/off"
 * for a direct-path install and must not grow one. `NeuralUplift` is the *feed*
 * path's switch, and on a direct-path install it renders here disabled, with
 * its value, beside the sentence saying the add-on that reads it is not loaded.
 *
 * # Draft, then apply — one write
 *
 * See `client_tuning.rs`'s module doc for why this is draft-then-apply rather
 * than per-control save: ReShade rewrites the whole INI file itself, and a save
 * on every slider drag would race it. `apply_note` carries the two facts that
 * follow from that — next launch, and only with ESO closed — and is shown
 * verbatim rather than restated here.
 *
 * Kalpa does not compete with the RenoDX Home-key overlay on tuning: the
 * overlay is live, knows the real float ranges, and needs no restart. This
 * panel is a recovery tool for when a bad value is blocking the overlay itself,
 * and it is the only place a parked path's settings can be seen at all.
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
      setDraft(buildDraft(allFields(next)));
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

  /**
   * The one section Kalpa may write, if any.
   *
   * The backend decides this — three conditions, all required: a verified field
   * table, the owning add-on live, and the section already present. The panel
   * never re-derives that rule, because a second implementation of it is a
   * second place for it to be wrong, and the write side re-checks it against
   * the folder as it is *now* regardless.
   */
  const writableSection = useMemo<TuningSection | null>(
    () => form?.sections.find((section) => section.writable) ?? null,
    [form]
  );

  const dirtyEdits = useMemo<TuningEdit[]>(() => {
    if (!writableSection) return [];
    return writableSection.fields
      .filter((field) => isDirty(field, draft))
      .map((field) => ({ key: field.key, value: draftValue(field, draft) ?? "" }));
  }, [writableSection, draft]);

  const handleDiscard = useCallback(() => {
    if (!form) return;
    setDraft(buildDraft(allFields(form)));
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

  return (
    <div className="space-y-3">
      {/* A measure, not the full 700px of the pane. Nine or ten words a line is
          the point at which the eye finds the next line without hunting; this
          paragraph was running to about a hundred and ten characters. */}
      <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
        Press Home in ESO to tune Neural Rendering. The overlay is live, knows the real ranges, and
        needs no restart — it is better at this than Kalpa. Use this page when you can&apos;t get
        there: a setting that blacks the screen or stops the overlay opening, to put the block back
        the way it was, or to read the settings belonging to a path this client is not running.
      </p>

      <PathSummary form={form} />

      {/* Fixed order, from the backend: the direct path's two sections, then
          the feed path's one. Nothing is filtered out — a section missing from
          the file is a fact worth stating, not a row to drop. */}
      {form.sections.map((section) => (
        <SectionCard
          key={section.section}
          section={section}
          draft={draft}
          editorOpen={editorOpen}
          advancedOpen={advancedOpen}
          onOpenEditor={() => setEditorOpen(true)}
          onToggleAdvanced={() => setAdvancedOpen((v) => !v)}
          onFieldChange={setFieldValue}
        />
      ))}

      {writableSection && editorOpen && (
        <>
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

          {/* Verbatim from the backend, and the panel keeps no copy of it: two
              facts that have each cost somebody an afternoon. The change lands
              at the next launch rather than now, and it only survives if ESO is
              closed, because ReShade rewrites ReShade.ini from memory when it
              exits and discards anything changed while it was running. */}
          <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
            {form.apply_note}
          </p>

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
/* Which path is live, and how Kalpa can tell                                 */
/* -------------------------------------------------------------------------- */

/**
 * The verdict, and the observations behind it.
 *
 * `path_evidence` is not decoration and is not collapsed away. Every value
 * below this card is labelled in force or not on the strength of it, and the
 * panel's previous version asked the user to take exactly that judgement on
 * trust — which is how a fossil came to be read as current tuning. Naming the
 * files that were and were not found is the panel's answer to "how do you
 * know?", and it is three or four short lines.
 */
function PathSummary({ form }: { form: TuningForm }) {
  const Icon = form.active_path === "unknown" ? CircleHelpIcon : CircleCheckIcon;
  return (
    <GlassPanel variant="subtle" className="space-y-2 p-3">
      <SectionHeader>Which setup is running</SectionHeader>
      <p className="flex items-start gap-2 text-xs leading-relaxed">
        <Icon aria-hidden className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
        <span>{ACTIVE_PATH_LABEL[form.active_path]}</span>
      </p>
      {form.path_evidence.length > 0 && (
        <ul className="space-y-0.5 border-t border-structure-06 pt-2 text-[11px] leading-5 text-muted-foreground">
          {form.path_evidence.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      )}
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* One section of ReShade.ini                                                 */
/* -------------------------------------------------------------------------- */

/**
 * One INI section: which add-on owns it, whether it is in force, and its values.
 *
 * Three shapes come through here, and the difference between them is the point
 * of the whole component:
 *
 * 1. **Writable** — `[RenoDX.DLSS5]` on a feed-path install. Opens as the
 *    compact read-only grid, because the Home-key overlay is the better place
 *    to tune and eighteen live controls is most of a starved pane's height;
 *    `Edit anyway…` swaps in the editor.
 * 2. **Typed but not writable** — the same section on a direct-path install,
 *    where its add-on is parked. The typed controls render *disabled*, values
 *    still in them. That is deliberate, and it is the answer to "where is the
 *    DLSS 5 toggle": `NeuralUplift` was never hidden, it is right here, and
 *    what the user actually needs is to see it, see its value, and see that
 *    moving it would do nothing because the add-on that reads it is not loaded.
 * 3. **Untyped** — the direct path's two sections, which have no field table at
 *    all. Raw key and value, collapsed behind a count, labelled undocumented.
 *
 * `read_only_reason` is rendered whenever the section is not writable. The
 * backend guarantees it is non-empty in that case, and a disabled control with
 * no sentence beside it reads as a missing feature rather than as a refusal.
 */
function SectionCard({
  section,
  draft,
  editorOpen,
  advancedOpen,
  onOpenEditor,
  onToggleAdvanced,
  onFieldChange,
}: {
  section: TuningSection;
  draft: Draft;
  editorOpen: boolean;
  advancedOpen: boolean;
  onOpenEditor: () => void;
  onToggleAdvanced: () => void;
  onFieldChange: (key: string, value: string) => void;
}) {
  const [entriesOpen, setEntriesOpen] = useState(false);
  const meta = PROVENANCE_META[section.provenance];
  const hasFields = section.fields.length > 0;
  const editing = section.writable && editorOpen;
  // The compact grid is the writable section's resting state only. A section
  // nobody can edit has no editor to reveal, so its values have to be legible
  // where they are.
  const compact = section.writable && hasFields && !editorOpen;

  return (
    <GlassPanel variant="subtle" className="space-y-3 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 truncate font-mono text-[12px] font-semibold">
          [{section.section}]
        </span>
        <InfoPill color={meta.color}>
          <meta.Icon aria-hidden className="size-3" />
          {meta.label}
        </InfoPill>
        {!section.writable && (
          <InfoPill color="muted">
            <LockIcon aria-hidden className="size-3" />
            Read-only
          </InfoPill>
        )}
      </div>

      {/* The owning add-on, always. A section name says nothing about which file
          writes it, and "which add-on is this" is the question every other
          statement on this card answers relative to. */}
      <p className="text-[11px] leading-5 text-muted-foreground">
        Written by <span className="font-mono">{section.owner}</span>.
      </p>

      {!section.writable && section.read_only_reason && (
        <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
          {section.read_only_reason}
        </p>
      )}

      {!section.present ? (
        <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
          <code>ReShade.ini</code> has no <code>[{section.section}]</code> section, so{" "}
          <span className="font-mono">{section.owner}</span> has never run in this client folder.
          There is nothing kept here to show.
        </p>
      ) : (
        <>
          {hasFields && compact && (
            <>
              <ReadOnlyValues fields={section.fields} entries={section.entries} />
              <Button variant="ghost" size="xs" onClick={onOpenEditor}>
                Edit anyway…
              </Button>
            </>
          )}

          {hasFields && !compact && (
            <div className="space-y-3">
              {groupFields(section.fields).map(({ group, fields }) =>
                group === "advanced" ? (
                  <AdvancedGroup
                    key={group}
                    open={advancedOpen}
                    onToggle={onToggleAdvanced}
                    fields={fields}
                    draft={draft}
                    disabled={!editing}
                    onFieldChange={onFieldChange}
                  />
                ) : (
                  <div key={group} className="space-y-3">
                    <SectionHeader>{GROUP_LABEL[group]}</SectionHeader>
                    {fields.map((field) => (
                      <FieldRow
                        key={field.key}
                        field={field}
                        value={editing ? draftValue(field, draft) : field.current}
                        dirty={editing && isDirty(field, draft)}
                        disabled={!editing}
                        onChange={(value) => onFieldChange(field.key, value)}
                      />
                    ))}
                  </div>
                )
              )}
            </div>
          )}

          {section.entries.length > 0 && !compact && (
            <RawEntries
              section={section}
              open={entriesOpen}
              onToggle={() => setEntriesOpen((v) => !v)}
            />
          )}
        </>
      )}
    </GlassPanel>
  );
}

/* -------------------------------------------------------------------------- */
/* Read-only value view — the writable section's resting state                */
/* -------------------------------------------------------------------------- */

/** Compact key/value listing of every known and unrecognised setting currently
 *  in the section. This is the writable section's default view — editing is a
 *  deliberate extra click (`Edit anyway…`), since the Home-key overlay is the
 *  better place to tune. Labels are shown only where the backend has a
 *  confirmed one (see `client_tuning.rs`); otherwise the raw key is the
 *  label. */
function ReadOnlyValues({ fields, entries }: { fields: TuningField[]; entries: TuningEntry[] }) {
  const orderedFields = GROUP_ORDER.flatMap((group) => fields.filter((f) => f.group === group));
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
    <div className="grid grid-cols-1 gap-x-10 gap-y-px text-[12px] sm:grid-cols-2">
      {orderedFields.map((field) => (
        <ReadOnlyRow key={field.key} rawKey={field.key} label={field.label} value={field.current} />
      ))}
      {entries.map((entry) => (
        <ReadOnlyRow key={entry.key} rawKey={entry.key} label={entry.key} value={entry.value} />
      ))}
    </div>
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
  // a size and a step of contrast and stops competing. The step is the token
  // change — the label inherits `foreground`, the key takes `muted-foreground`
  // — and *not* an alpha fade on top of it: `muted-foreground` already sits at
  // 4.5:1 on every theme, so multiplying it by an alpha drops it below, which
  // is what `text-alpha-utilities.test.ts` ratchets against.
  //
  // `tabular-nums` is the reason the value column exists at all: proportional
  // digits put `0`, `1.05`, `1.62` and `0.48` on four different right edges
  // even though every one of them is right-aligned.
  return (
    <div className="flex items-baseline justify-between gap-3 leading-6">
      <span className="min-w-0 truncate">
        {hasConfirmedLabel && <span className="mr-1.5 truncate font-medium">{label}</span>}
        <span className="font-mono text-[10px] text-muted-foreground">{rawKey}</span>
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

/**
 * One typed control.
 *
 * `disabled` is a first-class state here rather than an afterthought: a section
 * whose add-on is parked renders every one of these read-only *with its value
 * still in it*, so the setting can be found and understood rather than merely
 * reported as unavailable. The sentence explaining why sits once at the top of
 * the card (`read_only_reason`) rather than being repeated on every row.
 */
function FieldRow({
  field,
  value,
  dirty,
  disabled,
  onChange,
}: {
  field: TuningField;
  value: string | null;
  dirty: boolean;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  const isMasterSwitch = field.key === "NeuralUplift";

  if (field.control === "toggle") {
    const checked = value === "1";
    return (
      <label
        className={cn(
          "group/field flex items-start gap-3 rounded-lg",
          disabled ? "cursor-default opacity-70" : "cursor-pointer",
          isMasterSwitch && "border border-primary/20 bg-primary/[0.04] p-2"
        )}
      >
        <Checkbox
          checked={checked}
          disabled={disabled}
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
      <div className={cn(disabled && "opacity-70")}>
        <FieldLabel field={field} dirty={dirty} />
        <FieldHelp field={field} />
        <Select
          value={value ?? undefined}
          disabled={disabled}
          onValueChange={(next) => next && onChange(next)}
        >
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
      <div className={cn(disabled && "opacity-70")}>
        <FieldLabel field={field} dirty={dirty} />
        <FieldHelp field={field} />
        <div className="mt-1.5 flex items-center gap-3">
          <input
            type="range"
            className="h-1.5 flex-1 appearance-none rounded-full bg-structure-08 accent-primary enabled:cursor-pointer disabled:cursor-default"
            min={min}
            max={max}
            step={Math.pow(10, -field.decimals)}
            value={Number.isFinite(numeric) ? numeric : min}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value)}
          />
          <Input
            type="text"
            inputMode="decimal"
            value={displayValue}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value)}
            className="w-24 shrink-0 text-right"
          />
        </div>
      </div>
    );
  }

  // key_code
  return (
    <div className={cn(disabled && "opacity-70")}>
      <FieldLabel field={field} dirty={dirty} />
      <FieldHelp field={field} />
      <div className="mt-1.5 flex items-center gap-2">
        <Input
          type="text"
          inputMode="numeric"
          value={value ?? ""}
          disabled={disabled}
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
  disabled,
  onFieldChange,
}: {
  open: boolean;
  onToggle: () => void;
  fields: TuningField[];
  draft: Draft;
  disabled: boolean;
  onFieldChange: (key: string, value: string) => void;
}) {
  const dirtyCount = disabled ? 0 : fields.filter((f) => isDirty(f, draft)).length;
  return (
    <div className="rounded-lg border border-structure-06">
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
              value={disabled ? field.current : draftValue(field, draft)}
              dirty={!disabled && isDirty(field, draft)}
              disabled={disabled}
              onChange={(value) => onFieldChange(field.key, value)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Raw entries — undocumented keys, read-only by design                      */
/* -------------------------------------------------------------------------- */

/**
 * Every key no verified field table describes, verbatim and in file order.
 *
 * For `[RENODX-DLSS]` and `[RENODX-DLSS-preset*]` that is all thirty of them,
 * and it is not a gap to be filled in later. `renodx-dlss.addon64` is closed
 * source; no label, range or enum meaning for its keys has been recovered, and
 * inventing one and hanging a writable control off it is how a working install
 * gets corrupted. Key and value, plainly, is the honest answer, and it is the
 * same rule `client_tuning.rs` already follows for float ranges and key codes.
 *
 * Collapsed by default only because thirty rows is most of a starved pane. The
 * count is on the summary, so nothing is hidden by omission.
 */
function RawEntries({
  section,
  open,
  onToggle,
}: {
  section: TuningSection;
  open: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="rounded-lg border border-structure-06">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        className="flex w-full items-center justify-between gap-2 p-3 text-left"
      >
        <span className="min-w-0 flex-1 font-heading text-[11px] font-bold uppercase tracking-[0.05em] text-muted-foreground">
          {section.entries.length} undocumented setting{section.entries.length === 1 ? "" : "s"}
        </span>
        {open ? (
          <ChevronDownIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <ChevronRightIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
        )}
      </button>
      {open && (
        <div className="space-y-2 border-t border-structure-06 p-3">
          <p className="max-w-[72ch] text-xs leading-relaxed text-muted-foreground">
            <span className="font-mono">{section.owner}</span> is closed source, and Kalpa has not
            been able to verify what these keys mean. It shows them exactly as they are on disk,
            invents no label or range for them, and will not change them. That is deliberate rather
            than unfinished: a wrong label on a control you can move is how a working install gets
            broken.
          </p>
          <ul className="grid grid-cols-1 gap-x-10 gap-y-px text-[12px] sm:grid-cols-2">
            {section.entries.map((entry) => (
              <li key={entry.key} className="flex items-baseline justify-between gap-3 leading-6">
                <span className="min-w-0 truncate font-mono text-[11px] text-muted-foreground">
                  {entry.key}
                </span>
                <span className="shrink-0 font-mono tabular-nums">{entry.value}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
