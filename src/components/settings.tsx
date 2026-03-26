import { useState } from "react";

interface SettingsProps {
  addonsPath: string;
  onPathChange: (path: string) => void;
  onClose: () => void;
}

export function Settings({ addonsPath, onPathChange, onClose }: SettingsProps) {
  const [path, setPath] = useState(addonsPath);

  const handleSave = () => {
    if (path.trim()) {
      onPathChange(path.trim());
    }
    onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onClose();
    if (e.key === "Enter") handleSave();
  };

  return (
    <div className="settings-overlay" onClick={onClose}>
      <div
        className="settings-panel"
        onClick={(e) => e.stopPropagation()}
        onKeyDown={handleKeyDown}
      >
        <h2>Settings</h2>
        <div className="settings-field">
          <label htmlFor="addons-path">ESO AddOns Folder Path</label>
          <input
            id="addons-path"
            type="text"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder="C:\Users\...\Elder Scrolls Online\live\AddOns"
            autoFocus
          />
        </div>
        <div className="settings-actions">
          <button className="btn" onClick={onClose}>
            Cancel
          </button>
          <button className="btn" onClick={handleSave}>
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
