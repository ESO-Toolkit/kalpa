//! Presets: switching between them, and fixing the one ordering mistake that
//! does not announce itself.
//!
//! # Why order is worth a feature
//!
//! `DLSS5_Feed` consumes motion vectors that another effect produces. ReShade
//! runs techniques in the order the preset lists them, so a preset that enables
//! the feed *above* its provider compiles cleanly, loads cleanly, errors
//! nothing — and feeds DLSS last frame's data. Nothing about either file is
//! defective. The image is just quietly wrong. That is the whole reason this
//! module offers a fix rather than only a report.
//!
//! # Which effect is the provider is not a constant
//!
//! It is resolved per preset by [`crate::client_stack`] from `DLSS5_MV_SOURCE`
//! and `MV_PROVIDER`: iMMERSE LaunchPad, or whichever enabled effect writes the
//! shared `texMotionVectors` texture. The fix moves *that* technique, read from
//! [`crate::client_stack::MvProvider`], never a hardcoded name — assuming
//! LaunchPad is what made the check silently not run in the first place.
//!
//! # Surgical edits only
//!
//! Both writes here change exactly one line of one file: `PresetPath` in
//! `ReShade.ini`, or `Techniques` in the preset. Everything else in the file
//! survives byte for byte, including comments, key order, per-effect uniform
//! blocks and the file's line endings. ReShade and its add-ons own these files;
//! Kalpa is editing one value in someone else's document, and a reserialised
//! file would be a whole-file diff for a one-value change.

use crate::client_stack::ClientStack;
use crate::client_write::{AllowedGameInstallPath, ManagedKind};
use serde::Serialize;
use std::path::Path;

/// The technique name `DLSS5_Feed.fx` is always enabled under. Unlike the
/// *provider*, this name is not configurable — only which effect feeds it
/// varies, which is why [`plan_order_fix`] takes that from
/// [`crate::client_stack::MvProvider`] instead.
const FEED_TECHNIQUE_NAME: &str = "DLSS5_Feed";

/// One preset the user could switch to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetChoice {
    /// File name relative to the client directory.
    pub relative_path: String,
    /// The form that goes into `PresetPath`, e.g. `.\ReShadePreset.ini`.
    pub preset_path: String,
    pub is_active: bool,
    /// How many techniques it enables, so the list is more than a row of
    /// identical file names.
    pub technique_count: usize,
}

/// The `Techniques` reordering Kalpa would perform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrderFix {
    /// The technique that has to run first, resolved from the preset's own
    /// `MV_PROVIDER`.
    pub provider_technique: String,
    pub feed_technique: String,
    /// The `Techniques=` value as it is now.
    pub before: String,
    /// The `Techniques=` value the fix would write.
    pub after: String,
    /// One line the confirmation shows verbatim.
    pub summary: String,
}

/// Everything the preset panel needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetOptions {
    pub client_dir: String,
    /// The active preset's relative path, when `PresetPath` names a file that
    /// is actually there.
    pub active: Option<String>,
    /// Every preset found at the client root, sorted by file name, active first
    /// only if that is where sorting puts it — the list order must not encode a
    /// recommendation.
    pub choices: Vec<PresetChoice>,
    /// The fix, when the active preset runs the feed before its provider.
    /// `None` when the order is right, when there is no feed technique, or when
    /// no provider technique is enabled at all — that last case is a different
    /// problem (`stack-mv-provider-missing`) that reordering cannot solve, and
    /// offering a reorder for it would be a fix that changes nothing.
    pub fix: Option<OrderFix>,
}

/// The result of a preset switch or an order fix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetChangeOutcome {
    /// The file that was edited, relative to the client directory.
    pub relative_path: String,
    /// Backup folder id holding the previous contents.
    pub backup_id: Option<String>,
    /// What changed, in one line, for the toast.
    pub summary: String,
}

// ── INI reading ──────────────────────────────────────────────────────────

/// Read one `key=value` out of the headerless top block of an INI-shaped file.
fn top_section_value(contents: &str, key: &str) -> Option<String> {
    section_value(contents, "", key)
}

