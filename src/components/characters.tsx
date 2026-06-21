import { useState, useEffect, useMemo } from "react";
import { toast } from "sonner";
import type { CharacterInfo, CharacterRoster } from "../types";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { InfoPill } from "@/components/ui/info-pill";
import { Fade } from "@/components/animate-ui/primitives/effects/fade";
import { CharactersSkeleton } from "@/components/ui/skeletons";
import { getTauriErrorMessage, invokeOrThrow } from "@/lib/tauri";

interface CharactersProps {
  addonsPath: string;
  onClose: () => void;
}

// Server bucket for characters recovered from SavedVariables whose megaserver
// can't be determined. Must match UNKNOWN_SERVER in the Rust `list_characters`.
const UNKNOWN_SERVER = "Unknown";

export function Characters({ addonsPath, onClose }: CharactersProps) {
  const [characters, setCharacters] = useState<CharacterInfo[]>([]);
  const [skippedFiles, setSkippedFiles] = useState(0);
  const [loading, setLoading] = useState(true);
  const [backupName, setBackupName] = useState("");
  const [backingUp, setBackingUp] = useState<string | null>(null);

  useEffect(() => {
    async function load() {
      try {
        const roster = await invokeOrThrow<CharacterRoster>("list_characters", {
          addonsPath,
        });
        setCharacters(roster.characters);
        setSkippedFiles(roster.skippedFiles);
      } catch (e) {
        toast.error(`Failed to load characters: ${getTauriErrorMessage(e)}`);
      } finally {
        setLoading(false);
      }
    }
    load();
  }, [addonsPath]);

  const handleBackup = async (char: CharacterInfo) => {
    // A character backup copies the whole SavedVariables files the character's
    // data lives in — it is keyed by name, not isolated per server, so two
    // same-named characters (an NA/EU twin) intentionally share one backup.
    const name = backupName.trim() || `${char.name}-backup`;
    setBackingUp(`${char.server}-${char.name}`);
    try {
      const count = await invokeOrThrow<number>("backup_character_settings", {
        addonsPath,
        characterName: char.name,
        backupName: name,
      });
      toast.success(`Backed up ${count} SavedVariables files for ${char.name}`);
    } catch (e) {
      toast.error(getTauriErrorMessage(e));
    } finally {
      setBackingUp(null);
    }
  };

  const byServer = useMemo(
    () =>
      characters.reduce(
        (acc, char) => {
          if (!acc[char.server]) acc[char.server] = [];
          acc[char.server]!.push(char);
          return acc;
        },
        {} as Record<string, CharacterInfo[]>
      ),
    [characters]
  );

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Characters</DialogTitle>
        </DialogHeader>

        <p className="text-sm text-muted-foreground">
          Your ESO characters. A backup copies the SavedVariables files a character&apos;s settings
          live in — those files can also hold your account-wide and other characters&apos; data.
        </p>

        {!loading && skippedFiles > 0 && (
          <p className="text-xs text-[#d9a441]">
            {skippedFiles} SavedVariables file{skippedFiles !== 1 ? "s" : ""} couldn&apos;t be read
            (too large, locked, or corrupt), so a character may be missing from this list.
          </p>
        )}

        <div>
          <label htmlFor="backup-name" className="text-xs text-muted-foreground">
            Backup name (optional)
          </label>
          <Input
            id="backup-name"
            placeholder="Leave blank for auto-name"
            value={backupName}
            onChange={(e) => setBackupName(e.target.value)}
          />
        </div>

        <div className="border-t border-white/[0.06]" />

        <div className="max-h-[350px] overflow-y-auto space-y-4">
          {loading ? (
            <CharactersSkeleton />
          ) : Object.keys(byServer).length === 0 ? (
            <Fade>
              <p className="text-sm text-muted-foreground text-center py-4">
                No characters found. Launch ESO at least once to generate character data.
              </p>
            </Fade>
          ) : (
            <div className="space-y-4">
              {Object.entries(byServer)
                .sort(([a], [b]) => {
                  // Keep the recovered "Unknown" bucket last; otherwise A–Z.
                  const au = a === UNKNOWN_SERVER ? 1 : 0;
                  const bu = b === UNKNOWN_SERVER ? 1 : 0;
                  return au - bu || a.localeCompare(b);
                })
                .map(([server, chars], serverIdx) => (
                  <Fade key={server} delay={serverIdx * 80}>
                    <div>
                      <div className="flex items-center gap-2 mb-2">
                        <InfoPill color={server === UNKNOWN_SERVER ? "muted" : "sky"}>
                          {server === UNKNOWN_SERVER ? "Unknown server" : server}
                        </InfoPill>
                        <span className="text-xs text-muted-foreground">
                          {chars.length} character{chars.length !== 1 ? "s" : ""}
                        </span>
                      </div>
                      {server === UNKNOWN_SERVER && (
                        <p className="text-xs text-muted-foreground mb-2">
                          Found in addon data; their megaserver couldn&apos;t be determined.
                        </p>
                      )}
                      <div className="space-y-1">
                        {chars.map((char, charIdx) => (
                          <Fade
                            key={`${char.server}-${char.name}`}
                            delay={serverIdx * 80 + charIdx * 40}
                          >
                            <div className="flex items-center justify-between rounded-xl border border-white/[0.06] bg-white/[0.02] p-3 transition-all duration-200 hover:border-white/[0.1]">
                              <span className="text-sm font-medium">{char.name}</span>
                              <Button
                                size="sm"
                                variant="outline"
                                onClick={() => handleBackup(char)}
                                disabled={backingUp === `${char.server}-${char.name}`}
                              >
                                {backingUp === `${char.server}-${char.name}`
                                  ? "Backing up..."
                                  : "Backup Settings"}
                              </Button>
                            </div>
                          </Fade>
                        ))}
                      </div>
                    </div>
                  </Fade>
                ))}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
