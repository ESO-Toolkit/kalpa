import { cn } from "@/lib/utils";

interface RichTextProps {
  text: string;
  className?: string;
}

interface RichDescriptionProps {
  text: string;
  className?: string;
}

type Block = { type: "p" | "ul"; items: string[] };

function classifyLine(line: string): "empty" | "bullet" | "text" {
  const trimmed = line.trim();
  if (!trimmed) return "empty";
  if (/^[-*•]\s+/.test(trimmed) || /^\d+[.)]\s+/.test(trimmed)) return "bullet";
  return "text";
}

function toBlocks(text: string): Block[] {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];

  const flushParagraph = (buffer: string[]) => {
    if (!buffer.length) return;
    blocks.push({
      type: "p",
      items: [buffer.join(" ")],
    });
    buffer.length = 0;
  };

  const flushList = (buffer: string[]) => {
    if (!buffer.length) return;
    blocks.push({ type: "ul", items: [...buffer] });
    buffer.length = 0;
  };

  const paragraphBuffer: string[] = [];
  const listBuffer: string[] = [];

  for (const line of lines) {
    const trimmed = line.trim();
    const type = classifyLine(line);

    if (type === "empty") {
      flushParagraph(paragraphBuffer);
      flushList(listBuffer);
      continue;
    }

    if (type === "bullet") {
      flushParagraph(paragraphBuffer);
      listBuffer.push(trimmed.replace(/^([-*•]|\d+[.)])\s+/, ""));
      continue;
    }

    if (listBuffer.length) {
      flushList(listBuffer);
    }

    paragraphBuffer.push(trimmed);
  }

  flushParagraph(paragraphBuffer);
  flushList(listBuffer);

  return blocks;
}

/**
 * Renders plain text as paragraph and bullet-list blocks, with no panel chrome.
 *
 * Use this when the caller already owns a surrounding surface (for example a
 * changelog that stacks many entry bodies inside one shared panel). For a
 * standalone description panel use `RichDescription`, which wraps this.
 *
 * The empty state lives here rather than in `RichDescription` so both entry
 * points degrade identically; `RichDescription` renders exactly what it did
 * before this was extracted.
 */
export function RichText({ text, className }: RichTextProps) {
  const blocks = toBlocks(text);

  return (
    <div className={cn("space-y-3 leading-relaxed", className)}>
      {blocks.length > 0 ? (
        blocks.map((block, idx) =>
          block.type === "p" ? (
            <p key={idx} className="text-foreground break-words wrap-anywhere">
              {block.items[0]}
            </p>
          ) : (
            <ul key={idx} className="list-disc space-y-1 pl-5 marker:text-primary">
              {block.items.map((item, itemIdx) => (
                <li key={`${idx}-${itemIdx}`} className="text-foreground break-words wrap-anywhere">
                  {item}
                </li>
              ))}
            </ul>
          )
        )
      ) : (
        <p className="text-muted-foreground">No description available.</p>
      )}
    </div>
  );
}

/** Plain text rendered as paragraphs and bullet lists inside a glass panel. */
export function RichDescription({ text, className }: RichDescriptionProps) {
  return (
    <div
      className={[
        "rounded-xl border border-structure-06 bg-gradient-to-b from-structure-03 to-structure-01 p-4 text-sm text-foreground shadow-[inset_0_1px_0_var(--structure-04)]",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <RichText text={text} />
    </div>
  );
}
