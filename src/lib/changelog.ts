/**
 * ESOUI changelog parsing.
 *
 * The Rust backend (`esoui.rs::clean_change_log`) hands us `detail.changeLog`
 * as plain text: BBCode stripped, `[*]` markers rewritten to `• `, and the
 * literal sentinel `"None"` collapsed to `""`. Entries arrive newest-first and
 * the format is entirely author-freeform — ESOUI imposes no structure at all.
 *
 * A survey of 65 live addons found that BBCode is *not* the delimiter (only 22%
 * of raw changelogs contain `[B]` at all) — **line structure** is, and line
 * structure survives the Rust cleaning intact. So the split happens here, in
 * TypeScript, against the already-cleaned text.
 *
 * ## Core invariant: lossless partition
 *
 * This parser only *partitions lines*. Every content line of the input appears
 * in exactly one output slot — the preamble, an entry's header, or an entry's
 * body — in the original order, with no rewriting, dropping, reordering or
 * de-duplication. The only transformation applied is trimming surrounding
 * whitespace.
 *
 * That is deliberate: header detection on freeform author text will sometimes
 * be wrong, and the invariant guarantees a wrong call is a *cosmetic* error (a
 * line rendered as a heading instead of body text, or vice versa) rather than
 * data loss. Headers keep their author decoration (`## 3.16.12`,
 * `version 1.7.8:`) because stripping it would be a rewrite; callers are free
 * to prettify at render time.
 */

export interface ChangelogEntry {
  /** The header line exactly as the author wrote it, minus surrounding whitespace. */
  header: string;
  /** Everything between this header and the next one. May be empty. */
  body: string;
}

export type ParsedChangelog =
  | { kind: "parsed"; preamble: string | null; entries: ChangelogEntry[] }
  | { kind: "unparsed"; text: string }
  | { kind: "empty" };

/**
 * Leading author decoration: markdown hashes, rules, emphasis, quote markers,
 * the bullet the Rust layer substitutes for `[*]`, and every flavour of dash.
 */
