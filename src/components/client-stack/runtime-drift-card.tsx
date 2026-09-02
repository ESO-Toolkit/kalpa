import type { StackPanelProps } from "@/components/client-stack/panel-props";

/**
 * Whether a game update has put ESO's own runtimes back over the user's swap,
 * and — only when a copy was kept — the action that undoes it.
 */
export function RuntimeDriftCard(
  _props: StackPanelProps & {
    /** File names shown on the surrounding stage, so the card reports only those. */
    filePaths: string[];
  }
) {
  return null;
}
