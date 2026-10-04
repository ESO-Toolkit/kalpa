import type { ClientStack } from "@/components/client-stack/types";

export type StackMutationResult<T> =
  { status: "committed"; value: T } | { status: "stale" } | { status: "busy" };

/**
 * The page owns installation identity and serializes every stack write. Child
 * panels must not publish mutation results independently: a successful
 * operation is committed only after the page has re-inspected the same
 * installation generation. A panel may then refresh its own derived view.
 */
export interface StackMutationCoordinator {
  pending: boolean;
  pendingLabel: string | null;
  run<T>(
    label: string,
    clientDir: string,
    operation: () => Promise<T>
  ): Promise<StackMutationResult<T>>;
}

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
  /** Page-level serialization and stale-completion protection for writes. */
  mutation: StackMutationCoordinator;
}