const DECORATION = /^[\s#=*_~>•\-–—]+/;

/** A line the Rust cleaner produced from a `[*]` list marker. */
const BULLET_LINE = /^\s*•/;

/**
 * Dotted version, with an optional `v`/`Version` prefix and an optional
 * revision suffix: `1.7.8`, `v2.5.49`, `Version 1.1.8`, `2.0 r43`, `3.16.8b`.
 */
const P_DOTTED = /^(v(?:ersion)?[.:]?\s*)?\d+(?:[._]\d+)+[a-z]?(?:[-\s]?r?\d+)*/i;

/** Keyword plus a bare number: `Version 34`, `UPDATE 1`, `v107`. */
const P_KEYWORD_BARE = /^(?:update|version|v)\s*\d+[a-z]?\b/i;

/** Standalone library revision: `r47`. */
const P_REVISION = /^r\d+$/i;

/** A whole line that is nothing but an integer, optionally colon-terminated: `89`, `12:`. */
const P_BARE_INT = /^\d+\s*:?\s*$/;

/** Date-led headers. Only consulted for short lines — see `matchHeader`. */
const P_DATES = [
  /^\d{1,2}[/.-]\d{1,2}[/.-]\d{2,4}\b/, // 4/3/2014, 1-22-24
  /^\d{4}-\d{2}-\d{2}\b/, // 2026-06-08
  /^\d{1,2}\s+\w{3,9}\s+\d{4}\b/, // 17 July 2026
];

/**
 * Prose like "2.5 million lines of code were rewritten this release" opens with
 * something that looks like a version. The tail is what tells them apart: a
 * real header is followed by little or nothing, or by an obvious separator.
 */
const MAX_HEADER_LEN = 70;
const MAX_DATE_HEADER_LEN = 50;
const MAX_TRAILING_LEN = 45;
const TRAILING_SEPARATORS = [":", "-", "("];

/** Preamble allowance before we stop believing we found a changelog at all. */
const PREAMBLE_ABSOLUTE_MAX = 400;
const PREAMBLE_RATIO_MAX = 0.2;

interface HeaderMatch {
  /** The matched version token, used to measure what trails it. */
  token: string;
  /** True when the match was anchored on an explicit `v`/`Version`/`Update` keyword. */
  keyworded: boolean;
}

function matchVersionToken(stripped: string): HeaderMatch | null {
  const dotted = P_DOTTED.exec(stripped);
  if (dotted) return { token: dotted[0], keyworded: Boolean(dotted[1]) };

  const keyword = P_KEYWORD_BARE.exec(stripped);
  if (keyword) return { token: keyword[0], keyworded: true };

  const revision = P_REVISION.exec(stripped);
  if (revision) return { token: revision[0], keyworded: false };

  const bareInt = P_BARE_INT.exec(stripped);
  if (bareInt) return { token: bareInt[0], keyworded: false };

  if (stripped.length <= MAX_DATE_HEADER_LEN) {
    for (const pattern of P_DATES) {
      const date = pattern.exec(stripped);
      if (date) return { token: date[0], keyworded: false };
    }
  }

  return null;
}

/**
 * Decide whether `line` reads as a version header. Returns the match so callers
 * can inspect `keyworded`; `null` means "body text".
 */
function matchHeader(line: string): HeaderMatch | null {
  const stripped = line.trimEnd().replace(DECORATION, "");
  if (!stripped || stripped.length > MAX_HEADER_LEN) return null;

  const match = matchVersionToken(stripped);
  if (!match) return null;

  const trailing = stripped.slice(match.token.length).trim();
  if (trailing.length > MAX_TRAILING_LEN && !TRAILING_SEPARATORS.includes(trailing.charAt(0))) {
    return null;
  }
  return match;
}

/**
 * Collect header line indices.
 *
 * Pass 1 (`allowBullets: false`) ignores `•`-prefixed lines entirely. Pass 2
 * lets them back in, but only when the match was keyword-anchored — some
 * authors wrap *every* line including their headers in `[*]`, and this recovers
 * those without promoting an ordinary bullet like "• 2 bug fixes" to a heading.
 */
function findHeaders(lines: string[], allowBullets: boolean): number[] {
  const found: number[] = [];
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? "";
    if (!line.trim()) continue;
    const isBullet = BULLET_LINE.test(line);
    if (isBullet && !allowBullets) continue;
    const match = matchHeader(line);
    if (!match) continue;
    if (isBullet && !match.keyworded) continue;
    found.push(i);
  }
  return found;
}

function firstNonEmptyIndex(lines: string[]): number {
  return lines.findIndex((line) => line.trim().length > 0);
}

export function parseChangelog(text: string): ParsedChangelog {
  const normalized = text.replace(/\r\n?/g, "\n");
  if (!normalized.trim()) return { kind: "empty" };

  const lines = normalized.split("\n");

  let headers = findHeaders(lines, false);
  if (headers.length < 2) {
    const pass2 = findHeaders(lines, true);
    if (pass2.length > headers.length) headers = pass2;
  }

  if (headers.length === 0) return { kind: "unparsed", text: normalized };
  if (headers.length === 1 && headers[0] !== firstNonEmptyIndex(lines)) {
    return { kind: "unparsed", text: normalized };
  }

  const preambleText = lines.slice(0, headers[0]).join("\n").trim();
  const preambleBudget = Math.max(PREAMBLE_ABSOLUTE_MAX, normalized.length * PREAMBLE_RATIO_MAX);
  if (preambleText.length >= preambleBudget) return { kind: "unparsed", text: normalized };

  const entries: ChangelogEntry[] = headers.map((start, n) => {
    const end = n + 1 < headers.length ? headers[n + 1] : lines.length;
    return {
      header: (lines[start] ?? "").trim(),
      body: lines
        .slice(start + 1, end)
        .join("\n")
        .trim(),
    };
  });

  return { kind: "parsed", preamble: preambleText || null, entries };
}