/// Read one `key=value` out of a named section, comparisons case-insensitive
/// on both section and key. A minimal reader deliberately kept local to this
/// module: [`crate::client_stack`]'s own parser is private to that module.
fn section_value(contents: &str, section: &str, key: &str) -> Option<String> {
    let target_section = section.trim().to_ascii_lowercase();
    let target_key = key.trim().to_ascii_lowercase();
    let mut current_section = String::new();

    for raw_line in contents.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current_section = name.trim().to_ascii_lowercase();
            continue;
        }
        if current_section != target_section {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim().to_ascii_lowercase() == target_key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// The `name` half of a ReShade `name@source.fx` technique entry.
fn technique_name(entry: &str) -> &str {
    entry.split('@').next().unwrap_or(entry).trim()
}

// ── Public helpers ───────────────────────────────────────────────────────

/// Find the presets in a client directory.
///
/// A preset is a `.ini` at the client root that is not `ReShade.ini` and that
/// has a `Techniques` key. The key is the test rather than the file name:
/// ReShade does not enforce a naming convention and users rename presets freely,
/// so matching on `*Preset*.ini` would hide half of them.
pub fn find_presets(client_dir: &Path, active_relative: Option<&str>) -> Vec<PresetChoice> {
    let mut choices = Vec::new();
    let Ok(entries) = std::fs::read_dir(client_dir) else {
        return choices;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if !lower.ends_with(".ini") || lower == "reshade.ini" {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(techniques_value) = top_section_value(&contents, "Techniques") else {
            continue;
        };

        let technique_count = techniques_value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .count();
        let is_active = active_relative.is_some_and(|active| active.eq_ignore_ascii_case(&name));

        choices.push(PresetChoice {
            preset_path: to_preset_path(&name),
            relative_path: name,
            is_active,
            technique_count,
        });
    }

    choices.sort_by(|a, b| {
        a.relative_path
            .to_ascii_lowercase()
            .cmp(&b.relative_path.to_ascii_lowercase())
    });
    choices
}

/// Turn a relative preset file name into the `PresetPath` form ReShade writes.
///
/// ReShade uses `.\Name.ini`. Kalpa writes the same shape rather than an
/// absolute path: an absolute path in this key would break the moment the user
/// moved or reinstalled the game.
pub fn to_preset_path(relative_path: &str) -> String {
    format!(".\\{relative_path}")
}

/// The inverse of [`to_preset_path`], tolerant of `.\`, `./` and a bare name.
pub fn from_preset_path(preset_path: &str) -> String {
    let trimmed = preset_path.trim().trim_matches('"');
    let trimmed = trimmed
        .strip_prefix(".\\")
        .or_else(|| trimmed.strip_prefix("./"))
        .unwrap_or(trimmed);
    trimmed.to_string()
}

/// Work out the ordering fix for a stack, or `None` when there is nothing to
/// fix. See [`PresetOptions::fix`] for exactly when this is `None`.
///
/// The fix moves the provider technique to sit immediately before the feed,
/// leaving every other technique in its existing relative order. Techniques
/// keep their `name@source.fx` spelling exactly as the preset had them.
pub fn plan_order_fix(stack: &ClientStack, preset_contents: &str) -> Option<OrderFix> {
    let preset = stack.preset.as_ref()?;
    let provider = preset.mv_provider.as_ref()?;
    let provider_technique = provider.technique.as_ref()?;

    let before = top_section_value(preset_contents, "Techniques")?;
    let mut entries: Vec<String> = before
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let feed_idx = entries
        .iter()
        .position(|e| technique_name(e).eq_ignore_ascii_case(FEED_TECHNIQUE_NAME))?;
    let provider_idx = entries
        .iter()
        .position(|e| technique_name(e).eq_ignore_ascii_case(provider_technique))?;

    if provider_idx <= feed_idx {
        // Already correctly ordered — or, if the two indices somehow matched,
        // there is nothing sane to move.
        return None;
    }

    let feed_name = technique_name(&entries[feed_idx]).to_string();
    let provider_name = technique_name(&entries[provider_idx]).to_string();

    let moved = entries.remove(provider_idx);
    entries.insert(feed_idx, moved);
    let after = entries.join(",");

    Some(OrderFix {
        summary: format!(
            "Move {provider_name} to run before {feed_name} so DLSS 5 Feed reads current \
             motion vectors."
        ),
        provider_technique: provider_name,
        feed_technique: feed_name,
        before,
        after,
    })
}

/// Rewrite one `key=value` line in an INI file, leaving the rest byte for byte.
///
/// `section` is `""` for the preset's headerless top block, where `Techniques`
/// lives. Preserves the line's original terminator and the file's; returns `Err`
/// when the key is not present, because inventing a `PresetPath` or a
/// `Techniques` line in a file that has none would be Kalpa writing
/// configuration rather than editing it.
pub fn replace_ini_value(
    contents: &str,
    section: &str,
    key: &str,
    value: &str,
) -> Result<String, String> {
    let target_section = section.trim().to_ascii_lowercase();
    let target_key = key.trim().to_ascii_lowercase();

    let mut out = String::with_capacity(contents.len() + value.len() + 8);
    let mut current_section = String::new();
    let mut found = false;
    let mut rest = contents;

    while !rest.is_empty() {
        let line_end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
        let raw_line = &rest[..line_end];
        rest = &rest[line_end..];

        let (content, terminator) = if let Some(stripped) = raw_line.strip_suffix("\r\n") {
            (stripped, "\r\n")
        } else if let Some(stripped) = raw_line.strip_suffix('\n') {
            (stripped, "\n")
        } else {
            (raw_line, "")
        };
        let trimmed = content.trim();

        if found || trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            out.push_str(raw_line);
            continue;
        }

        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current_section = name.trim().to_ascii_lowercase();
            out.push_str(raw_line);
            continue;
        }

        if current_section == target_section {
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().to_ascii_lowercase() == target_key {
                    out.push_str(k.trim());
                    out.push('=');
                    out.push_str(value);
                    out.push_str(terminator);
                    found = true;
                    continue;
                }
            }
        }

        out.push_str(raw_line);
    }

    if !found {
        let where_ = if section.trim().is_empty() {
            "the top-level section".to_string()
        } else {
            format!("section [{section}]")
        };
        return Err(format!("{key} was not found in {where_}."));
    }

    Ok(out)
}

