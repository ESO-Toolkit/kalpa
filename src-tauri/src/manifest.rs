use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

/// Strip ESO rich-text formatting codes from a string.
/// Matches: |cXXXXXX (color), |r (reset), |t (tab), |u..:|u (hyperlink).
fn strip_eso_codes(s: &str) -> String {
    static ESO_FORMAT_RE: OnceLock<Regex> = OnceLock::new();
    let re = ESO_FORMAT_RE
        .get_or_init(|| Regex::new(r"(?i)\|c[0-9a-f]{6}|\|r|\|t|\|u[^|]*:\|u").unwrap());
    re.replace_all(s, "").trim().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub min_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonManifest {
    pub folder_name: String,
    pub title: String,
    pub author: String,
    pub version: String,
    pub addon_version: Option<u32>,
    pub api_version: Vec<u32>,
    pub description: String,
    pub is_library: bool,
    pub depends_on: Vec<Dependency>,
    pub optional_depends_on: Vec<Dependency>,
    pub missing_dependencies: Vec<String>,
    #[serde(default)]
    pub outdated_dependencies: Vec<String>,
    /// Optional dependencies that are not currently installed. Lets the UI show
    /// a present/absent state for optional deps using the same subfolder-aware,
    /// case-insensitive resolution as required deps (computed in `commands.rs`).
    #[serde(default)]
    pub missing_optional_dependencies: Vec<String>,
    pub esoui_id: Option<u32>,
    pub tags: Vec<String>,
    pub esoui_last_update: u64,
    /// When this addon was installed/last updated locally, as an ISO 8601 UTC
    /// string (copied from the metadata store). Empty for addons Kalpa is not
    /// tracking (e.g. manually dropped in, or installed before metadata existed).
    #[serde(default)]
    pub installed_at: String,
    pub disabled: bool,
    #[serde(default)]
    pub modified_file_count: u32,
}

/// Strip zero-width / BOM characters that occasionally get glued onto manifest
/// tokens (copy-paste artifacts, exotic editors). Left in place they would make
/// a dependency name fail to match its on-disk folder and produce a false
/// "missing" flag.
fn strip_invisible(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'))
        .collect()
}

fn parse_dependencies(value: &str) -> Vec<Dependency> {
    // ESO writes `Name>=NNNN`, but tolerate stray whitespace around `>=`
    // (e.g. `Name >= NNNN`) so the version pin is never silently dropped.
    static GE_RE: OnceLock<Regex> = OnceLock::new();
    let re = GE_RE.get_or_init(|| Regex::new(r"\s*>=\s*").unwrap());
    let normalized = re.replace_all(value, ">=");

    normalized
        .split_whitespace()
        .filter_map(|dep| {
            let dep = strip_invisible(dep);
            let dep = dep.trim();
            if dep.is_empty() {
                return None;
            }
            if let Some(pos) = dep.find(">=") {
                let name = dep[..pos].trim().to_string();
                if name.is_empty() {
                    return None;
                }
                let min_version = dep[pos + 2..].trim().parse::<u32>().ok();
                Some(Dependency { name, min_version })
            } else {
                Some(Dependency {
                    name: dep.to_string(),
                    min_version: None,
                })
            }
        })
        .collect()
}

pub fn parse_manifest(folder_name: &str, manifest_path: &Path) -> Option<AddonManifest> {
    let bytes = fs::read(manifest_path).ok()?;
    let raw = String::from_utf8_lossy(&bytes);
    // Strip a leading UTF-8 BOM so the first directive (often `## Title:` or
    // `## AddOnVersion:`) isn't shadowed by an invisible byte-order mark.
    let content: &str = raw.strip_prefix('\u{FEFF}').unwrap_or(&raw);

    let mut title = String::new();
    let mut author = String::new();
    let mut version = String::new();
    let mut addon_version: Option<u32> = None;
    let mut api_version: Vec<u32> = Vec::new();
    let mut description = String::new();
    let mut is_library = false;
    let mut depends_on_raw = String::new();
    let mut optional_depends_on_raw = String::new();

    // Track which multi-line field we're continuing (DependsOn can span lines)
    let mut continuation: Option<&str> = None;

    for line in content.lines() {
        let line = line.trim();

        if let Some(line) = line.strip_prefix("## ") {
            continuation = None;
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };

            let key = key.trim();
            let value = value.trim();

            match key {
                "Title" => title = value.to_string(),
                "Author" => author = value.to_string(),
                "Version" => version = value.to_string(),
                "AddOnVersion" => addon_version = value.parse().ok(),
                "APIVersion" => {
                    api_version = value
                        .split_whitespace()
                        .filter_map(|v| v.parse().ok())
                        .collect();
                }
                "Description" => description = value.to_string(),
                "IsLibrary" => is_library = value.eq_ignore_ascii_case("true"),
                "DependsOn" => {
                    depends_on_raw = value.to_string();
                    continuation = Some("DependsOn");
                }
                "OptionalDependsOn" => {
                    optional_depends_on_raw = value.to_string();
                    continuation = Some("OptionalDependsOn");
                }
                _ => {}
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            // Continuation line for multi-line DependsOn.
            // Valid dep lines only contain addon names (word chars, hyphens, dots, >=).
            // Stop if we hit file listings (.lua, .xml, paths with /, ;, :).
            let looks_like_deps = !line.contains(".lua")
                && !line.contains(".xml")
                && !line.contains('/')
                && !line.contains('\\')
                && !line.contains(';')
                && !line.contains(':');

            if looks_like_deps {
                match continuation {
                    Some("DependsOn") => {
                        depends_on_raw.push(' ');
                        depends_on_raw.push_str(line);
                    }
                    Some("OptionalDependsOn") => {
                        optional_depends_on_raw.push(' ');
                        optional_depends_on_raw.push_str(line);
                    }
                    _ => {
                        continuation = None;
                    }
                }
            } else {
                continuation = None;
            }
        }
    }

    let depends_on = parse_dependencies(&depends_on_raw);
    let optional_depends_on = parse_dependencies(&optional_depends_on_raw);

    title = strip_eso_codes(&title);
    author = strip_eso_codes(&author);
    description = strip_eso_codes(&description);

    if title.is_empty() {
        title = folder_name.to_string();
    }

    Some(AddonManifest {
        folder_name: folder_name.to_string(),
        title,
        author,
        version,
        addon_version,
        api_version,
        description,
        is_library,
        depends_on,
        optional_depends_on,
        missing_dependencies: Vec::new(),
        outdated_dependencies: Vec::new(),
        missing_optional_dependencies: Vec::new(),
        esoui_id: None,
        tags: Vec::new(),
        esoui_last_update: 0,
        installed_at: String::new(),
        disabled: false,
        modified_file_count: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn write_manifest(dir: &Path, folder: &str, content: &str) -> PathBuf {
        let addon_dir = dir.join(folder);
        fs::create_dir_all(&addon_dir).unwrap();
        let path = addon_dir.join(format!("{folder}.txt"));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_basic_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: My Cool Addon
## Author: TestAuthor
## Version: 1.2.3
## APIVersion: 101042
## Description: A test addon
";
        let path = write_manifest(dir.path(), "MyCoolAddon", content);
        let m = parse_manifest("MyCoolAddon", &path).unwrap();

        assert_eq!(m.title, "My Cool Addon");
        assert_eq!(m.author, "TestAuthor");
        assert_eq!(m.version, "1.2.3");
        assert_eq!(m.api_version, vec![101042]);
        assert_eq!(m.description, "A test addon");
        assert!(!m.is_library);
        assert!(m.depends_on.is_empty());
    }

    #[test]
    fn parses_library_flag() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: LibStub
## IsLibrary: true
";
        let path = write_manifest(dir.path(), "LibStub", content);
        let m = parse_manifest("LibStub", &path).unwrap();

        assert!(m.is_library);
    }

    #[test]
    fn parses_dependencies_with_versions() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: TestAddon
## DependsOn: LibAddonMenu-2.0>=32 LibStub
## OptionalDependsOn: LibAsync
";
        let path = write_manifest(dir.path(), "TestAddon", content);
        let m = parse_manifest("TestAddon", &path).unwrap();

        assert_eq!(m.depends_on.len(), 2);
        assert_eq!(m.depends_on[0].name, "LibAddonMenu-2.0");
        assert_eq!(m.depends_on[0].min_version, Some(32));
        assert_eq!(m.depends_on[1].name, "LibStub");
        assert_eq!(m.depends_on[1].min_version, None);
        assert_eq!(m.optional_depends_on.len(), 1);
        assert_eq!(m.optional_depends_on[0].name, "LibAsync");
    }

    #[test]
    fn parses_dependencies_with_spaces_around_operator() {
        // Some manifests (or hand edits) put spaces around ">=" — the version
        // pin must still be captured, not split into a bogus separate token.
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: TestAddon
## DependsOn: LuiData >= 7221 LuiMedia>= 7133 LibStub
";
        let path = write_manifest(dir.path(), "TestAddon", content);
        let m = parse_manifest("TestAddon", &path).unwrap();

        assert_eq!(m.depends_on.len(), 3);
        assert_eq!(m.depends_on[0].name, "LuiData");
        assert_eq!(m.depends_on[0].min_version, Some(7221));
        assert_eq!(m.depends_on[1].name, "LuiMedia");
        assert_eq!(m.depends_on[1].min_version, Some(7133));
        assert_eq!(m.depends_on[2].name, "LibStub");
        assert_eq!(m.depends_on[2].min_version, None);
    }

    #[test]
    fn strips_zero_width_chars_from_dependency_names() {
        let dir = tempfile::tempdir().unwrap();
        // Zero-width space glued onto a dependency token.
        let content = "## Title: T\n## DependsOn: LuiMedia\u{200B}>=7133\n";
        let path = write_manifest(dir.path(), "T", content);
        let m = parse_manifest("T", &path).unwrap();

        assert_eq!(m.depends_on.len(), 1);
        assert_eq!(m.depends_on[0].name, "LuiMedia");
        assert_eq!(m.depends_on[0].min_version, Some(7133));
    }

    #[test]
    fn parses_manifest_with_utf8_bom() {
        let dir = tempfile::tempdir().unwrap();
        // Leading UTF-8 BOM before the first directive.
        let content = "\u{FEFF}## Title: BomAddon\n## AddOnVersion: 7221\n";
        let path = write_manifest(dir.path(), "BomAddon", content);
        let m = parse_manifest("BomAddon", &path).unwrap();

        assert_eq!(m.title, "BomAddon");
        assert_eq!(m.addon_version, Some(7221));
    }

    #[test]
    fn parses_multiple_api_versions() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: TestAddon
## APIVersion: 101042 101043
";
        let path = write_manifest(dir.path(), "TestAddon", content);
        let m = parse_manifest("TestAddon", &path).unwrap();

        assert_eq!(m.api_version, vec![101042, 101043]);
    }

    #[test]
    fn strips_eso_formatting_codes() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: |cFFD700Fancy|r Addon
## Author: |c00FF00Green|r Author
";
        let path = write_manifest(dir.path(), "FancyAddon", content);
        let m = parse_manifest("FancyAddon", &path).unwrap();

        assert_eq!(m.title, "Fancy Addon");
        assert_eq!(m.author, "Green Author");
    }

    #[test]
    fn falls_back_to_folder_name_for_empty_title() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Version: 1.0
";
        let path = write_manifest(dir.path(), "NoTitle", content);
        let m = parse_manifest("NoTitle", &path).unwrap();

        assert_eq!(m.title, "NoTitle");
    }

    #[test]
    fn parses_addon_version_number() {
        let dir = tempfile::tempdir().unwrap();
        let content = "\
## Title: TestAddon
## AddOnVersion: 42
";
        let path = write_manifest(dir.path(), "TestAddon", content);
        let m = parse_manifest("TestAddon", &path).unwrap();

        assert_eq!(m.addon_version, Some(42));
    }

    #[test]
    fn returns_none_for_missing_file() {
        let result = parse_manifest("NoSuchAddon", Path::new("/nonexistent/path.txt"));
        assert!(result.is_none());
    }
}
