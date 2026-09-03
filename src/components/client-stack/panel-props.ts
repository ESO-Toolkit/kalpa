import type { ClientStack } from "@/components/client-stack/types";

/**
 * What every management panel under this folder needs.
 *
 * Each panel fetches its own data and owns its own state rather than being fed
 * through the master-detail prop object: these are independent actions on one
 * folder, and threading four more feature's worth of loading/error/confirm
 * state through `StackBodyProps` would make one already-large interface the
 * place every future feature has to touch.
 */
export interface StackPanelProps {
  /** The client folder these actions apply to. */
  clientDir: string;
  /** The inventory the surrounding panel was rendered from. */
  stack: ClientStack;
  /**
   * Re-run the surrounding panel's inspection. Call after any successful write:
   * every one of these actions changes what `inspect_client_stack` would say.
   */
  onChanged: () => void | Promise<void>;
}