// ── Commands ─────────────────────────────────────────────────────────────

/// Read-only: the presets available and whether the active one is misordered.
#[tauri::command(async)]
pub fn list_client_presets(client_dir: String) -> Result<PresetOptions, String> {
    let location = crate::client_install::validate_client_dir(Path::new(&client_dir))?;
    let stack = crate::client_stack::inspect_stack(&location.client_dir);

    let active = stack
        .preset
        .as_ref()
        .filter(|preset| preset.exists)
        .map(|preset| from_preset_path(&preset.path));

    let choices = find_presets(&location.client_dir, active.as_deref());

    let fix = stack
        .preset
        .as_ref()
        .filter(|preset| preset.exists)
        .and_then(|preset| {
            // `PresetPath` is a value out of someone else's config file, so it
            // is joined through the containment check like any other untrusted
            // relative path — `Path::join` with `C:whatever.ini` or a `..`
            // would silently resolve outside the client folder.
            let relative = from_preset_path(&preset.path);
            let path =
                crate::client_write::safe_relative_join(&location.client_dir, &relative).ok()?;
            let contents = std::fs::read_to_string(path).ok()?;
            plan_order_fix(&stack, &contents)
        });

    Ok(PresetOptions {
        client_dir: location.client_dir.to_string_lossy().to_string(),
        active,
        choices,
        fix,
    })
}

