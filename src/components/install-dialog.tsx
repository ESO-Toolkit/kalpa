import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { EsouiAddonInfo } from "../types";

type InstallState =
  | "idle"
  | "resolving"
  | "resolved"
  | "installing"
  | "installed"
  | "error";

interface InstallDialogProps {
  addonsPath: string;
  onInstalled: () => void;
  onClose: () => void;
}

export function InstallDialog({
  addonsPath,
  onInstalled,
  onClose,
}: InstallDialogProps) {
  const [input, setInput] = useState("");
  const [state, setState] = useState<InstallState>("idle");
  const [addonInfo, setAddonInfo] = useState<EsouiAddonInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installedFolders, setInstalledFolders] = useState<string[]>([]);

  const handleResolve = async () => {
    if (!input.trim()) return;
    setState("resolving");
    setError(null);
    try {
      const info = await invoke<EsouiAddonInfo>("resolve_esoui_addon", {
        input: input.trim(),
      });
      setAddonInfo(info);
      setState("resolved");
    } catch (e) {
      setError(String(e));
      setState("error");
    }
  };

  const handleInstall = async () => {
    if (!addonInfo) return;
    setState("installing");
    setError(null);
    try {
      const folders = await invoke<string[]>("install_addon", {
        addonsPath,
        downloadUrl: addonInfo.downloadUrl,
      });
      setInstalledFolders(folders);
      setState("installed");
      onInstalled();
    } catch (e) {
      setError(String(e));
      setState("error");
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    if (e.key === "Enter" && state === "idle") handleResolve();
  };

  const busy = state === "resolving" || state === "installing";

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div
        className="settings-panel"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <h2>Install Addon</h2>

        <div className="settings-field">
          <label htmlFor="esoui-input">ESOUI URL or Addon ID</label>
          <input
            id="esoui-input"
            type="text"
            value={input}
            onChange={(e) => {
              setInput(e.target.value);
              if (state !== "idle" && state !== "error") {
                setState("idle");
                setAddonInfo(null);
              }
            }}
            placeholder="https://www.esoui.com/downloads/info123 or 123"
            disabled={busy}
            autoFocus
          />
        </div>

        {addonInfo && state === "resolved" && (
          <div className="install-preview">
            <div className="install-preview-title">{addonInfo.title}</div>
            <div className="install-preview-meta">
              ESOUI #{addonInfo.id}
            </div>
          </div>
        )}

        {state === "installed" && (
          <div className="install-success">
            Installed successfully
            {installedFolders.length > 0 && (
              <span className="install-folders">
                : {installedFolders.join(", ")}
              </span>
            )}
          </div>
        )}

        {error && <div className="install-error">{error}</div>}

        <div className="settings-actions">
          <button className="btn" onClick={onClose}>
            {state === "installed" ? "Close" : "Cancel"}
          </button>

          {(state === "idle" || state === "error") && (
            <button
              className="btn btn-accent"
              onClick={handleResolve}
              disabled={!input.trim()}
            >
              Resolve
            </button>
          )}

          {state === "resolving" && (
            <button className="btn" disabled>
              Resolving...
            </button>
          )}

          {state === "resolved" && (
            <button className="btn btn-accent" onClick={handleInstall}>
              Install
            </button>
          )}

          {state === "installing" && (
            <button className="btn" disabled>
              Installing...
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
