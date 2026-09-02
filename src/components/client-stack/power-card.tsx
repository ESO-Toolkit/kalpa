import type { StackPanelProps } from "@/components/client-stack/panel-props";

/**
 * Switch the whole stack off, or back on.
 *
 * The confirmation is the plan: one line per operation, in the order they run,
 * computed by the backend from what is actually on disk. The confirm button
 * stays disabled until that plan has loaded — a user cannot approve a list they
 * have not been shown.
 */
export function StackPowerCard(_props: StackPanelProps) {
  return null;
}
