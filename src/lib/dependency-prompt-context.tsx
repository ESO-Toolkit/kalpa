import { getStrictContext } from "@/lib/get-strict-context";
import type { PendingDependency } from "@/types";

/**
 * Hands the dependencies an install/update deferred (because `dependencyPolicy`
 * is "ask") to the app's single picker dialog.
 *
 * Modelled on {@link EnsureEsoNotBlocking}: every flow that writes to the AddOns
 * folder needs the same modal, so App.tsx owns one dialog and one implementation
 * instead of each install surface growing its own.
 *
 * The returned promise resolves once the prompt has been RAISED (or skipped
 * because there was nothing left to ask about) — not once the user has decided.
 * The decision, the install and the list refresh are handled by the owner, so a
 * caller can fire this and let its own busy state clear. It never rejects:
 * failures surface as toasts, the same channel the install code already uses.
 */
export type ResolvePendingDeps = (
  pending: PendingDependency[],
  /**
   * The AddOns folder the caller passed to the install/update command that
   * produced `pending`. Required, and deliberately not inferred from whatever
   * folder is selected when this resolves: a command started for one game
   * instance can finish after the user has switched to another, and reading the
   * live path there would attribute one instance's missing libraries to a
   * different instance's folder.
   */
  addonsPath: string
) => Promise<void>;

const [DependencyPromptProvider, useResolvePendingDeps] = getStrictContext<ResolvePendingDeps>(
  "DependencyPromptProvider"
);

export { DependencyPromptProvider, useResolvePendingDeps };
