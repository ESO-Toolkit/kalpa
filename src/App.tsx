import { useEffect, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AddonList } from "./components/addon-list";
import { AddonDetail } from "./components/addon-detail";
import { Settings } from "./components/settings";
import type { AddonManifest } from "./types";

function App() {
  const [addonsPath, setAddonsPath] = useState<string>("");
  const [addons, setAddons] = useState<AddonManifest[]>([]);
  const [selectedAddon, setSelectedAddon] = useState<AddonManifest | null>(
    null,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");

  const scanAddons = useCallback(
    async (path: string) => {
      setLoading(true);
      setError(null);
      try {
        const result = await invoke<AddonManifest[]>("scan_installed_addons", {
          addonsPath: path,
        });
        setAddons(result);
        // Update selected addon if it still exists
        if (selectedAddon) {
          const updated = result.find(
            (a) => a.folderName === selectedAddon.folderName,
          );
          setSelectedAddon(updated ?? null);
        }
      } catch (e) {
        setError(String(e));
        setAddons([]);
      } finally {
        setLoading(false);
      }
    },
    [selectedAddon],
  );

  useEffect(() => {
    async function init() {
      try {
        const path = await invoke<string>("detect_addons_folder");
        setAddonsPath(path);
        await scanAddons(path);
      } catch {
        setError(
          "Could not detect ESO AddOns folder. Please set it in Settings.",
        );
        setLoading(false);
      }
    }
    init();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleRefresh = () => {
    if (addonsPath) {
      scanAddons(addonsPath);
    }
  };

  const handlePathChange = (newPath: string) => {
    setAddonsPath(newPath);
    setSelectedAddon(null);
    scanAddons(newPath);
  };

  const filteredAddons = addons.filter((addon) => {
    if (!searchQuery) return true;
    const q = searchQuery.toLowerCase();
    return (
      addon.title.toLowerCase().includes(q) ||
      addon.folderName.toLowerCase().includes(q) ||
      addon.author.toLowerCase().includes(q)
    );
  });

  const missingDepCount = addons.filter(
    (a) => a.missingDependencies.length > 0,
  ).length;

  return (
    <div className="app">
      <header className="header">
        <h1>ESO Addon Manager</h1>
        <div className="header-actions">
          <span className="addon-count">
            {addons.length} addons
            {missingDepCount > 0 && ` · ${missingDepCount} with issues`}
          </span>
          <button className="btn" onClick={handleRefresh} disabled={loading}>
            {loading ? "Scanning..." : "Refresh"}
          </button>
          <button className="btn" onClick={() => setShowSettings(true)}>
            Settings
          </button>
        </div>
      </header>

      {error && <div className="error-banner">{error}</div>}

      <div className="main-content">
        <AddonList
          addons={filteredAddons}
          selectedAddon={selectedAddon}
          onSelect={setSelectedAddon}
          searchQuery={searchQuery}
          onSearchChange={setSearchQuery}
          loading={loading}
        />
        <AddonDetail addon={selectedAddon} installedAddons={addons} />
      </div>

      {showSettings && (
        <Settings
          addonsPath={addonsPath}
          onPathChange={handlePathChange}
          onClose={() => setShowSettings(false)}
        />
      )}
    </div>
  );
}

export default App;
