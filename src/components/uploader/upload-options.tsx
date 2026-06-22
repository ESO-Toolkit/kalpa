// Report name, region + visibility controls shown before an upload. Defaults
// persist via the parent (last choice), and each visibility tier shows a
// one-line consequence. The report name carries smart suggestions + run-tags.

import { Globe, Link2, Lock, Sparkles, Tag } from "lucide-react";
import { Input } from "@/components/ui/input";
import { SectionHeader } from "@/components/ui/section-header";
import { cn } from "@/lib/utils";
import {
  REGION_OPTIONS,
  type FightSummary,
  type UploadOptions,
  type Visibility,
} from "@/types/uploader";
import { RUN_TAGS, suggestReportName, withTag } from "./naming";

const VISIBILITY_TIERS: {
  value: Visibility;
  label: string;
  hint: string;
  Icon: typeof Globe;
}[] = [
  {
    value: "public",
    label: "Public",
    hint: "Anyone can find it; it ranks on the leaderboards.",
    Icon: Globe,
  },
  {
    value: "unlisted",
    label: "Unlisted",
    hint: "Only people with the link can open it.",
    Icon: Link2,
  },
  {
    value: "private",
    label: "Private",
    hint: "Only you (or your guild) can see it.",
    Icon: Lock,
  },
];

export function UploadOptionsControl({
  options,
  onChange,
  disabled,
  // When direct upload is the intended path, visibility is applied immediately;
  // the official uploader instead asks the user to confirm before publishing.
  willUseNative = false,
  // Parsed fights + the log's time, used to suggest a report name from content.
  fights = [],
  whenMs = null,
}: {
  options: UploadOptions;
  onChange: (next: UploadOptions) => void;
  disabled?: boolean;
  willUseNative?: boolean;
  fights?: FightSummary[];
  whenMs?: number | null;
}) {
  const name = options.description ?? "";
  const setName = (v: string) => onChange({ ...options, description: v || null });
  const suggestion = suggestReportName(fights, whenMs);
  const showSuggest = name.trim() !== suggestion;

  return (
    // Hairline dividers between the three groups read as one cohesive form rather
    // than flat siblings — without over-containering (no nested boxes inside the
    // already-raised options panel).
    <div className="divide-y divide-white/[0.06]">
      {/* Report name only has effect on the DIRECT upload path — the official
          uploader/CLI ignores options.description. Show it only when direct
          upload is the intended path so a typed name never silently does nothing. */}
      {willUseNative && (
        <div className="space-y-2 pb-4">
          <SectionHeader>Report name</SectionHeader>
          <Input
            value={name}
            disabled={disabled}
            onChange={(e) => setName(e.target.value)}
            placeholder={suggestion}
            aria-label="Report name"
            maxLength={120}
          />
          <div className="flex flex-wrap items-center gap-1.5">
            {showSuggest && (
              <button
                type="button"
                disabled={disabled}
                onClick={() => setName(suggestion)}
                className="inline-flex items-center gap-1 rounded-md border border-accent-sky/25 bg-accent-sky/[0.06] px-2 py-0.5 text-[11px] font-medium text-accent-sky transition-colors hover:bg-accent-sky/[0.12] disabled:opacity-50"
              >
                <Sparkles className="size-3" aria-hidden />
                {suggestion}
              </button>
            )}
            <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/70">
              <Tag className="size-3" aria-hidden />
            </span>
            {RUN_TAGS.map((t) => (
              <button
                key={t.id}
                type="button"
                disabled={disabled}
                title={t.hint}
                onClick={() => setName(withTag(name || suggestion, t.id))}
                className="rounded-md border border-white/[0.08] bg-white/[0.03] px-2 py-0.5 text-[11px] font-medium text-muted-foreground transition-colors hover:border-white/[0.16] hover:text-foreground/80 disabled:opacity-50"
              >
                {t.label}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="space-y-2 py-4 first:pt-0">
        <SectionHeader>Region</SectionHeader>
        {/* Plain toggle buttons (aria-pressed) rather than a radiogroup: a true
            radiogroup implies roving-tabindex arrow-key nav we don't implement,
            so aria-pressed matches the actual Tab+Enter behavior (and ModeTab). */}
        <div className="flex gap-2" role="group" aria-label="Region">
          {REGION_OPTIONS.map((r) => (
            <button
              key={r.id}
              type="button"
              aria-pressed={options.region === r.id}
              aria-label={`${r.label} region`}
              disabled={disabled}
              onClick={() => onChange({ ...options, region: r.id })}
              className={cn(
                "flex-1 rounded-lg border px-3 py-2 text-sm transition-colors duration-150",
                "focus-visible:border-accent-sky/40 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                "disabled:opacity-50",
                options.region === r.id
                  ? "border-accent-sky/40 bg-accent-sky/[0.06] text-foreground"
                  : "border-white/[0.06] bg-white/[0.02] text-muted-foreground hover:border-white/[0.12]"
              )}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>

      <div className="space-y-2 pt-4">
        <SectionHeader>Visibility</SectionHeader>
        <div
          className="grid grid-cols-1 gap-2 sm:grid-cols-3"
          role="group"
          aria-label="Report visibility"
        >
          {VISIBILITY_TIERS.map(({ value, label, hint, Icon }) => {
            const active = options.visibility === value;
            return (
              <button
                key={value}
                type="button"
                aria-pressed={active}
                aria-label={`${label} visibility — ${hint}`}
                disabled={disabled}
                onClick={() => onChange({ ...options, visibility: value })}
                className={cn(
                  "rounded-lg border p-3 text-left transition-colors duration-150 disabled:opacity-50",
                  "focus-visible:border-accent-sky/40 focus-visible:ring-2 focus-visible:ring-accent-sky/30 focus-visible:outline-none",
                  // Selection state is uniformly sky across the uploader (Region,
                  // mode tabs, visibility); gold is reserved for primary actions.
                  active
                    ? "border-accent-sky/40 bg-accent-sky/[0.06]"
                    : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                )}
              >
                <div
                  className={cn(
                    "flex items-center gap-1.5 text-sm font-medium",
                    active ? "text-accent-sky" : "text-foreground/80"
                  )}
                >
                  <Icon className="size-3.5" aria-hidden />
                  {label}
                </div>
                <div className="mt-1 text-[11px] leading-snug text-muted-foreground">{hint}</div>
              </button>
            );
          })}
        </div>
        <p className="text-[11px] text-muted-foreground/80">
          {willUseNative
            ? "Direct upload applies this visibility immediately."
            : "You'll confirm visibility in the ESO Logs Uploader before the report goes live."}
        </p>
      </div>
    </div>
  );
}
