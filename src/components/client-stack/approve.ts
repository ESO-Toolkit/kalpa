import { invokeOrThrow } from "@/lib/tauri";
import type { EsoClientLocation } from "@/components/client-stack/types";

/**
 * Approve a client folder for writes, for the rest of this session.
 *
 * **Every command that writes to the client folder must be preceded by this
 * call, from inside the handler for the user's own click on that specific
 * folder.** The backend's write gate holds one approved root and refuses any
 * write to a folder that is not it, so a write command called without this
 * fails outright — which is exactly what shipped once already.
 *
 * It is deliberately not called on detect, inspect or refresh. Reading a folder
 * must not be what grants permission to write to it; if it were, the gate would
 * be satisfied by the panel merely being open.
 */
export async function approveClientWrites(clientDir: string): Promise<void> {
  await invokeOrThrow<EsoClientLocation>("set_game_install_path", { path: clientDir });
}