/// Switch the active preset by rewriting `PresetPath` in `ReShade.ini`.
///
/// Refuses a preset that is not one of the choices [`find_presets`] returns,
/// rather than trusting a path from the frontend — the write path would accept
/// any `.ini` under the client folder, and "the user picked it from a list" is
/// only true if the backend checks it against the same list.
#[tauri::command]
pub async fn set_client_preset(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
    relative_path: String,
) -> Result<PresetChangeOutcome, String> {
    let root = crate::client_write::begin_write(&state, &client_dir).await?;

    tokio::task::spawn_blocking(move || {
        let client_root = root.path().to_path_buf();

        let choices = find_presets(&client_root, None);
        if !choices
            .iter()
            .any(|choice| choice.relative_path == relative_path)
        {
            return Err(format!(
                "{relative_path} is not one of the presets in this folder."
            ));
        }

        let reshade_ini = client_root.join("ReShade.ini");
        let contents = std::fs::read_to_string(&reshade_ini)
            .map_err(|e| format!("Failed to read ReShade.ini: {e}"))?;
        let preset_path_value = to_preset_path(&relative_path);
        let updated = replace_ini_value(&contents, "GENERAL", "PresetPath", &preset_path_value)?;

        let outcome = crate::client_backup::edit_managed_file(
            &app,
            &root,
            "ReShade.ini",
            ManagedKind::ReShadeConfig,
            updated.as_bytes(),
        )?;

        Ok(PresetChangeOutcome {
            relative_path: "ReShade.ini".to_string(),
            backup_id: outcome.backup_id,
            summary: format!("Switched the active preset to {relative_path}."),
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

/// Reorder the active preset's `Techniques` so the provider runs before the feed.
///
/// The fix is recomputed here from the folder rather than accepted from the
/// caller, for the same reason `adopt_stack` recomputes its own plan.
#[tauri::command]
pub async fn fix_client_technique_order(
    app: tauri::AppHandle,
    state: tauri::State<'_, AllowedGameInstallPath>,
    client_dir: String,
) -> Result<PresetChangeOutcome, String> {
    let root = crate::client_write::begin_write(&state, &client_dir).await?;

    tokio::task::spawn_blocking(move || {
        let client_root = root.path().to_path_buf();
        let stack = crate::client_stack::inspect_stack(&client_root);

        let preset = stack
            .preset
            .as_ref()
            .ok_or_else(|| "This client folder has no active preset.".to_string())?;
        if !preset.exists {
            return Err("The active preset file does not exist.".to_string());
        }
        let relative = from_preset_path(&preset.path);

        // Same reasoning as `list_client_presets`: `PresetPath` is untrusted
        // text, and `Path::join` would happily follow it out of the folder.
        let preset_file = crate::client_write::safe_relative_join(&client_root, &relative)?;
        let contents = std::fs::read_to_string(&preset_file)
            .map_err(|e| format!("Failed to read {relative}: {e}"))?;

        let fix = plan_order_fix(&stack, &contents).ok_or_else(|| {
            "The active preset's technique order does not need a fix.".to_string()
        })?;

        let updated = replace_ini_value(&contents, "", "Techniques", &fix.after)?;

        let outcome = crate::client_backup::edit_managed_file(
            &app,
            &root,
            &relative,
            ManagedKind::Preset,
            updated.as_bytes(),
        )?;

        Ok(PresetChangeOutcome {
            relative_path: relative,
            backup_id: outcome.backup_id,
            summary: fix.summary,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_backup::edit_managed_file_in;
    use crate::client_stack::inspect_stack;
    use crate::client_write::ApprovedRoot;

    fn write(dir: &Path, name: &str, contents: &str) {
        std::fs::write(dir.join(name), contents).expect("write fixture");
    }

    // ── find_presets ─────────────────────────────────────────────────────

    #[test]
    fn find_presets_picks_up_a_renamed_preset() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            tmp.path(),
            "MyLook.ini",
            "Techniques=A@A.fx,B@B.fx\nTechniqueSorting=A@A.fx,B@B.fx\n",
        );

        let choices = find_presets(tmp.path(), None);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].relative_path, "MyLook.ini");
        assert_eq!(choices[0].technique_count, 2);
        assert_eq!(choices[0].preset_path, ".\\MyLook.ini");
    }

    #[test]
    fn find_presets_skips_reshade_ini_and_files_without_techniques() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "ReShade.ini", "Techniques=A@A.fx\n");
        write(tmp.path(), "Notes.ini", "[GENERAL]\nSomething=1\n");
        write(tmp.path(), "Real.ini", "Techniques=A@A.fx\n");

        let names: Vec<String> = find_presets(tmp.path(), None)
            .into_iter()
            .map(|c| c.relative_path)
            .collect();
        assert_eq!(names, vec!["Real.ini".to_string()]);
    }

    #[test]
    fn find_presets_marks_the_active_one() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "A.ini", "Techniques=A@A.fx\n");
        write(tmp.path(), "B.ini", "Techniques=B@B.fx\n");

        let active: Vec<String> = find_presets(tmp.path(), Some("B.ini"))
            .into_iter()
            .filter(|c| c.is_active)
            .map(|c| c.relative_path)
            .collect();
        assert_eq!(active, vec!["B.ini".to_string()]);
    }

    // ── preset path round trip ───────────────────────────────────────────

    #[test]
    fn preset_path_round_trips() {
        assert_eq!(to_preset_path("MyLook.ini"), ".\\MyLook.ini");
        assert_eq!(from_preset_path(".\\MyLook.ini"), "MyLook.ini");
        assert_eq!(from_preset_path("./MyLook.ini"), "MyLook.ini");
        assert_eq!(from_preset_path("MyLook.ini"), "MyLook.ini");
        assert_eq!(
            from_preset_path(&to_preset_path("MyLook.ini")),
            "MyLook.ini"
        );
    }

    // ── replace_ini_value ────────────────────────────────────────────────

    #[test]
    fn replace_ini_value_errs_when_the_key_is_absent() {
        let err = replace_ini_value("[GENERAL]\nOther=1\n", "GENERAL", "PresetPath", "x")
            .expect_err("missing key must error");
        assert!(err.contains("PresetPath"), "{err}");
    }

    #[test]
    fn replace_ini_value_edits_the_right_section_when_the_key_repeats() {
        let contents = "[A]\nX=1\n[B]\nX=2\n";
        let updated = replace_ini_value(contents, "B", "X", "9").expect("edit");
        assert_eq!(updated, "[A]\nX=1\n[B]\nX=9\n");
    }

    /// Real preset files are CRLF on Windows; everything but the one edited
    /// line must survive byte for byte.
    #[test]
    fn replace_ini_value_preserves_crlf_and_the_rest_of_the_file() {
        let original = "Techniques=A@A.fx,B@B.fx\r\nTechniqueSorting=A@A.fx,B@B.fx\r\n\r\n[DLSS5_Feed.fx]\r\nDEBUG_VIEW=1\r\nMV_PROVIDER=0\r\n";

        let updated = replace_ini_value(original, "", "Techniques", "B@B.fx,A@A.fx").expect("edit");

        let expected = original.replace(
            "Techniques=A@A.fx,B@B.fx\r\n",
            "Techniques=B@B.fx,A@A.fx\r\n",
        );
        assert_eq!(
            updated, expected,
            "only the Techniques line should have changed"
        );
        assert!(
            !updated.replace("\r\n", "").contains('\n'),
            "no bare LF should have been introduced: {updated:?}"
        );
    }

    // ── plan_order_fix ───────────────────────────────────────────────────

    fn healthy_client(dir: &Path, preset_contents: &str) {
        write(dir, "eso64.exe", "");
        write(
            dir,
            "ReShade.ini",
            "[GENERAL]\nPresetPath=.\\ReShadePreset.ini\n",
        );
        write(dir, "ReShadePreset.ini", preset_contents);
        let shaders = dir.join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(shaders.join("MartysMods_LAUNCHPAD.fx"), "").unwrap();
        std::fs::write(shaders.join("DLSS5_Feed.fx"), "").unwrap();
    }

    #[test]
    fn plan_order_fix_moves_the_provider_before_the_feed() {
        let tmp = tempfile::tempdir().unwrap();
        let preset =
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n";
        healthy_client(tmp.path(), preset);
        let stack = inspect_stack(tmp.path());

        let fix = plan_order_fix(&stack, preset).expect("a fix should be offered");
        assert_eq!(fix.provider_technique, "MartysMods_Launchpad");
        assert_eq!(fix.feed_technique, "DLSS5_Feed");
        assert_eq!(
            fix.after,
            "MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx"
        );
    }

    #[test]
    fn plan_order_fix_preserves_other_techniques_relative_order() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        write(
            tmp.path(),
            "ReShade.ini",
            "[GENERAL]\nPresetPath=.\\ReShadePreset.ini\n",
        );
        let shaders = tmp.path().join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        for name in [
            "Daltonize.fx",
            "MartysMods_LAUNCHPAD.fx",
            "DLSS5_Feed.fx",
            "Vignette.fx",
        ] {
            std::fs::write(shaders.join(name), "").unwrap();
        }

        let preset = "Techniques=Daltonize@Daltonize.fx,DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,Vignette@Vignette.fx\n";
        write(tmp.path(), "ReShadePreset.ini", preset);
        let stack = inspect_stack(tmp.path());

        let fix = plan_order_fix(&stack, preset).expect("a fix should be offered");
        assert_eq!(
            fix.after,
            "Daltonize@Daltonize.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx,Vignette@Vignette.fx"
        );
    }

    /// The bug this feature was built after: the provider is not always
    /// LaunchPad, and the fix must move whatever `mv_provider.technique` names.
    #[test]
    fn plan_order_fix_handles_a_shared_texture_provider_under_a_different_name() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "eso64.exe", "");
        write(
            tmp.path(),
            "ReShade.ini",
            "[GENERAL]\nPresetPath=.\\ReShadePreset.ini\n",
        );
        let shaders = tmp.path().join("reshade-shaders").join("Shaders");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(shaders.join("DLSS5_Feed.fx"), "").unwrap();
        std::fs::write(
            shaders.join("MotionEstimation.fx"),
            "texture texMotionVectors { Format = RG16F; };\n",
        )
        .unwrap();

        let preset = "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MotionEstimation@MotionEstimation.fx\n\n[DLSS5_Feed.fx]\nMV_PROVIDER=1\n";
        write(tmp.path(), "ReShadePreset.ini", preset);
        let stack = inspect_stack(tmp.path());

        let fix = plan_order_fix(&stack, preset).expect("a fix should be offered");
        assert_eq!(fix.provider_technique, "MotionEstimation");
        assert_eq!(fix.feed_technique, "DLSS5_Feed");
        assert_eq!(
            fix.after,
            "MotionEstimation@MotionEstimation.fx,DLSS5_Feed@DLSS5_Feed.fx"
        );
    }

    #[test]
    fn plan_order_fix_is_none_when_the_order_is_already_right() {
        let tmp = tempfile::tempdir().unwrap();
        let preset =
            "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx,DLSS5_Feed@DLSS5_Feed.fx\n";
        healthy_client(tmp.path(), preset);
        let stack = inspect_stack(tmp.path());

        assert!(plan_order_fix(&stack, preset).is_none());
    }

    #[test]
    fn plan_order_fix_is_none_when_no_provider_is_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let preset = "Techniques=DLSS5_Feed@DLSS5_Feed.fx\n\n[DLSS5_Feed.fx]\nMV_PROVIDER=1\n";
        healthy_client(tmp.path(), preset);
        let stack = inspect_stack(tmp.path());

        assert!(plan_order_fix(&stack, preset).is_none());
    }

    #[test]
    fn plan_order_fix_is_none_without_the_feed_technique() {
        let tmp = tempfile::tempdir().unwrap();
        let preset = "Techniques=MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n";
        healthy_client(tmp.path(), preset);
        let stack = inspect_stack(tmp.path());

        assert!(plan_order_fix(&stack, preset).is_none());
    }

    // ── end to end ───────────────────────────────────────────────────────

    /// Apply the fix through the same write path the command uses, then
    /// re-inspect the stack and confirm the ordering finding is gone.
    #[test]
    fn applying_the_fix_clears_the_stack_technique_order_finding() {
        let tmp = tempfile::tempdir().unwrap();
        let client = tmp.path().join("client");
        std::fs::create_dir_all(&client).unwrap();
        let manifest = tmp.path().join("client-managed.json");
        let backups = tmp.path().join("backups");
        std::fs::create_dir_all(&backups).unwrap();

        let preset =
            "Techniques=DLSS5_Feed@DLSS5_Feed.fx,MartysMods_Launchpad@MartysMods_LAUNCHPAD.fx\n";
        healthy_client(&client, preset);

        let before = inspect_stack(&client);
        assert!(
            before
                .findings
                .iter()
                .any(|f| f.id == "stack-technique-order"),
            "fixture should start misordered"
        );

        let fix = plan_order_fix(&before, preset).expect("a fix should be offered");
        let updated = replace_ini_value(preset, "", "Techniques", &fix.after).expect("rewrite");

        let root = ApprovedRoot::for_tests_idle(client.clone());
        edit_managed_file_in(
            &manifest,
            &backups,
            &root,
            "ReShadePreset.ini",
            ManagedKind::Preset,
            updated.as_bytes(),
        )
        .expect("edit should succeed");

        let after = inspect_stack(&client);
        assert!(
            !after
                .findings
                .iter()
                .any(|f| f.id == "stack-technique-order"),
            "expected the ordering finding to be gone, got {:?}",
            after.findings
        );
    }
}
