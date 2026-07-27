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
export type ResolvePendingDeps = (pending: PendingDependency[]) => Promise<void>;

const [DependencyPromptProvider, useResolvePendingDeps] = getStrictContext<ResolvePendingDeps>(
  "DependencyPromptProvider"
);

export { DependencyPromptProvider, useResolvePendingDeps };
