use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Dependency {
    pub name: String,
    pub min_version: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
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
}

fn parse_dependencies(value: &str) -> Vec<Dependency> {
    value
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|dep| {
            if let Some(pos) = dep.find(">=") {
                let name = dep[..pos].to_string();
                let min_version = dep[pos + 2..].parse::<u32>().ok();
                Dependency { name, min_version }
            } else {
                Dependency {
                    name: dep.to_string(),
                    min_version: None,
                }
            }
        })
        .collect()
}

pub fn parse_manifest(folder_name: &str, manifest_path: &Path) -> Option<AddonManifest> {
    let content = fs::read_to_string(manifest_path).ok()?;

    let mut title = String::new();
    let mut author = String::new();
    let mut version = String::new();
    let mut addon_version: Option<u32> = None;
    let mut api_version: Vec<u32> = Vec::new();
    let mut description = String::new();
    let mut is_library = false;
    let mut depends_on: Vec<Dependency> = Vec::new();
    let mut optional_depends_on: Vec<Dependency> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("## ") {
            continue;
        }

        let line = &line[3..];
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
            "DependsOn" => depends_on = parse_dependencies(value),
            "OptionalDependsOn" => optional_depends_on = parse_dependencies(value),
            _ => {}
        }
    }

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
    })
}
