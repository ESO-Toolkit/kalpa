import type { AddonManifest } from "../types";

interface AddonListProps {
  addons: AddonManifest[];
  selectedAddon: AddonManifest | null;
  onSelect: (addon: AddonManifest) => void;
  searchQuery: string;
  onSearchChange: (query: string) => void;
  loading: boolean;
}

export function AddonList({
  addons,
  selectedAddon,
  onSelect,
  searchQuery,
  onSearchChange,
  loading,
}: AddonListProps) {
  return (
    <div className="addon-list-panel">
      <div className="search-bar">
        <input
          type="text"
          placeholder="Search addons..."
          value={searchQuery}
          onChange={(e) => onSearchChange(e.target.value)}
        />
      </div>
      <div className="addon-list">
        {loading ? (
          <div className="loading">
            <div className="spinner" />
          </div>
        ) : addons.length === 0 ? (
          <div className="loading">No addons found</div>
        ) : (
          addons.map((addon) => (
            <div
              key={addon.folderName}
              className={`addon-item ${
                selectedAddon?.folderName === addon.folderName ? "selected" : ""
              }`}
              onClick={() => onSelect(addon)}
            >
              <div className="addon-item-header">
                <span className="addon-item-title">{addon.title}</span>
                {addon.isLibrary && (
                  <span className="badge badge-lib">LIB</span>
                )}
                {addon.missingDependencies.length > 0 && (
                  <span className="badge badge-warning">
                    {addon.missingDependencies.length} missing
                  </span>
                )}
                <span className="addon-item-version">
                  {addon.version || `v${addon.addonVersion ?? "?"}`}
                </span>
              </div>
              {addon.author && (
                <div className="addon-item-author">by {addon.author}</div>
              )}
            </div>
          ))
        )}
      </div>
    </div>
  );
}
