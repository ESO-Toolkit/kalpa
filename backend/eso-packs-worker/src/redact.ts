import type { Pack } from "./types";

export const ANONYMOUS_AUTHOR_NAME = "Anonymous";

/**
 * Enforce anonymity server-side: strip the real author identity from an
 * anonymous pack in every public response path. Anonymization used to happen
 * only in the Rust client (and only for author_name), so anyone calling the
 * API directly could read the real author of an "anonymous" pack.
 *
 * `viewerId` is the validated bearer identity, when known — the author still
 * sees their own real fields (their "my packs" management flows need them).
 * Callers serving cacheable responses must NOT pass a viewerId, so an
 * author's unredacted copy can never be cached and served to someone else.
 */
export function redactAnonymousPack(pack: Pack, viewerId?: string): Pack {
  if (!pack.is_anonymous || (viewerId !== undefined && pack.author_id === viewerId)) {
    return pack;
  }
  return { ...pack, author_name: ANONYMOUS_AUTHOR_NAME, author_id: "" };
}
