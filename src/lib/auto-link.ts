import { toast } from "sonner";
import { getSettingChecked, setSetting } from "@/lib/store";

/** Unmatched local folders are expected for custom/private addons. Explain new
 * ones once per AddOns directory, without reporting deliberate bundle skips. */
export async function notifyUnmatchedAddons(path: string, folders: string[]): Promise<void> {
  if (folders.length === 0) return;

  const key = `autoLink.notifiedFolders:${path}`;
  const saved = await getSettingChecked<unknown>(key, []);
  // Avoid overwriting notice history or repeating notices during a store failure.
  if (!saved.ok) return;
  const seen = new Set(
    Array.isArray(saved.value)
      ? saved.value.filter((folder): folder is string => typeof folder === "string")
      : []
  );
  const unseen = [...new Set(folders)].filter((folder) => !seen.has(folder)).sort();
  if (unseen.length === 0) return;

  toast.info(
    `${unseen.length} local addon${unseen.length === 1 ? " has" : "s have"} no ESOUI match`,
    {
      description: `${unseen.join(", ")}. Custom or private addons may have no ESOUI listing. ESO can still load these folders; Kalpa cannot check them for updates.`,
      duration: 10000,
    }
  );

  // Keep a union so removing one folder does not re-notify all remaining folders.
  await setSetting(key, [...new Set([...seen, ...unseen])].sort());
}
