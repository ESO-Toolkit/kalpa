import { ExternalLinkIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { SectionHeader } from "@/components/ui/section-header";
import { cn } from "@/lib/utils";

import { PresetPanel } from "./preset-panel";
import { RuntimeDriftCard } from "./runtime-drift-card";
import { TuningPanel } from "./tuning-panel";
import {
  FINDING_IMPACT,
  LEVEL_META,
  MV_PROVIDER_LABEL,
  ROLE_LABEL,
  ROLE_TO_SLOT,
  SLOT_LABEL,
  SLOT_SOURCE,
  SOURCE_LABEL,
  SOURCE_PILL_COLOR,
  findingsForSlot,
} from "./slots";
import type { Slot } from "./slots";
import type { ClientStack, HealthFinding, PreservedOriginal, StackItem } from "./types";

/**
 * What is in one slot, what is wrong with it, and what could be there instead.
 *
 * The order is fixed and it is the argument of the whole redesign: **state,
 * then problems, then options.** A finding renders in the same pane as the
 * control that fixes it, which is what retires the old "Show me where" button —
 * that button existed only because the diagnosis and its fix lived on different
 * tabs, and a link between two halves of one thought is a symptom, not a
 * feature.
 *
 * There is no options list yet for most slots, and the panes say so honestly
 * rather than rendering an empty chooser. A slot that holds exactly one thing
 * is a *fact*; it only reads as a stub if something on screen implies a choice
 * that cannot be offered. That is why `SOURCE_LABEL` is on screen from the
 * start: "Bring your own" is a true and complete statement about the DLSS slot
 * today, where a disabled "Install…" button would be a promise.
 */
export function SlotPane({
  slot,
  stack,
  onStackChanged,
  onOpenGuide,
}: {
  slot: Slot;
  stack: ClientStack;
  onStackChanged: () => Promise<void>;
  onOpenGuide: (url: string) => void;
}) {
  const findings = findingsForSlot(slot, stack);
  const source = SLOT_SOURCE[slot];
  const sourceLabel = SOURCE_LABEL[source];

  return (
    <section aria-labelledby={`slot-${slot}`} className="space-y-3">
      <div className="flex items-center justify-between gap-2">
        <SectionHeader id={`slot-${slot}`}>{SLOT_LABEL[slot]}</SectionHeader>
        {sourceLabel && <InfoPill color={SOURCE_PILL_COLOR[source]}>{sourceLabel}</InfoPill>}
      </div>

      <SlotContents slot={slot} stack={stack} />

      {findings.length > 0 && (
        <ul className="space-y-2">
          {findings.map((finding) => (
            <SlotFinding key={finding.id} finding={finding} onOpenGuide={onOpenGuide} />
          ))}
        </ul>
      )}

      {/* The controls that act on this slot, mounted in the slot they act on —
          once each. They used to render on both tabs, which meant two copies
          fetching the same state independently. */}
      <SlotActions slot={slot} stack={stack} onStackChanged={onStackChanged} />
    </section>
  );
}

/* -------------------------------------------------------------------------- */
/* Findings                                                                   */
/* -------------------------------------------------------------------------- */

/**
 * A finding, led by what the user will notice.
 *
 * The Rust `detail` explains the configuration — which file points where, which
 * technique sits above which — and that is what makes the diagnosis checkable.
 * But it does not answer the question someone actually arrives with, which is
 * whether this explains the thing that made them open the panel. The impact
 * line answers that, so it goes first and the configuration goes beneath it.
 *
 * It matters most for the silent failures. A wrong technique order and a
 * missing motion-vector provider both produce a stack where every file loads,
 * nothing errors, and the image is quietly wrong; for those, this line is the
 * only thing on screen connecting the diagnosis to what the player sees.
 */
function SlotFinding({
  finding,
  onOpenGuide,
}: {
  finding: HealthFinding;
  onOpenGuide: (url: string) => void;
}) {
  const meta = LEVEL_META[finding.level];
  const { Icon } = meta;
  const impact = FINDING_IMPACT[finding.id];

  return (
    <li
      className={cn(
        "rounded-xl border border-l-[3px] p-3 transition-colors duration-150",
        meta.border,
        meta.tint
      )}
    >
      <div className="flex items-start gap-2">
        <Icon aria-hidden className={cn("mt-0.5 size-4 shrink-0", meta.text)} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="font-heading text-[13px] font-semibold">{finding.title}</h4>
            {/* The level as a word: colour alone says nothing on the light and
                high-contrast themes. */}
            <span className={cn("text-[11px] font-semibold uppercase tracking-wide", meta.text)}>
              {meta.label}
            </span>
          </div>
          {impact && <p className="mt-1 text-xs leading-relaxed">{impact}</p>}
          <p className="mt-1 text-xs leading-relaxed text-muted-foreground">{finding.detail}</p>
          {finding.guide_url && (
            <Button
              variant="link"
              size="xs"
              className="mt-1 h-auto px-0"
              onClick={() => onOpenGuide(finding.guide_url!)}
            >
              Read the guide
              <ExternalLinkIcon />
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}

/* -------------------------------------------------------------------------- */
/* What is in the slot                                                        */
/* -------------------------------------------------------------------------- */

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <GlassPanel variant="subtle" className="p-3 text-xs leading-relaxed text-muted-foreground">
      {children}
    </GlassPanel>
  );
}

function SlotContents({ slot, stack }: { slot: Slot; stack: ClientStack }) {
  switch (slot) {
    case "shaders":
      return <ShadersContents stack={stack} />;
    case "motion":
      return <MotionContents stack={stack} />;
    case "preset":
      return <PresetContents stack={stack} />;
    case "tuning":
      return <TuningContents stack={stack} />;
    default:
      return <ItemsContents slot={slot} stack={stack} />;
  }
}

/** Copy for a slot with nothing in it. Each says something specific: "not
 *  present" tells the user nothing they could act on. */
const EMPTY_COPY: Partial<Record<Slot, string>> = {
  reshade: "No ReShade in this folder. ESO is running stock.",
  addons:
    "No ReShade add-ons here. The RenoDX DLSS 5 add-on is distributed through its Discord, so Kalpa cannot fetch it for you.",
  nr: "No Neural Rendering runtime. Kalpa never downloads NVIDIA runtimes — they are not licensed for redistribution, and there is no upstream to fetch them from.",
  sr: "Nothing swapped in, so ESO is using whatever it ships with.",
  shaders: "No shader tree here. Every effect ReShade can run comes from one of these packs.",
};

function ItemsContents({ slot, stack }: { slot: Slot; stack: ClientStack }) {
  const items = stack.items.filter((item) => ROLE_TO_SLOT[item.role] === slot);
  const originalFor = (name: string): PreservedOriginal | null =>
    stack.preserved_originals.find((o) => o.backs_up?.toLowerCase() === name.toLowerCase()) ?? null;

  if (items.length === 0) return <Empty>{EMPTY_COPY[slot] ?? "Nothing here."}</Empty>;

  return (
    <div className="space-y-2">
      {items.map((item) => (
        <ItemCard key={item.file_name} item={item} original={originalFor(item.file_name)} />
      ))}
      {slot === "addons" && stack.disabled_addons.length > 0 && (
        <p className="text-xs text-muted-foreground">
          Switched off in ReShade:{" "}
          <span className="font-mono">{stack.disabled_addons.join(", ")}</span>
        </p>
      )}
    </div>
  );
}

function ItemCard({ item, original }: { item: StackItem; original: PreservedOriginal | null }) {
  // No product name, no version, no company and no description means the PE
  // carried no version resource at all — so all Kalpa knows is the bytes.
  const identifiedByHash =
    !item.display_name && !item.version && !item.company && !item.description;

  return (
    <GlassPanel
      variant="subtle"
      className="space-y-1 rounded-xl border-l-[3px] border-l-structure-10 p-3"
    >
      <div className="flex flex-wrap items-center gap-2">
        <h4 className="font-heading text-[13px] font-semibold">{ROLE_LABEL[item.role]}</h4>
        {identifiedByHash && <InfoPill color="muted">identified by hash</InfoPill>}
      </div>
      <p className="text-xs text-muted-foreground">
        {item.display_name ?? "no product name"} &middot; {item.version ?? "no version info"}
      </p>
      <p className="font-mono text-[11px] text-muted-foreground">{item.file_name}</p>
      {original && (
        <p className="text-xs text-status-info">
          Your original &middot; {original.version ?? "no version info"} &middot; kept by you
        </p>
      )}
    </GlassPanel>
  );
}

function ShadersContents({ stack }: { stack: ClientStack }) {
  const shaders = stack.shaders;
  if (!shaders.present) return <Empty>{EMPTY_COPY.shaders}</Empty>;
  return (
    <GlassPanel variant="subtle" className="space-y-1 p-3 text-xs">
      <p>
        <span className="text-muted-foreground">Effects:</span> {shaders.effect_count} &middot;{" "}
        <span className="text-muted-foreground">textures:</span> {shaders.texture_count}
      </p>
      <p className="break-words">
        <span className="text-muted-foreground">Search paths:</span>{" "}
        <span className="font-mono">{shaders.effect_search_paths ?? "not configured"}</span>
      </p>
    </GlassPanel>
  );
}

/**
 * The motion-vector slot.
 *
 * This is a *setting*, not a file: `DLSS5_Feed.fx` names which provider it
 * reads, and the provider is whichever enabled technique actually writes the
 * vectors. So the slot shows the selection and who is currently honouring it.
 *
 * Kalpa cannot change the selection yet, and the pane says exactly that rather
 * than rendering a chooser that does nothing. The instruction points at the
 * ReShade overlay because that is where the user can genuinely do it today.
 */
function MotionContents({ stack }: { stack: ClientStack }) {
  const provider = stack.preset?.mv_provider;

  if (!provider) {
    return (
      <Empty>
        This preset does not enable DLSS5_Feed, so nothing is asking for motion vectors and there is
        no provider to choose.
      </Empty>
    );
  }

  return (
    <div className="space-y-2">
      <GlassPanel
        variant="subtle"
        className={cn(
          "space-y-1 rounded-xl border-l-[3px] p-3",
          provider.technique ? "border-l-status-success" : "border-l-status-danger"
        )}
      >
        <p className="text-xs">
          <span className="text-muted-foreground">DLSS5_Feed reads from</span>{" "}
          {MV_PROVIDER_LABEL[provider.kind]}
        </p>
        {provider.technique ? (
          <p className="font-mono text-[11px] text-muted-foreground">
            supplied by {provider.technique}
          </p>
        ) : (
          <p className="text-xs text-status-danger">Nothing enabled is supplying them.</p>
        )}
      </GlassPanel>
      <p className="text-xs leading-relaxed text-muted-foreground">
        Changing which effect supplies the motion vectors means editing the preset&apos;s
        preprocessor definitions and its technique order. Do it in ReShade&apos;s overlay for now —
        press Home in ESO. Kalpa cannot switch this yet.
      </p>
    </div>
  );
}

function PresetContents({ stack }: { stack: ClientStack }) {
  const preset = stack.preset;
  if (!preset) return <Empty>No preset configured in ReShade.ini.</Empty>;
  if (!preset.exists) {
    return (
      <Empty>
        ReShade.ini points at <span className="font-mono">{preset.path}</span>, and there is no file
        there.
      </Empty>
    );
  }

  return (
    <GlassPanel variant="subtle" className="space-y-2 p-3 text-xs">
      <p className="break-words">
        <span className="text-muted-foreground">Path:</span>{" "}
        <span className="font-mono">{preset.path}</span>
      </p>
      <div>
        <p className="mb-1 text-muted-foreground">Enabled techniques, in run order:</p>
        {preset.techniques.length === 0 ? (
          <p className="text-muted-foreground">None enabled.</p>
        ) : (
          <ol className="list-decimal space-y-1 pl-4">
            {preset.techniques.map((t) => (
              <li key={t.name} className="flex flex-wrap items-center gap-2">
                <span className="font-mono">{t.name}</span>
                {!t.source_present && <InfoPill color="red">missing {t.source}</InfoPill>}
                {preset.mv_provider?.technique === t.name && (
                  <InfoPill color="sky">motion vectors</InfoPill>
                )}
              </li>
            ))}
          </ol>
        )}
      </div>
    </GlassPanel>
  );
}

function TuningContents({ stack }: { stack: ClientStack }) {
  if (stack.tuning.length === 0) {
    return (
      <Empty>
        RenoDX DLSS 5 has never run here — ReShade.ini has no [RenoDX.DLSS5] section yet.
      </Empty>
    );
  }
  // Nothing here on purpose. `TuningPanel` below leads with the overlay and
  // then lists the same values with the add-on's own labels beside them, so a
  // raw key/value table here would be the same content twice in one pane.
  return null;
}

/* -------------------------------------------------------------------------- */
/* The controls that act on a slot                                            */
/* -------------------------------------------------------------------------- */

function SlotActions({
  slot,
  stack,
  onStackChanged,
}: {
  slot: Slot;
  stack: ClientStack;
  onStackChanged: () => Promise<void>;
}) {
  // The drift card is scoped to the runtime files of its own slot. It used to
  // be handed every managed path from the overview as well as the per-layer
  // list, so it mounted twice and fetched twice.
  if (slot === "nr" || slot === "sr") {
    const filePaths = stack.items
      .filter((item) => ROLE_TO_SLOT[item.role] === slot)
      .map((item) => item.file_name);
    return (
      <RuntimeDriftCard
        clientDir={stack.client_dir}
        stack={stack}
        onChanged={onStackChanged}
        filePaths={filePaths}
      />
    );
  }
  if (slot === "preset") {
    return <PresetPanel clientDir={stack.client_dir} stack={stack} onChanged={onStackChanged} />;
  }
  if (slot === "tuning") {
    return <TuningPanel clientDir={stack.client_dir} stack={stack} onChanged={onStackChanged} />;
  }
  return null;
}
