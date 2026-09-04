import { DownloadIcon, ExternalLinkIcon, HandIcon, SlidersHorizontalIcon } from "lucide-react";

import { Button } from "@/components/ui/button";
import { GlassPanel } from "@/components/ui/glass-panel";
import { InfoPill } from "@/components/ui/info-pill";
import { cn } from "@/lib/utils";

import { PresetPanel } from "./preset-panel";
import { RuntimeDriftCard } from "./runtime-drift-card";
import { ShaderPacksPanel } from "./shader-packs-panel";
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
  SOURCE_META,
  findingsForSlot,
} from "./slots";
import type { Slot, SlotSource } from "./slots";
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

  const meta = SOURCE_META[source];

  return (
    // The pane is an *object*, not a region.
    //
    // It used to be two floating cards above 500px of nothing, which reads as
    // unfinished rather than spacious. Given a titled top edge and a bounded
    // bottom edge, the same empty space becomes the interior of a designed
    // container — composure instead of abandonment — and it costs no content
    // Kalpa does not already have.
    <section
      aria-labelledby={`slot-${slot}`}
      className="flex min-h-full flex-col overflow-hidden rounded-xl border border-structure-06 bg-glass-bg-light shadow-[0_4px_16px_var(--scrim-20)]"
    >
      {/* The dialog's own header recipe, reused once. A 4% structural fade is
          the house way of saying "this strip is a header"; it is not
          decoration, and it appears exactly twice in the panel. */}
      <header className="flex shrink-0 items-center justify-between gap-3 border-b border-structure-06 bg-gradient-to-b from-structure-04 to-transparent px-4 py-3">
        {/* A pane title, not an eyebrow. This was an 11px uppercase
            SectionHeader — the *least* important type style in the system used
            for the most important text in the pane, so the card headings below
            outranked it and nothing anchored the column. SectionHeader is still
            right one level down, labelling sub-sections.

            15px, not 18px. At 18/600 it outranked the dialog's own title
            ("Graphics stack", 16/600), so the child was louder than the frame
            containing it. 15/600 still clears the 14/500 card headings beneath
            it — which is the whole job — without arguing with the header. */}
        <h3
          id={`slot-${slot}`}
          className="truncate font-heading text-[15px] leading-5 font-semibold tracking-[-0.01em]"
        >
          {SLOT_LABEL[slot]}
        </h3>
        {sourceLabel && (
          // Provenance: whose hand is on this thing. The word is mandatory —
          // on the light themes the status hues collapse towards one near-black
          // and the label is what actually carries it. Which is why it is set
          // as a micro-label rather than a whisper of body text: on a theme
          // where `--primary` lands near the foreground (Elsweyr Moons reseeds
          // it to a pale blue-white), the glyph tint alone is barely a signal
          // and this word is the entire provenance axis.
          <span className="inline-flex shrink-0 items-center gap-1.5 font-heading text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground">
            <SourceGlyph source={source} className={cn("size-3.5", meta.glyph)} />
            {sourceLabel}
          </span>
        )}
      </header>

      <div className="flex-1 space-y-4 p-4">
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
      </div>
    </section>
  );
}

/** The provenance glyph: what kind of hand is on this slot. */
function SourceGlyph({ source, className }: { source: SlotSource; className?: string }) {
  const Icon =
    source === "fetchable"
      ? DownloadIcon
      : source === "link_only"
        ? ExternalLinkIcon
        : source === "derived"
          ? SlidersHorizontalIcon
          : HandIcon;
  return <Icon aria-hidden className={className} />;
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

/**
 * A slot with nothing in it: one sentence, set as a sentence.
 *
 * It used to be a bordered, rounded, padded `GlassPanel` — a container drawn
 * around a single line of prose, in a pane that already *is* a bordered
 * container. Two boxes to hold one sentence reads as a placeholder for a
 * component that has not been built yet, which is the opposite of what this
 * copy is doing: it is a finished statement of fact. Prose gets a measure
 * instead of a border.
 */
function Empty({ children }: { children: React.ReactNode }) {
  return <p className="max-w-[62ch] text-xs leading-relaxed text-muted-foreground">{children}</p>;
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

/** Bytes as the user would say them. */
function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

function ItemCard({ item, original }: { item: StackItem; original: PreservedOriginal | null }) {
  // No product name, no version, no company and no description means the PE
  // carried no version resource at all — so all Kalpa knows is the bytes.
  const identifiedByHash =
    !item.display_name && !item.version && !item.company && !item.description;

  // A flat plate inside the pane container, not a card floating on the dialog.
  // Depth is three steps and only three: dialog surface, pane container with
  // one drop shadow, flat item plates. A glass panel inside a glass panel reads
  // as no elevation at all, because both carry the same inset highlight.
  //
  // The left border is gone: severity owns that device, and a healthy file drew
  // a 3px bar that said nothing.
  return (
    <article className="rounded-lg bg-structure-03 p-4">
      <div className="mb-3 flex items-baseline justify-between gap-3">
        <h4 className="truncate font-heading text-sm font-semibold tracking-[-0.01em]">
          {ROLE_LABEL[item.role]}
        </h4>
        {/* The file name is the card's *subtitle*, not its co-title. At 12px it
            sat at the same optical weight as the heading and the pair read as
            two headings on one line. */}
        <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
          {item.file_name}
        </span>
      </div>
      {/* A label/value grid rather than a middot-joined string. Most of this
          came off the PE version resource and was being fetched and thrown
          away — `company`, `description` and `size_bytes` were read by the
          backend and rendered nowhere. */}
      <dl className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-xs">
        <dt className="text-muted-foreground">Product</dt>
        <dd className="truncate">{item.display_name ?? "no product name"}</dd>
        <dt className="text-muted-foreground">Version</dt>
        <dd className="tabular-nums">{item.version ?? "no version info"}</dd>
        {item.company && (
          <>
            <dt className="text-muted-foreground">Publisher</dt>
            <dd className="truncate">{item.company}</dd>
          </>
        )}
        {item.description && (
          <>
            <dt className="text-muted-foreground">Description</dt>
            <dd className="truncate" title={item.description}>
              {item.description}
            </dd>
          </>
        )}
        <dt className="text-muted-foreground">Size</dt>
        <dd className="tabular-nums">{formatBytes(item.size_bytes)}</dd>
        {identifiedByHash && (
          <>
            <dt className="text-muted-foreground">Identified</dt>
            <dd>by hash — this file carries no version resource</dd>
          </>
        )}
      </dl>
      {original && (
        <div className="mt-3 border-t border-structure-06 pt-3">
          <dl className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-xs">
            <dt className="text-accent-sky">Your original</dt>
            <dd className="tabular-nums">{original.version ?? "no version info"}</dd>
            <dt className="text-muted-foreground">Kept</dt>
            <dd className="tabular-nums">{formatBytes(original.size_bytes)}</dd>
          </dl>
        </div>
      )}
    </article>
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
  if (slot === "shaders") {
    return (
      <ShaderPacksPanel clientDir={stack.client_dir} stack={stack} onChanged={onStackChanged} />
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
