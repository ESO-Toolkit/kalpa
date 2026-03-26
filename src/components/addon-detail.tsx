import type { AddonManifest } from "../types";

interface AddonDetailProps {
  addon: AddonManifest | null;
  installedAddons: AddonManifest[];
}

export function AddonDetail({ addon, installedAddons }: AddonDetailProps) {
  if (!addon) {
    return (
      <div className="detail-panel">
        <div className="detail-empty">Select an addon to view details</div>
      </div>
    );
  }

  const installedSet = new Set(installedAddons.map((a) => a.folderName));

  return (
    <div className="detail-panel">
      <h2 className="detail-title">{addon.title}</h2>
      <div className="detail-folder">{addon.folderName}/</div>

      <dl className="detail-meta">
        {addon.author && (
          <>
            <dt>Author</dt>
            <dd>{addon.author}</dd>
          </>
        )}
        <dt>Version</dt>
        <dd>{addon.version || addon.addonVersion || "Unknown"}</dd>
        {addon.apiVersion.length > 0 && (
          <>
            <dt>API Version</dt>
            <dd>{addon.apiVersion.join(", ")}</dd>
          </>
        )}
        <dt>Type</dt>
        <dd>{addon.isLibrary ? "Library" : "Addon"}</dd>
      </dl>

      {addon.description && (
        <div className="detail-section">
          <h3>Description</h3>
          <p>{addon.description}</p>
        </div>
      )}

      {addon.dependsOn.length > 0 && (
        <div className="detail-section">
          <h3>Required Dependencies</h3>
          <ul className="dep-list">
            {addon.dependsOn.map((dep) => {
              const installed = installedSet.has(dep.name);
              return (
                <li key={dep.name}>
                  <span className={installed ? "dep-ok" : "dep-missing"}>
                    {installed ? "\u2713" : "\u2717"}
                  </span>
                  <span>{dep.name}</span>
                  {dep.min_version !== null && (
                    <span className="addon-item-version">
                      {" "}
                      &gt;={dep.min_version}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}

      {addon.optionalDependsOn.length > 0 && (
        <div className="detail-section">
          <h3>Optional Dependencies</h3>
          <ul className="dep-list">
            {addon.optionalDependsOn.map((dep) => {
              const installed = installedSet.has(dep.name);
              return (
                <li key={dep.name} className="dep-optional">
                  <span className={installed ? "dep-ok" : ""}>
                    {installed ? "\u2713" : "\u25CB"}
                  </span>
                  <span>{dep.name}</span>
                  {dep.min_version !== null && (
                    <span className="addon-item-version">
                      {" "}
                      &gt;={dep.min_version}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      )}
    </div>
  );
}
