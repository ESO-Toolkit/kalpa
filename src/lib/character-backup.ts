// Helpers for the per-character backup UI.

/** Server bucket for characters whose megaserver can't be determined. Must match
 * `UNKNOWN_SERVER` in the Rust `list_characters`. */
export const UNKNOWN_SERVER = "Unknown";

/**
 * Short, filesystem-friendly tag for a megaserver (`"NA Megaserver"` -> `"NA"`),
 * or `null` when the server is unknown / not a recognized megaserver.
 */
export function serverTag(server: string): string | null {
  if (!server || server === UNKNOWN_SERVER) return null;
  // Strip the " Megaserver" suffix; keep the rest (e.g. "PTS") verbatim.
  return server.replace(/\s+Megaserver$/i, "").trim() || null;
}

/**
 * Default backup name for a character. Includes the server tag for known
 * megaservers so a same-name NA/EU twin gets a DISTINCT default backup
 * (`Bob-NA-backup` vs `Bob-EU-backup`) instead of colliding on `Bob-backup`.
 * Unknown-server characters are unique by name in the roster, so no tag is added.
 */
export function defaultCharacterBackupName(name: string, server: string): string {
  const tag = serverTag(server);
  return tag ? `${name}-${tag}-backup` : `${name}-backup`;
}
