import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { GlassPanel } from "@/components/ui/glass-panel";
import { Kbd } from "@/components/ui/kbd";
import { SectionHeader } from "@/components/ui/section-header";
import { modKeyLabel, modKeySpokenLabel } from "@/lib/platform";

type ShortcutSeparator = "chord" | "or";

interface ShortcutItem {
  action: string;
  keys: string[];
  separator?: ShortcutSeparator;
  spoken: string;
}

interface ShortcutGroup {
  id: string;
  title: string;
  items: ShortcutItem[];
}

function shortcutGroups(): ShortcutGroup[] {
  const modifier = modKeyLabel();
  const spokenModifier = modKeySpokenLabel();

  return [
    {
      id: "text-size",
      title: "Text size",
      items: [
        {
          action: "Bigger text",
          keys: [modifier, "+"],
          spoken: `${spokenModifier} plus Plus`,
        },
        {
          action: "Smaller text",
          keys: [modifier, "\u2212"],
          spoken: `${spokenModifier} plus Minus`,
        },
        {
          action: "Reset text size to 100%",
          keys: [modifier, "0"],
          spoken: `${spokenModifier} plus 0`,
        },
      ],
    },
    {
      id: "getting-around",
      title: "Getting around",
      items: [
        {
          action: "Check for addon updates",
          keys: [modifier, "R"],
          spoken: `${spokenModifier} plus R`,
        },
        {
          action: "Browse ESOUI",
          keys: [modifier, "B"],
          spoken: `${spokenModifier} plus B`,
        },
        {
          action: "Install from a link",
          keys: [modifier, "I"],
          spoken: `${spokenModifier} plus I`,
        },
        {
          action: "Leave a text box, close Discover, or clear the selection",
          keys: ["Esc"],
          spoken: "Escape",
        },
        {
          action: "Show this list",
          keys: ["?"],
          spoken: "Question mark",
        },
      ],
    },
    {
      id: "addon-list",
      title: "Addon list",
      items: [
        {
          action: "Move through the list",
          keys: ["\u2191", "\u2193"],
          separator: "or",
          spoken: "Up arrow or Down arrow",
        },
        {
          action: "Jump to the first or last addon",
          keys: ["Home", "End"],
          separator: "or",
          spoken: "Home or End",
        },
        {
          action: "Open the highlighted addon",
          keys: ["Enter", "Space"],
          separator: "or",
          spoken: "Enter or Space",
        },
        {
          action: "Add an addon to the selection",
          keys: [modifier, "click"],
          spoken: `${spokenModifier} plus click`,
        },
      ],
    },
    {
      id: "discover",
      title: "Discover",
      items: [
        {
          action: "Move through search results while typing",
          keys: ["\u2191", "\u2193"],
          separator: "or",
          spoken: "Up arrow or Down arrow",
        },
        {
          action: "Search",
          keys: ["Enter"],
          spoken: "Enter",
        },
        {
          action: "Previous or next screenshot",
          keys: ["\u2190", "\u2192"],
          separator: "or",
          spoken: "Left arrow or Right arrow",
        },
      ],
    },
  ];
}

function KeyCombo({
  keys,
  separator = "chord",
}: {
  keys: string[];
  separator?: ShortcutSeparator;
}) {
  return (
    <span aria-hidden className="inline-flex items-center gap-1">
      {keys.map((key, index) => (
        <span key={`${key}-${index}`} className="inline-flex items-center gap-1">
          {index > 0 && separator === "or" && (
            <span className="px-0.5 text-xs text-foreground">or</span>
          )}
          <Kbd>{key}</Kbd>
        </span>
      ))}
    </span>
  );
}

export function KeyboardShortcuts({ onClose }: { onClose: () => void }) {
  const groups = shortcutGroups();

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex max-h-[85vh] flex-col sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Keyboard Shortcuts</DialogTitle>
          <DialogDescription className="text-foreground">
            Press <Kbd>?</Kbd> at any time to open this list.
          </DialogDescription>
        </DialogHeader>

        <div
          tabIndex={0}
          role="group"
          aria-label="Keyboard shortcuts"
          className="-mx-1 min-h-0 flex-1 space-y-4 overflow-y-auto px-1 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-sky"
        >
          {groups.map((group) => (
            <section key={group.id} aria-labelledby={`shortcut-group-${group.id}`}>
              <SectionHeader id={`shortcut-group-${group.id}`} className="mb-2">
                {group.title}
              </SectionHeader>
              <GlassPanel variant="subtle">
                <dl className="divide-y divide-structure-06">
                  {group.items.map((item) => (
                    <div
                      key={item.action}
                      className="flex items-baseline justify-between gap-4 px-3 py-2"
                    >
                      <dt className="text-[13px] leading-snug text-foreground">{item.action}</dt>
                      <dd className="shrink-0" aria-label={item.spoken}>
                        <KeyCombo keys={item.keys} separator={item.separator} />
                      </dd>
                    </div>
                  ))}
                </dl>
              </GlassPanel>
            </section>
          ))}
        </div>

        <DialogFooter showCloseButton />
      </DialogContent>
    </Dialog>
  );
}
