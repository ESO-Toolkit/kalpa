import { describe, it, expect } from "vitest";
import {
  parseChangelog,
  matchInstalledEntry,
  buildVersionDateIndex,
  dateForEntry,
  type ChangelogEntry,
} from "../changelog";
import * as fixtures from "./__fixtures__/changelogs";

/**
 * Every fixture in `__fixtures__/changelogs.ts` is a real ESOUI changelog,
 * cleaned exactly the way `src-tauri/src/esoui.rs` cleans it before the
 * frontend ever sees the string. The point of testing against real data is that
 * the format is author-freeform: no synthetic sample would have produced
 * `2.0 r42 (consoles only)` or `• Version 1.0.18 (2021/03/08)`.
 */

/** The content of a string, with whitespace-only lines and indentation dropped. */
function contentLines(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

/** Re-join a parse result back into the text it partitioned. */
function reassemble(result: ReturnType<typeof parseChangelog>): string {
  if (result.kind === "empty") return "";
  if (result.kind === "unparsed") return result.text;
  const parts: string[] = [];
  if (result.preamble) parts.push(result.preamble);
  for (const entry of result.entries) {
    parts.push(entry.header);
    if (entry.body) parts.push(entry.body);
  }
  return parts.join("\n");
}

function expectParsed(text: string) {
  const result = parseChangelog(text);
  if (result.kind !== "parsed") {
    throw new Error(`expected a parsed changelog, got "${result.kind}"`);
  }
  return result;
}

function entryAt(result: { entries: ChangelogEntry[] }, index: number): ChangelogEntry {
  const found = result.entries[index];
  if (!found) throw new Error(`expected an entry at index ${index}`);
  return found;
}

function headers(entries: ChangelogEntry[]): string[] {
  return entries.map((entry) => entry.header);
}

describe("parseChangelog — empty input", () => {
  // The Rust layer collapses ESOUI's literal "None" sentinel to "" before it
  // reaches us, so both spellings have to land on the same single signal.
  it.each([
    ["empty string", ""],
    ["whitespace only", "   \n\t\n  "],
    ["CRLF whitespace", "\r\n\r\n"],
    ["the cleaned 'None' sentinel", ""],
  ])("returns empty for %s", (_label, input) => {
    expect(parseChangelog(input)).toEqual({ kind: "empty" });
  });
});

describe("parseChangelog — AwesomeGuildStore (esoui 695)", () => {
  // `version 1.7.8:` — lowercase keyword, trailing colon, and a "Changes:"
  // sub-heading inside every body. 105 entries in the truncated fixture.
  it("splits every version header", () => {
    const result = expectParsed(fixtures.awesomeGuildStore);
    expect(result.entries).toHaveLength(105);
    expect(result.preamble).toBeNull();
  });

  it("keeps the newest entry first and the header text verbatim", () => {
    const result = expectParsed(fixtures.awesomeGuildStore);
    expect(headers(result.entries).slice(0, 3)).toEqual([
      "version 1.7.8:",
      "version 1.7.7:",
      "version 1.7.6:",
    ]);
    expect(entryAt(result, 0).body).toContain("Fixed errors in XBox Play Anywhere edition");
  });

  // "Changes:" opens nearly every body. If it were ever promoted to a header
  // the entry count would blow up, so pin it as body text explicitly.
  it("does not mistake the 'Changes:' sub-heading for a version", () => {
    const result = expectParsed(fixtures.awesomeGuildStore);
    expect(headers(result.entries)).not.toContain("Changes:");
    expect(entryAt(result, 0).body.startsWith("Changes:")).toBe(true);
  });
});

describe("parseChangelog — LibAddonMenu-2.0 (esoui 7)", () => {
  // `2.0 r43` — a dotted version plus a space-separated library revision. The
  // revision is part of the version, not trailing prose.
  it("keeps the revision attached to the version", () => {
    const result = expectParsed(fixtures.libAddonMenu);
    expect(result.entries).toHaveLength(19);
    expect(headers(result.entries).slice(0, 3)).toEqual([
      "2.0 r43",
      "2.0 r42 (consoles only)",
      "2.0 r41",
    ]);
  });

  it("allows a parenthesised note after the version", () => {
    const result = expectParsed(fixtures.libAddonMenu);
    expect(entryAt(result, 1).body).toContain(
      "temporarily turned LHAS into an optional dependency"
    );
  });

  it("treats the `- fixed ...` body lines as body, not headers", () => {
    const result = expectParsed(fixtures.libAddonMenu);
    expect(headers(result.entries).some((h) => h.startsWith("- "))).toBe(false);
  });
});

describe("parseChangelog — LibCombat (esoui 2528)", () => {
  // Bare integers as version headers: `89`, `88`, `87`. Nothing but a number
  // on the line, which is why the bare-integer rule is whole-line anchored.
  it("recognises whole-line bare integers", () => {
    const result = expectParsed(fixtures.libCombat);
    expect(result.entries).toHaveLength(22);
    expect(headers(result.entries).slice(0, 4)).toEqual(["89", "88", "87", "86"]);
  });

  it("keeps bullet body lines out of the header set", () => {
    const result = expectParsed(fixtures.libCombat);
    expect(headers(result.entries).some((h) => h.startsWith("•"))).toBe(false);
    expect(entryAt(result, 0).body).toContain("Nightblade class mastery");
  });
});

describe("parseChangelog — HarvestMap (esoui 57)", () => {
  // Markdown `## 3.16.12`, plus letter-suffixed patch versions like `3.16.8b`.
  it("strips markdown decoration when detecting, but not when storing", () => {
    const result = expectParsed(fixtures.harvestMap);
    expect(result.entries).toHaveLength(25);
    expect(headers(result.entries).slice(0, 2)).toEqual(["## 3.16.12", "## 3.16.11"]);
  });

  it("handles letter-suffixed patch versions", () => {
    const result = expectParsed(fixtures.harvestMap);
    expect(headers(result.entries)).toContain("## 3.16.8b");
    expect(headers(result.entries)).toContain("## 3.16.7b");
  });
});

describe("parseChangelog — AlignGrid (esoui 1292)", () => {
  // Date-led headers: `1/22/2024 Version 1.4.4`. The date comes first, so none
  // of the version patterns fire — the date rule is what carries these.
  it("recognises `M/D/YYYY Version X` headers", () => {
    const result = expectParsed(fixtures.alignGrid);
    expect(result.entries).toHaveLength(15);
    expect(headers(result.entries).slice(0, 3)).toEqual([
      "1/22/2024 Version 1.4.4",
      "1/21/2024 Version 1.4.3",
      "11/4/2022 Version 1.4.2",
    ]);
    expect(entryAt(result, result.entries.length - 1).header).toBe("3/7/2016 Version 1.1");
  });

  it("keeps the multi-line body of a single entry together", () => {
    const result = expectParsed(fixtures.alignGrid);
    const entry = result.entries.find((e) => e.header === "10/23/2022 Version 1.4.0");
    expect(entry?.body.split("\n")).toHaveLength(6);
  });
});

describe("parseChangelog — Keybinding: Log Out (esoui 1456), pass 2", () => {
  // This author wrapped EVERY line in `[*]`, headers included, so after the
  // Rust cleaning every single line begins with `• `. Pass 1 sees zero headers;
  // pass 2 rescues the keyword-anchored ones only.
  it("recovers bullet-wrapped version headers", () => {
    const result = expectParsed(fixtures.keybindingLogOut);
    expect(result.entries).toHaveLength(19);
    expect(headers(result.entries).slice(0, 2)).toEqual([
      "• Version 1.0.18 (2021/03/08)",
      "• Version 1.0.17 (2020/11/05)",
    ]);
  });

  it("leaves ordinary bullets in the body", () => {
    const result = expectParsed(fixtures.keybindingLogOut);
    expect(entryAt(result, 0).body).toContain(
      "• API version bump for Update 29 (Flames of Ambition)"
    );
    expect(headers(result.entries)).not.toContain(
      "• API version bump for Update 29 (Flames of Ambition)"
    );
  });

  // Pass 2 is deliberately narrow. A bullet reading "• 2 bug fixes" or
  // "• 1.5 seconds faster" must stay body text — otherwise every list item in
  // every bullet-heavy changelog becomes a fake version heading.
  it("does not promote non-keyworded bullets in pass 2", () => {
    const text = [
      "• 2 bug fixes",
      "• 1.5 improvements",
      "• Version 3.1",
      "• did a thing",
      "• Version 3.0",
      "• did another thing",
    ].join("\n");
    const result = expectParsed(text);
    expect(headers(result.entries)).toEqual(["• Version 3.1", "• Version 3.0"]);
    expect(result.preamble).toBe("• 2 bug fixes\n• 1.5 improvements");
  });

  // Pass 2 only runs when pass 1 came up short. A changelog that already parses
  // on non-bullet headers must not have bullets injected into its header set.
  it("stays on pass 1 when pass 1 already found two or more headers", () => {
    const result = expectParsed(fixtures.libCombat);
    expect(headers(result.entries).every((h) => !h.startsWith("•"))).toBe(true);
  });
});

describe("parseChangelog — preamble", () => {
  // pChat opens with a maintenance notice and a literal "Changelog:" line
  // before the first version. That is content, not noise, so it is kept.
  it("keeps a short pChat-style preamble (esoui 93)", () => {
    const result = expectParsed(fixtures.pChat);
    expect(result.entries).toHaveLength(14);
    expect(result.preamble).toContain("Maintained by Baertram");
    expect(result.preamble).toContain("Changelog:");
    expect(headers(result.entries)[0]).toBe("## v10.0.7.4 ## 2026-06-08");
  });

  it("returns null when the first line is already a header", () => {
    expect(expectParsed(fixtures.libAddonMenu).preamble).toBeNull();
  });

  // A single header buried under paragraphs of prose is far more likely to be
  // a stray number than a real changelog, so a lone header must lead.
  it("rejects a lone header that is not the first non-empty line", () => {
    const text = ["Please read the readme before installing this addon.", "", "1.0.0", "- first"]
      .join("\n")
      .trim();
    expect(parseChangelog(text).kind).toBe("unparsed");
  });

  it("accepts a lone header when it is the first non-empty line", () => {
    const result = expectParsed("\n\n1.0.0\n- first release\n- second line");
    expect(result.entries).toHaveLength(1);
    expect(entryAt(result, 0).header).toBe("1.0.0");
    expect(result.preamble).toBeNull();
  });

  // Past the budget, the "preamble" is really the whole document and the one
  // header we found was an accident.
  it("rejects a preamble that dominates the document", () => {
    const wall = "This addon has a long licensing notice. ".repeat(20);
    const text = `${wall}\n\n1.0.0\n- tiny\n\n0.9.0\n- tiny`;
    expect(wall.length).toBeGreaterThan(400);
    expect(parseChangelog(text).kind).toBe("unparsed");
  });

  it("allows a preamble under the 400 character floor even on a short changelog", () => {
    const notice = "Renamed from OldAddon. Settings do not carry over.";
    const text = `${notice}\n\n1.0.0\n- a\n\n0.9.0\n- b`;
    const result = expectParsed(text);
    expect(result.preamble).toBe(notice);
    expect(result.entries).toHaveLength(2);
  });
});

describe("parseChangelog — refuses to guess", () => {
  // esoui 1121: dated bullet entries with no version anywhere. Every line is a
  // bullet, and the dates sit inside long prose lines, so there is nothing
  // trustworthy to split on. Showing the raw text beats inventing structure.
  it("returns unparsed for the Crafting Writ Assistant changelog (esoui 1121)", () => {
    const result = parseChangelog(fixtures.craftingWritAssistant);
    expect(result.kind).toBe("unparsed");
    if (result.kind !== "unparsed") throw new Error("unreachable");
    expect(result.text).toBe(fixtures.craftingWritAssistant);
  });

  // The trailing-length guard exists for exactly this: prose that opens on
  // something version-shaped.
  it.each([
    "2.5 million lines of code were rewritten for this release, so expect bugs",
    "1.2 seconds is roughly how much faster the map now opens on a cold start",
    "3 new features were added to the map because people kept asking for them",
  ])("does not treat version-shaped prose as a header: %s", (line) => {
    const text = `${line}\n${line}\n${line}`;
    expect(parseChangelog(text).kind).toBe("unparsed");
  });

  it("returns unparsed for a wall of prose with no numbers at all", () => {
    const text = [
      "Thanks for downloading! I maintain this in my spare time.",
      "If something breaks, please open a ticket rather than a comment.",
      "Translations are always welcome, see the readme for the format.",
    ].join("\n");
    expect(parseChangelog(text).kind).toBe("unparsed");
  });

  it("preserves the normalised text on the unparsed branch", () => {
    const result = parseChangelog("just\r\nsome\rprose here");
    expect(result).toEqual({ kind: "unparsed", text: "just\nsome\nprose here" });
  });
});

describe("parseChangelog — header shapes", () => {
  const shapes: Array<[string, string]> = [
    ["dotted", "1.7.8"],
    ["dotted with v prefix", "v2.5.49"],
    ["dotted with Version prefix", "Version 1.1.8"],
    ["dotted with trailing colon", "version 1.7.8:"],
    ["dotted with revision", "2.0 r43"],
    ["dotted with letter suffix", "3.16.8b"],
    ["underscore separated", "1_4_2"],
    ["markdown heading", "## 3.16.12"],
    ["equals rule", "== 4.1.0 =="],
    ["keyword plus bare number", "Version 34"],
    ["uppercase keyword", "UPDATE 1"],
    ["compact v number", "v107"],
    ["library revision", "r47"],
    ["bare integer", "89"],
    ["bare integer with colon", "12:"],
    ["US date", "4/3/2014"],
    ["ISO date", "2026-06-08"],
    ["long-form date", "17 July 2026"],
    ["version with author suffix", "v2.5.49 ~DakJaniels (17-July-2026)"],
    ["date then version", "1/22/2024 Version 1.4.4"],
  ];

  it.each(shapes)("recognises a %s header (%s)", (_label, header) => {
    const text = `${header}\n- a change\n\n0.0.1\n- older`;
    const result = expectParsed(text);
    expect(entryAt(result, 0).header).toBe(header);
  });

  const rejects: Array<[string, string]> = [
    ["a long date-led prose line", "4/3/2014 was the day I first started writing this addon"],
    ["a sentence that happens to start with a number", "3 things changed in this release overall"],
    ["a header over 70 characters", `1.0.0 ${"x".repeat(80)}`],
    ["a mid-line version", "fixed a crash in 1.2.3 that nobody reported"],
    ["an update word without digits", "Updated for the Gold Road chapter"],
  ];

  it.each(rejects)("does not treat %s as a header (%s)", (_label, line) => {
    const text = `2.0.0\n${line}\n\n1.0.0\n- older`;
    const result = expectParsed(text);
    expect(headers(result.entries)).toEqual(["2.0.0", "1.0.0"]);
    expect(entryAt(result, 0).body).toBe(line);
  });
});

describe("parseChangelog — line-ending normalisation", () => {
  it("normalises CRLF and lone CR before parsing", () => {
    const crlf = expectParsed("1.1.0\r\n- a\r\n\r\n1.0.0\r\n- b");
    const cr = expectParsed("1.1.0\r- a\r\r1.0.0\r- b");
    const lf = expectParsed("1.1.0\n- a\n\n1.0.0\n- b");
    expect(crlf).toEqual(lf);
    expect(cr).toEqual(lf);
    expect(lf.entries.map((e) => e.body)).toEqual(["- a", "- b"]);
  });
});

describe("parseChangelog — lossless partition invariant", () => {
  /**
   * The load-bearing property: the parser partitions lines and does nothing
   * else. Every non-blank line of the input must reappear exactly once, in the
   * same order, across preamble + headers + bodies. A misclassified header is
   * then only ever a cosmetic mistake — it can never swallow a change note.
   */
  function expectLossless(input: string) {
    const result = parseChangelog(input);
    const before = contentLines(input);
    const after = contentLines(reassemble(result));
    expect(after).toEqual(before);
  }

  it.each(Object.entries(fixtures))("holds for the %s fixture", (_name, text) => {
    expectLossless(text);
  });

  // A cheap deterministic generator: shuffle a vocabulary of real header and
  // body lines into soups the parser has never seen, including ones where a
  // header lands mid-body or two headers sit back to back.
  const vocabulary = [
    "version 1.7.8:",
    "## 3.16.12",
    "2.0 r43",
    "89",
    "• Version 1.0.18 (2021/03/08)",
    "1/22/2024 Version 1.4.4",
    "r47",
    "Changes:",
    "- fixed a crash",
    "•  Improved load times by deferring initialization",
    "",
    "   ",
    "Thanks to everyone who reported this",
    "2.5 million lines of code were rewritten for this release, expect bugs",
    "v2.5.49 ~DakJaniels (17-July-2026)",
    "12:",
    "Updated for the Gold Road chapter",
  ];

  function seeded(seed: number): () => number {
    let state = seed >>> 0;
    return () => {
      state = (state * 1664525 + 1013904223) >>> 0;
      return state / 0x100000000;
    };
  }

  it("holds for 500 generated line soups", () => {
    for (let seed = 1; seed <= 500; seed++) {
      const rand = seeded(seed);
      const length = 1 + Math.floor(rand() * 25);
      const lines: string[] = [];
      for (let i = 0; i < length; i++) {
        lines.push(vocabulary[Math.floor(rand() * vocabulary.length)] ?? "");
      }
      expectLossless(lines.join("\n"));
    }
  });

  it.each([
    ["two headers back to back", "1.1.0\n1.0.0\n- only body"],
    ["a header as the only line", "1.0.0"],
    ["trailing blank lines", "1.1.0\n- a\n\n\n1.0.0\n- b\n\n\n"],
    ["leading blank lines", "\n\n\n1.1.0\n- a\n\n1.0.0\n- b"],
    ["duplicate headers", "1.0.0\n- a\n\n1.0.0\n- a"],
    ["an entry with no body", "1.1.0\n\n1.0.0\n- b"],
  ])("holds for %s", (_label, input) => {
    expectLossless(input);
  });

  // De-duplication would be the sneakiest possible loss: two releases that
  // shipped the same one-line note would silently collapse into one.
  it("keeps duplicate entries distinct", () => {
    const result = expectParsed("1.0.1\n- updated API version\n\n1.0.0\n- updated API version");
    expect(result.entries).toHaveLength(2);
    expect(entryAt(result, 0).body).toBe(entryAt(result, 1).body);
  });
});

describe("matchInstalledEntry", () => {
  it.each([
    ["awesomeGuildStore", "1.7.8", "version 1.7.8:"],
    ["libAddonMenu", "2.0 r43", "2.0 r43"],
    ["libCombat", "89", "89"],
    ["harvestMap", "3.16.12", "## 3.16.12"],
    ["alignGrid", "1.4.4", "1/22/2024 Version 1.4.4"],
    ["keybindingLogOut", "1.0.18", "• Version 1.0.18 (2021/03/08)"],
    ["pChat", "10.0.7.4", "## v10.0.7.4 ## 2026-06-08"],
  ] as const)("matches the installed version in %s", (name, version, expected) => {
    const result = expectParsed(fixtures[name]);
    const index = matchInstalledEntry(result.entries, version);
    expect(result.entries[index]?.header).toBe(expected);
  });

  it("finds an older installed version further down the list", () => {
    const result = expectParsed(fixtures.harvestMap);
    expect(matchInstalledEntry(result.entries, "3.16.8b")).toBe(
      headers(result.entries).indexOf("## 3.16.8b")
    );
  });

  it("ignores punctuation and case on both sides", () => {
    const entries: ChangelogEntry[] = [
      { header: "VERSION_2-0-R43 (final)", body: "" },
      { header: "1.0.0", body: "" },
    ];
    expect(matchInstalledEntry(entries, "2.0 r43")).toBe(0);
  });

  // A miss has to be routine, not exceptional: the version field and the
  // changelog are written by hand in two different places.
  it.each([
    ["an unlisted version", "9.9.9"],
    ["an empty string", ""],
    ["punctuation only", "..-"],
  ])("returns -1 for %s", (_label, version) => {
    const result = expectParsed(fixtures.libAddonMenu);
    expect(matchInstalledEntry(result.entries, version)).toBe(-1);
  });

  it("returns -1 when the version is undefined", () => {
    const result = expectParsed(fixtures.libAddonMenu);
    expect(matchInstalledEntry(result.entries, undefined)).toBe(-1);
  });

  it("returns -1 for an empty entry list", () => {
    expect(matchInstalledEntry([], "1.0.0")).toBe(-1);
  });

  // Documented behaviour, not an accident: the newest matching entry wins, so
  // a substring-y version like "89" resolves to the newest header containing
  // it rather than an older `189`.
  it("returns the first (newest) match", () => {
    const entries: ChangelogEntry[] = [
      { header: "89", body: "" },
      { header: "189", body: "" },
    ];
    expect(matchInstalledEntry(entries, "89")).toBe(0);
  });

  // The reason the doc comment forbids slicing: one sampled addon reported a
  // version newer than its own newest changelog entry. Hiding entries above the
  // match would have shown that user an empty changelog.
  it("misses cleanly when the reported version is ahead of the changelog", () => {
    const result = expectParsed(fixtures.libCombat);
    expect(matchInstalledEntry(result.entries, "90")).toBe(-1);
    expect(result.entries.length).toBeGreaterThan(0);
  });
});

describe("matchInstalledEntry — prefix versions", () => {
  // Regression: containment alone marked the NEWEST entry as installed whenever
  // the installed version was a numeric prefix of it. A user on 1.7 with 1.7.8
  // available saw "LATEST | INSTALLED" on the same row while the dialog header
  // said "v1.7 -> v1.7.8", and the update delta collapsed to zero.
  const entries: ChangelogEntry[] = [
    { header: "version 1.7.8:", body: "newest" },
    { header: "version 1.7.7:", body: "older" },
    { header: "version 1.7:", body: "the one actually installed" },
  ];

  it("does not mark 1.7.8 as installed when 1.7 is", () => {
    expect(matchInstalledEntry(entries, "1.7")).toBe(2);
  });

  it("still matches the exact version", () => {
    expect(matchInstalledEntry(entries, "1.7.8")).toBe(0);
  });

  it("falls back to containment for multi-token versions", () => {
    // "2.0 r43" spans two tokens, so no single token can equal the needle.
    const libEntries: ChangelogEntry[] = [
      { header: "2.0 r44", body: "" },
      { header: "2.0 r43 (consoles only)", body: "" },
    ];
    expect(matchInstalledEntry(libEntries, "2.0 r43")).toBe(1);
  });
});

describe("buildVersionDateIndex / dateForEntry", () => {
  const archived = [
    { version: "1.7.7", date: "04/23/26 01:16 PM" },
    { version: "1.7.6", date: "09/06/25 11:16 AM" },
    { version: "2.5.49", date: "01/02/25 09:00 AM" },
  ];
  const index = buildVersionDateIndex(archived);

  it("dates a header carrying author noise", () => {
    expect(dateForEntry("version 1.7.7:", index)).toBe("04/23/26 01:16 PM");
  });

  it("dates a v-prefixed header against an unprefixed archive entry", () => {
    // Without normalising the leading v, every v-prefixed changelog went undated.
    expect(dateForEntry("v2.5.49", index)).toBe("01/02/25 09:00 AM");
  });

  it("resolves a multi-version header to the first known release", () => {
    expect(dateForEntry("v104, 1.7.6", index)).toBe("09/06/25 11:16 AM");
  });

  it("never lets a prefix version borrow another release's date", () => {
    // "1.7" is a substring of "1.7.7"/"1.7.6"; a plausible-but-wrong date is
    // worse than none, so this must stay undefined.
    expect(dateForEntry("1.7", index)).toBeUndefined();
  });

  it("returns undefined when the addon archives nothing", () => {
    // The Tamriel Trade Centre case: no archived files at all, so every row
    // renders with a blank date rather than a guess.
    expect(dateForEntry("version 1.7.7:", buildVersionDateIndex([]))).toBeUndefined();
  });

  it("keeps the first date when a version is archived twice", () => {
    const dupes = buildVersionDateIndex([
      { version: "3.0", date: "first" },
      { version: "3.0", date: "second" },
    ]);
    expect(dateForEntry("3.0", dupes)).toBe("first");
  });
});
