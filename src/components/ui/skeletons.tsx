import { Skeleton } from "@/components/ui/skeleton";

export function PackCardSkeleton({ index = 0 }: { index?: number }) {
  const titleWidths = ["60%", "45%", "55%", "40%", "50%"];
  const descWidths = ["90%", "80%", "85%", "75%", "88%"];
  const delay = `${index * 80}ms`;
  const style = { "--shimmer-delay": delay } as React.CSSProperties;

  return (
    <div className="rounded-xl border border-white/[0.06] p-3 border-l-[3px] border-l-white/[0.08] shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1 space-y-1.5">
          <div className="flex items-center gap-2">
            <Skeleton
              className="h-3.5 rounded"
              style={{ width: titleWidths[index % titleWidths.length], ...style }}
            />
            <Skeleton className="h-4 w-14 rounded-full" style={style} />
          </div>
          <Skeleton
            className="h-2.5 rounded"
            style={{ width: descWidths[index % descWidths.length], ...style }}
          />
        </div>
        <Skeleton className="h-9 w-10 shrink-0 rounded-lg" style={style} />
      </div>
      <div className="mt-2.5 flex items-center gap-1.5">
        <Skeleton className="h-4 w-10 rounded-full" style={style} />
        <Skeleton className="h-4 w-12 rounded-full" style={style} />
        <Skeleton className="h-3 w-16 rounded ml-auto" style={style} />
      </div>
    </div>
  );
}

export function PackListSkeleton({ count = 4 }: { count?: number }) {
  return (
    <div className="space-y-2" role="status" aria-label="Loading packs">
      {Array.from({ length: count }, (_, i) => (
        <PackCardSkeleton key={i} index={i} />
      ))}
    </div>
  );
}

export function DiscoverResultRowSkeleton({ index = 0 }: { index?: number }) {
  const titleWidths = ["55%", "42%", "63%", "38%", "50%", "45%", "58%"];
  const metaWidths = ["30%", "22%", "35%", "25%", "28%", "32%", "20%"];
  const delay = `${index * 60}ms`;
  const style = { "--shimmer-delay": delay } as React.CSSProperties;

  return (
    <div className="border-l-3 border-l-transparent px-4 py-2.5">
      <div className="flex items-center gap-2.5">
        <Skeleton
          className="h-3.5 flex-1 rounded"
          style={{ maxWidth: titleWidths[index % titleWidths.length], ...style }}
        />
      </div>
      <div className="mt-1 flex items-center gap-2">
        <Skeleton
          className="h-2.5 rounded"
          style={{ width: metaWidths[index % metaWidths.length], ...style }}
        />
        <Skeleton className="h-4 w-12 rounded-full" style={style} />
      </div>
    </div>
  );
}

export function DiscoverResultListSkeleton({ count = 8 }: { count?: number }) {
  return (
    <div role="status" aria-label="Loading results">
      {Array.from({ length: count }, (_, i) => (
        <DiscoverResultRowSkeleton key={i} index={i} />
      ))}
    </div>
  );
}

