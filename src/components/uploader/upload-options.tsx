// Region + visibility controls shown before an upload. Defaults persist via the
// parent (last choice), and each visibility tier shows a one-line consequence.

import { Globe, Link2, Lock } from "lucide-react";
import { SectionHeader } from "@/components/ui/section-header";
import { cn } from "@/lib/utils";
import { REGION_OPTIONS, type UploadOptions, type Visibility } from "@/types/uploader";

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
}: {
  options: UploadOptions;
  onChange: (next: UploadOptions) => void;
  disabled?: boolean;
}) {
  return (
    <div className="space-y-4">
      <div className="space-y-2">
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
              disabled={disabled}
              onClick={() => onChange({ ...options, region: r.id })}
              className={cn(
                "flex-1 rounded-lg border px-3 py-2 text-sm transition-colors duration-150",
                "disabled:opacity-50",
                options.region === r.id
                  ? "border-sky-400/40 bg-sky-400/[0.06] text-foreground"
                  : "border-white/[0.06] bg-white/[0.02] text-muted-foreground hover:border-white/[0.12]"
              )}
            >
              {r.label}
            </button>
          ))}
        </div>
      </div>

      <div className="space-y-2">
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
                disabled={disabled}
                onClick={() => onChange({ ...options, visibility: value })}
                className={cn(
                  "rounded-lg border p-3 text-left transition-colors duration-150 disabled:opacity-50",
                  active
                    ? "border-[#c4a44a]/40 bg-[#c4a44a]/[0.05]"
                    : "border-white/[0.06] bg-white/[0.02] hover:border-white/[0.12]"
                )}
              >
                <div
                  className={cn(
                    "flex items-center gap-1.5 text-sm font-medium",
                    active ? "text-[#c4a44a]" : "text-foreground/80"
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
      </div>
    </div>
  );
}
