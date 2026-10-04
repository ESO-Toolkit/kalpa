import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { FINDING_IMPACT, FINDING_SLOT, SLOT_ORDER } from "../slots";

/**
 * Guards the finding -> slot contract across the Rust/TypeScript boundary.
 *
 * A finding renders only if some lookup table claims it. Nothing on either side
 * of the boundary enforces that: `build_findings` emits opaque string ids, the
 * panel looks them up in a `Record<string, Slot>`, and a miss is silently
 * `undefined` — so an unmapped finding does not throw, does not warn, and does
 * not appear anywhere on screen.
 *
 * That is not hypothetical. `stack-mv-provider-missing` shipped missing from
 * both of the old lookup tables, so "nothing is producing motion vectors" —
 * which means DLSS is upscaling a still image — rendered in neither the finding
 * list nor the layer rail. It was found by reading the source, not by any test.
 *
 * So this reads `client_stack.rs` as text and asserts every id it can emit has
 * both a slot and an impact line. The Rust file is the authority; adding a
 * finding there and nothing here is the failure being caught.
 */

const CLIENT_STACK_RS = join(process.cwd(), "src-tauri", "src", "client_stack.rs");

/**
 * `stack-disabled` is the whole-stack power state, not a per-slot problem.
 *
 * The status strip states it with the switch-back-on action attached, which is
 * strictly more useful than a finding describing it — and `build_findings`
 * returns early when it fires, so it is never accompanied by others. It is
 * excluded here on purpose rather than by omission; see `FINDING_SLOT`.
 */
const NOT_A_SLOT_FINDING = new Set(["stack-disabled"]);

/**
 * Ids passed to `finding(...)` in `build_findings`.
 *
 * Matched against the `finding(` call rather than every `"stack-…"` string in
 * the file, because the test module below asserts on those same ids by name and
 * would otherwise be scraped as if it were a source of truth.
 */
function emittedFindingIds(source: string): string[] {
  const ids = new Set<string>();
  const pattern = /\bfinding\(\s*"(stack-[a-z0-9-]+)"/g;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(source)) !== null) {
    const id = match[1];
    if (id) ids.add(id);
  }
  return [...ids].sort();
}

describe("finding to slot mapping", () => {
  const source = readFileSync(CLIENT_STACK_RS, "utf8");
  const emitted = emittedFindingIds(source);

  it("finds the findings in the Rust source", () => {
    // Guards the scraper itself: a refactor that renames the `finding` helper
    // would otherwise make every assertion below vacuously pass.
    expect(emitted.length).toBeGreaterThanOrEqual(10);
    expect(emitted).toContain("stack-mv-provider-missing");
  });

  it.each(emitted.filter((id) => !NOT_A_SLOT_FINDING.has(id)))(
    "%s has a slot to render in",
    (id) => {
      expect(SLOT_ORDER).toContain(FINDING_SLOT[id]);
    }
  );

  it.each(emitted.filter((id) => !NOT_A_SLOT_FINDING.has(id)))(
    "%s says what the user will notice",
    (id) => {
      expect(FINDING_IMPACT[id]?.trim()).toBeTruthy();
    }
  );

  it("maps nothing the backend cannot emit", () => {
    // The other direction: a stale entry here is a finding id that was renamed
    // or removed in Rust, and it would sit in the table looking maintained.
    const known = new Set(emitted);
    for (const id of Object.keys(FINDING_SLOT)) expect(known).toContain(id);
    for (const id of Object.keys(FINDING_IMPACT)) expect(known).toContain(id);
  });
});