export function DiscoverDetailSkeleton() {
  const style = (i: number) => ({ "--shimmer-delay": `${i * 60}ms` }) as React.CSSProperties;

  return (
    <div className="h-full p-6 space-y-5" role="status" aria-label="Loading addon details">
      {/* Header */}
      <div className="flex items-start justify-between gap-4">
        <div className="flex-1 min-w-0 space-y-2">
          {/* Title + version badge */}
          <div className="flex items-center gap-2.5">
            <Skeleton className="h-6 w-[55%] rounded" style={style(0)} />
            <Skeleton className="h-5 w-12 rounded" style={style(0)} />
          </div>
          {/* Author + category pill */}
          <div className="flex items-center gap-2">
            <Skeleton className="h-3.5 w-20 rounded" style={style(1)} />
            <Skeleton className="h-4 w-14 rounded-full" style={style(1)} />
          </div>
          {/* Compatibility subtitle */}
          <Skeleton className="h-3 w-48 rounded" style={style(2)} />
        </div>
        {/* Install button + ESOUI link */}
        <div className="flex flex-col items-end gap-1.5 shrink-0">
          <Skeleton className="h-9 w-[100px] rounded-md" style={style(0)} />
          <Skeleton className="h-3 w-20 rounded" style={style(1)} />
        </div>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-4 gap-2">
        {Array.from({ length: 4 }, (_, i) => (
          <div
            key={i}
            className="rounded-xl border border-white/[0.04] bg-white/[0.02] p-2.5 space-y-1.5"
          >
            <div className="flex items-center gap-1.5">
              <Skeleton className="size-3.5 rounded" style={style(i + 3)} />
              <Skeleton className="h-2 w-12 rounded" style={style(i + 3)} />
            </div>
            <Skeleton className="h-4 w-16 rounded" style={style(i + 3)} />
          </div>
        ))}
      </div>

      {/* Secondary metadata line (Created + MD5) */}
      <div className="flex items-center gap-2">
        <Skeleton className="h-2.5 w-36 rounded" style={style(7)} />
        <Skeleton className="h-2.5 w-20 rounded" style={style(8)} />
      </div>

      {/* Screenshots */}
      <div className="space-y-2">
        <Skeleton className="h-2.5 w-24 rounded" style={style(9)} />
        <Skeleton className="h-[200px] w-full rounded-xl" style={style(9)} />
        <div className="flex gap-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-14 w-24 shrink-0 rounded-lg" style={style(10 + i)} />
          ))}
        </div>
      </div>

      {/* Description */}
      <div className="space-y-2">
        <Skeleton className="h-2.5 w-20 rounded" style={style(14)} />
        <Skeleton className="h-3 w-full rounded" style={style(14)} />
        <Skeleton className="h-3 w-[92%] rounded" style={style(15)} />
        <Skeleton className="h-3 w-[78%] rounded" style={style(16)} />
      </div>
    </div>
  );
}

