interface RichDescriptionProps {
  text: string;
  className?: string;
}

function classifyLine(line: string): "empty" | "bullet" | "text" {
  const trimmed = line.trim();
  if (!trimmed) return "empty";
  if (/^[-*•]\s+/.test(trimmed) || /^\d+[.)]\s+/.test(trimmed)) return "bullet";
  return "text";
}

export function RichDescription({ text, className }: RichDescriptionProps) {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Array<{ type: "p" | "ul"; items: string[] }> = [];

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

  return (
    <div
      className={[
        "rounded-xl border border-white/[0.06] bg-gradient-to-b from-white/[0.03] to-white/[0.01] p-4 text-sm text-foreground/90 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className="space-y-3 leading-relaxed">
        {blocks.length > 0 ? (
          blocks.map((block, idx) =>
            block.type === "p" ? (
              <p key={idx} className="text-foreground/85">
                {block.items[0]}
              </p>
            ) : (
              <ul key={idx} className="list-disc space-y-1 pl-5 marker:text-[#c4a44a]">
                {block.items.map((item, itemIdx) => (
                  <li key={`${idx}-${itemIdx}`} className="text-foreground/85">
                    {item}
                  </li>
                ))}
              </ul>
            )
          )
        ) : (
          <p className="text-foreground/70">No description available.</p>
        )}
      </div>
    </div>
  );
}