/**
 * Reduce a version or header to comparable digits and letters.
 *
 * A leading `v` that introduces digits is dropped so an author's `v2.5.49`
 * compares equal to the archive table's `2.5.49` — without this, every
 * `v`-prefixed changelog goes undated. The word `version` is unaffected: it
 * normalises to `version…`, whose second character is not a digit.
 */
function normalizeVersion(value: string): string {
  const compact = value.toLowerCase().replace(/[^0-9a-z]/g, "");
  return /^v\d/.test(compact) ? compact.slice(1) : compact;
}

/**
 * The normalised version-ish tokens in a header. Headers carry author noise
 * ("version 1.7.8:") and can name several releases ("v104, v105"), so callers
 * compare against tokens rather than the header as a whole.
 */
function versionTokens(header: string): string[] {
  return header
    .split(/[\s,;/]+/)
    .map(normalizeVersion)
    .filter((token) => token.length > 0);
}

function hasMultipleVersionTokens(value: string): boolean {
  return value.split(/[\s,;/]+/).filter((token) => normalizeVersion(token).length > 0).length > 1;
}

/**
 * Find the entry that corresponds to the installed `version`, for decoration
 * only — a "you have this one" marker next to a single row.
 *
 * Matching is a *containment* test on normalised text, because headers carry
 * author noise the version field does not (`version 1.7.8:`, `2.0 r43
 * (consoles only)`, `v2.5.49 ~DakJaniels`). It hits roughly 84% of the time,
 * and a miss must be completely unremarkable in the UI.
 *
 * IMPORTANT: callers must NEVER hide, slice or truncate entries based on this
 * index. In the sampled set at least one addon's reported version was *ahead*
 * of its own newest changelog entry, so "show everything from the match down"
 * would have rendered an empty changelog for an addon that plainly has one.
 * Show every entry, always; use this only to highlight.
 *
 * @returns the index of the first matching entry, or -1.
 */
export function matchInstalledEntry(
  entries: ChangelogEntry[],
  version: string | undefined
): number {
  if (!version) return -1;
  const needle = normalizeVersion(version);
  if (!needle) return -1;

  // Exact token match first. Containment alone marks the user as running the
  // NEWEST entry whenever their version is a numeric prefix of it (1.7 inside
  // 1.7.8, 2.5 inside 2.5.1 — both common), which contradicts the dialog's own
  // "v1.7 -> v1.7.8" header and collapses the update delta to zero.
  const exact = entries.findIndex((entry) => versionTokens(entry.header).includes(needle));
  if (exact !== -1) return exact;

  // Fallback only for versions that genuinely span several tokens ("2.0 r43"),
  // where no single token can equal the needle. A one-token version such as
  // "1.7" must not match a newer "1.7.8" merely because it is a prefix.
  if (!hasMultipleVersionTokens(version)) return -1;
  return entries.findIndex((entry) => normalizeVersion(entry.header).includes(needle));
}

/**
 * Index archived release dates by normalised version, for annotating entries.
 *
 * Matching is exact on the normalised token, never a substring: `1.7` appearing
 * inside `1.7.8` would otherwise stamp an entry with a different release's date,
 * and a plausible-but-wrong date is worse than none.
 */
export function buildVersionDateIndex(
  archived: Array<{ version: string; date: string }>
): Map<string, string> {
  const index = new Map<string, string>();
  for (const entry of archived) {
    const key = normalizeVersion(entry.version);
    if (key && !index.has(key)) index.set(key, entry.date);
  }
  return index;
}

/**
 * The release date for a changelog entry, or undefined when unknown.
 *
 * A header can name several releases ("v104, v105", "v100, 101, 103"), so each
 * whitespace/comma-separated token is tried and the first known one wins —
 * that is the release the entry's notes actually shipped in.
 */
export function dateForEntry(header: string, index: Map<string, string>): string | undefined {
  const direct = index.get(normalizeVersion(header));
  if (direct) return direct;

  for (const key of versionTokens(header)) {
    const hit = index.get(key);
    if (hit) return hit;
  }
  return undefined;
}