export function CharactersSkeleton() {
  const style = (i: number) => ({ "--shimmer-delay": `${i * 60}ms` }) as React.CSSProperties;

  return (
    <div
      className="space-y-4 opacity-0 animate-[skeleton-enter_150ms_ease-out_150ms_forwards]"
      role="status"
      aria-label="Loading characters"
    >
      {Array.from({ length: 2 }, (_, serverIdx) => (
        <div key={serverIdx}>
          <div className="flex items-center gap-2 mb-2">
            <Skeleton className="h-5 w-20 rounded-full" style={style(serverIdx * 4)} />
            <Skeleton className="h-3 w-16 rounded" style={style(serverIdx * 4)} />
          </div>
          <div className="space-y-1">
            {Array.from({ length: 3 }, (_, charIdx) => (
              <div
                key={charIdx}
                className="flex items-center justify-between rounded-xl border border-white/[0.06] bg-white/[0.02] p-3"
              >
                <Skeleton
                  className="h-3.5 rounded"
                  style={{
                    width: `${70 + charIdx * 15}px`,
                    ...style(serverIdx * 4 + charIdx + 1),
                  }}
                />
                <Skeleton
                  className="h-8 w-28 rounded-md"
                  style={style(serverIdx * 4 + charIdx + 1)}
                />
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

export function PackDetailSkeleton() {
  const style = (i: number) => ({ "--shimmer-delay": `${i * 60}ms` }) as React.CSSProperties;

  return (
    <div className="flex flex-col gap-3" role="status" aria-label="Loading pack details">
      <Skeleton className="h-3 w-[80%] rounded" style={style(0)} />
      <div className="flex items-center gap-2 flex-wrap">
        <Skeleton className="h-5 w-20 rounded-full" style={style(1)} />
        <Skeleton className="h-5 w-14 rounded-full" style={style(2)} />
        <Skeleton className="h-5 w-16 rounded-full" style={style(3)} />
        <Skeleton className="h-3 w-24 rounded ml-auto" style={style(4)} />
      </div>
    </div>
  );
}

export function SearchResultRowSkeleton({ index = 0 }: { index?: number }) {
  const titleWidths = ["55%", "42%", "60%", "48%", "53%", "38%"];
  const metaWidths = ["40%", "35%", "45%", "30%", "42%", "38%"];
  const delay = `${index * 60}ms`;
  const style = { "--shimmer-delay": delay } as React.CSSProperties;

  return (
    <div className="rounded-lg p-2 border border-transparent">
      <div className="flex items-center gap-2">
        <Skeleton className="size-3.5 shrink-0 rounded" style={style} />
        <Skeleton
          className="h-3.5 rounded flex-1"
          style={{ maxWidth: titleWidths[index % titleWidths.length], ...style }}
        />
        <Skeleton className="h-3 w-10 shrink-0 rounded" style={style} />
      </div>
      <div className="mt-0.5 ml-5">
        <Skeleton
          className="h-2.5 rounded"
          style={{ width: metaWidths[index % metaWidths.length], ...style }}
        />
      </div>
    </div>
  );
}

export function SearchResultListSkeleton({ count = 5 }: { count?: number }) {
  return (
    <div className="space-y-1" role="status" aria-label="Loading search results">
      {Array.from({ length: count }, (_, i) => (
        <SearchResultRowSkeleton key={i} index={i} />
      ))}
    </div>
  );
}

export function RosterPackSkeleton() {
  const style = (i: number) => ({ "--shimmer-delay": `${i * 60}ms` }) as React.CSSProperties;

  return (
    <div className="flex flex-col gap-3" role="status" aria-label="Loading pack">
      <Skeleton className="h-3 w-20 rounded" style={style(0)} />
      <div className="flex flex-col gap-1">
        {Array.from({ length: 5 }, (_, i) => (
          <div
            key={i}
            className="flex items-center gap-3 rounded-lg border border-white/[0.04] bg-white/[0.02] px-3 py-2"
          >
            <Skeleton className="size-4 shrink-0 rounded-[3px]" style={style(i + 1)} />
            <Skeleton
              className="h-3.5 rounded flex-1"
              style={{ maxWidth: `${120 + i * 20}px`, ...style(i + 1) }}
            />
            <Skeleton className="h-4 w-14 shrink-0 rounded-full ml-auto" style={style(i + 1)} />
          </div>
        ))}
      </div>
    </div>
  );
}

export function SavedVariablesSkeleton() {
  const style = (i: number) => ({ "--shimmer-delay": `${i * 60}ms` }) as React.CSSProperties;

  return (
    <div
      className="flex gap-0 h-full opacity-0 animate-[skeleton-enter_150ms_ease-out_150ms_forwards]"
      role="status"
      aria-label="Loading saved variables"
    >
      {/* Left panel skeleton */}
      <div className="flex w-[200px] shrink-0 flex-col border-r border-white/[0.06]">
        <div className="p-2">
          <Skeleton className="h-6 w-full rounded-lg" style={style(0)} />
        </div>
        <div className="flex-1 px-1 pb-1 space-y-1">
          {Array.from({ length: 8 }, (_, i) => (
            <Skeleton
              key={i}
              className="h-5 rounded"
              style={{
                width: `${60 + (i % 3) * 25}%`,
                marginLeft: `${(i % 3) * 8}px`,
                ...style(i + 1),
              }}
            />
          ))}
        </div>
        <div className="flex gap-1 border-t border-white/[0.06] bg-white/[0.02] p-1.5">
          <Skeleton className="h-5 flex-1 rounded-md" style={style(9)} />
          <Skeleton className="h-5 flex-1 rounded-md" style={style(9)} />
        </div>
      </div>
      {/* Right panel skeleton */}
      <div className="flex-1 p-3 space-y-3">
        <Skeleton className="h-3 w-32 rounded" style={style(10)} />
        <Skeleton className="h-8 w-full rounded-lg" style={style(11)} />
        <Skeleton className="h-3 w-48 rounded" style={style(12)} />
        <Skeleton className="h-8 w-full rounded-lg" style={style(13)} />
      </div>
    </div>
  );
}
