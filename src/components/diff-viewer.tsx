import { useMemo } from "react";
import { structuredPatch } from "diff";
import { GlassPanel } from "@/components/ui/glass-panel";

interface DiffViewerProps {
  userContent: string;
  upstreamContent: string;
  fileName: string;
}

export function DiffViewer({ userContent, upstreamContent, fileName }: DiffViewerProps) {
  const hunks = useMemo(() => {
    const patch = structuredPatch(
      fileName,
      fileName,
      userContent,
      upstreamContent,
      "Your version",
      "The update",
      { context: 3 }
    );
    return patch.hunks;
  }, [userContent, upstreamContent, fileName]);

  if (hunks.length === 0) {
    return (
      <GlassPanel variant="subtle" className="p-4 text-center text-sm text-muted-foreground/60">
        Files are identical
      </GlassPanel>
    );
  }

  return (
    <GlassPanel variant="subtle" className="overflow-hidden">
      <div className="flex border-b border-white/[0.06] text-xs">
        <div className="flex-1 px-3 py-1.5 text-[#c4a44a]/80 font-medium">Your version</div>
        <div className="flex-1 px-3 py-1.5 text-sky-400/80 font-medium border-l border-white/[0.06]">
          The update
        </div>
      </div>
      <div className="max-h-[400px] overflow-auto font-mono text-xs leading-5">
        {hunks.map((hunk, hi) => (
          <div key={hi}>
            {hi > 0 && (
              <div className="border-y border-white/[0.04] bg-white/[0.01] px-3 py-0.5 text-muted-foreground/30 text-center text-[10px]">
                ···
              </div>
            )}
            {hunk.lines.map((line, li) => {
              const prefix = line[0];
              const content = line.slice(1);

              if (prefix === "-") {
                return (
                  <div key={`${hi}-${li}`} className="flex">
                    <div className="flex-1 bg-[#c4a44a]/[0.06] border-l-2 border-[#c4a44a]/30 px-3 whitespace-pre-wrap break-all">
                      <span className="text-[#c4a44a]/40 select-none mr-2">−</span>
                      <span className="text-[#c4a44a]/80">{content}</span>
                    </div>
                    <div className="flex-1 border-l border-white/[0.06]" />
                  </div>
                );
              }

              if (prefix === "+") {
                return (
                  <div key={`${hi}-${li}`} className="flex">
                    <div className="flex-1" />
                    <div className="flex-1 bg-sky-400/[0.06] border-l-2 border-sky-400/30 px-3 whitespace-pre-wrap break-all">
                      <span className="text-sky-400/40 select-none mr-2">+</span>
                      <span className="text-sky-400/80">{content}</span>
                    </div>
                  </div>
                );
              }

              return (
                <div key={`${hi}-${li}`} className="flex">
                  <div className="flex-1 px-3 text-muted-foreground/50 whitespace-pre-wrap break-all">
                    <span className="text-transparent select-none mr-2"> </span>
                    {content}
                  </div>
                  <div className="flex-1 px-3 text-muted-foreground/50 border-l border-white/[0.06] whitespace-pre-wrap break-all">
                    <span className="text-transparent select-none mr-2"> </span>
                    {content}
                  </div>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </GlassPanel>
  );
}
