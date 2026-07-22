use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

fn default_sync_zed_settings() -> bool {
    true
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FontConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buffer_size: Option<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThemeConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_variant: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_row: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover_row: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_primary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_secondary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_dim: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_highlight: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_highlight_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_gutter_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_line_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor_active_line_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_muted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_line_bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_modified: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_deleted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_conflict: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_renamed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_untracked: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ignored: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    #[serde(default)]
    pub editor: String,
    #[serde(default = "default_sync_zed_settings", alias = "sync-zed-settings")]
    pub sync_zed_settings: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_keybind: Option<String>,
    #[serde(default, alias = "base_dir", skip_serializing_if = "Option::is_none")]
    pub base_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub follow_symlinks: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_height: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker_pane_width: Option<f32>,
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            editor: String::new(),
            sync_zed_settings: true,
            global_keybind: None,
            base_path: None,
            exclude_dirs: Vec::new(),
            follow_symlinks: false,
            window_width: None,
            window_height: None,
            picker_pane_width: None,
            font: FontConfig::default(),
            theme: ThemeConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: PathBuf,
    pub config: AppConfig,
}

// Resolve the user's home directory, falling back to the filesystem root when
// `HOME` is unset. Shared across `main.rs` and `theme.rs` so path resolution
// (config, base path, Zed config dirs) agrees on one home.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_path() -> PathBuf {
    resolve_config_path(std::env::var_os("XDG_CONFIG_HOME"), home_dir())
}

// Pure resolution split out of `config_path` so the XDG_CONFIG_HOME-vs-`~/.config`
// precedence is unit-testable without mutating (racy) process-global env vars:
// an explicit `XDG_CONFIG_HOME` wins, otherwise fall back under `home`.
fn resolve_config_path(xdg_config_home: Option<std::ffi::OsString>, home: PathBuf) -> PathBuf {
    match xdg_config_home {
        Some(xdg) => PathBuf::from(xdg).join("fff-gpui/config.toml"),
        None => home.join(".config/fff-gpui/config.toml"),
    }
}

fn config_parent(path: &Path) -> Option<PathBuf> {
    path.parent().map(PathBuf::from)
}

pub fn load_active_config() -> Result<LoadedConfig> {
    let path = config_path();

    if !path.exists() {
        ensure_config_file(&path)?;
    }

    load_config_from(&path)
        .with_context(|| format!("failed to load config from {}", path.display()))
}

pub fn load_config_from(path: &Path) -> Result<LoadedConfig> {
    let contents = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let mut config = toml::from_str::<AppConfig>(&contents)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;
    if config
        .global_keybind
        .as_deref()
        .is_some_and(|binding| binding.trim().is_empty())
    {
        config.global_keybind = None;
    }
    config
        .exclude_dirs
        .retain(|path| !path.as_os_str().is_empty());
    config.window_width = sanitize_dimension(config.window_width, "window_width");
    config.window_height = sanitize_dimension(config.window_height, "window_height");
    config.picker_pane_width = sanitize_dimension(config.picker_pane_width, "picker_pane_width");
    Ok(LoadedConfig {
        path: path.to_path_buf(),
        config,
    })
}

// A px sizing override is usable only when finite and strictly positive.
// Shared with `theme::sync_from_config` so both validate identically.
pub fn is_valid_dimension(px: f32) -> bool {
    px.is_finite() && px > 0.0
}

// Validate an optional px sizing override: keep finite positive values,
// drop anything else back to `None` (viewport-relative sizing).
fn sanitize_dimension(value: Option<f32>, key: &str) -> Option<f32> {
    match value {
        Some(px) if !is_valid_dimension(px) => {
            warn!(value = px, "invalid {key} in config; ignoring override");
            None
        }
        other => other,
    }
}

const DEFAULT_CONFIG: &str = "\
sync_zed_settings = true
editor = \"\"
global_keybind = \"\"
";

pub fn ensure_config_file(path: &Path) -> Result<()> {
    if let Some(parent) = config_parent(path) {
        fs::create_dir_all(&parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }

    if path.exists() {
        return Ok(());
    }

    fs::write(path, DEFAULT_CONFIG)
        .with_context(|| format!("failed to write config file {}", path.display()))?;
    info!(path = %path.display(), "wrote default config");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unique per-test scratch directory so the file-backed `load_config_from`
    // tests never collide when run in parallel.
    fn temp_config_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fff-gpui-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn follow_symlinks_defaults_to_false() {
        assert!(!AppConfig::default().follow_symlinks);
        // Absent from the file, serde falls back to the default.
        let config: AppConfig = toml::from_str("editor = \"\"").unwrap();
        assert!(!config.follow_symlinks);
    }

    #[test]
    fn follow_symlinks_parses_from_toml() {
        let config: AppConfig = toml::from_str("follow_symlinks = true").unwrap();
        assert!(config.follow_symlinks);
    }

    #[test]
    fn base_path_parses_from_canonical_and_alias_spellings() {
        // The canonical spelling used in the config.
        let canonical: AppConfig = toml::from_str("base_path = \"~/Developer\"").unwrap();
        assert_eq!(canonical.base_path.as_deref(), Some("~/Developer"));

        // The legacy `base_dir` spelling still maps onto `base_path` via the alias.
        let alias: AppConfig = toml::from_str("base_dir = \"~/Developer\"").unwrap();
        assert_eq!(alias.base_path.as_deref(), Some("~/Developer"));
    }

    #[test]
    fn sizing_keys_absent_parse_as_none() {
        let default = AppConfig::default();
        assert_eq!(default.window_width, None);
        assert_eq!(default.window_height, None);
        assert_eq!(default.picker_pane_width, None);

        let config: AppConfig = toml::from_str("editor = \"\"").unwrap();
        assert_eq!(config.window_width, None);
        assert_eq!(config.window_height, None);
        assert_eq!(config.picker_pane_width, None);
    }

    #[test]
    fn sizing_keys_present_parse_as_overrides() {
        let config: AppConfig = toml::from_str(
            "window_width = 960.0\nwindow_height = 520.0\npicker_pane_width = 430.0",
        )
        .unwrap();
        assert_eq!(config.window_width, Some(960.0));
        assert_eq!(config.window_height, Some(520.0));
        assert_eq!(config.picker_pane_width, Some(430.0));
    }

    #[test]
    fn sanitize_dimension_keeps_valid_values_and_none() {
        assert_eq!(
            sanitize_dimension(Some(430.0), "picker_pane_width"),
            Some(430.0)
        );
        assert_eq!(sanitize_dimension(None, "window_width"), None);
    }

    #[test]
    fn is_valid_dimension_matches_sanitize_rules() {
        // The predicate shared with theme::sync_from_config.
        assert!(is_valid_dimension(430.0));
        assert!(!is_valid_dimension(0.0));
        assert!(!is_valid_dimension(-1.0));
        assert!(!is_valid_dimension(f32::NAN));
        assert!(!is_valid_dimension(f32::INFINITY));
    }

    #[test]
    fn sanitize_dimension_drops_invalid_values() {
        assert_eq!(sanitize_dimension(Some(0.0), "window_width"), None);
        assert_eq!(sanitize_dimension(Some(-100.0), "window_height"), None);
        assert_eq!(sanitize_dimension(Some(f32::NAN), "window_width"), None);
        assert_eq!(
            sanitize_dimension(Some(f32::INFINITY), "window_width"),
            None
        );
        assert_eq!(
            sanitize_dimension(Some(f32::NEG_INFINITY), "window_width"),
            None
        );
    }

    #[test]
    fn theme_config_parses_new_token_overrides() {
        let config: AppConfig = toml::from_str(
            "[theme]\n\
             active_line_bg = \"#111111\"\n\
             git_created = \"#222222\"\n\
             git_modified = \"#333333\"\n\
             git_deleted = \"#444444\"\n\
             git_conflict = \"#555555\"\n\
             git_renamed = \"#666666\"\n\
             git_untracked = \"#777777\"\n\
             git_ignored = \"#888888\"\n",
        )
        .unwrap();
        assert_eq!(config.theme.active_line_bg.as_deref(), Some("#111111"));
        assert_eq!(config.theme.git_created.as_deref(), Some("#222222"));
        assert_eq!(config.theme.git_modified.as_deref(), Some("#333333"));
        assert_eq!(config.theme.git_deleted.as_deref(), Some("#444444"));
        assert_eq!(config.theme.git_conflict.as_deref(), Some("#555555"));
        assert_eq!(config.theme.git_renamed.as_deref(), Some("#666666"));
        assert_eq!(config.theme.git_untracked.as_deref(), Some("#777777"));
        assert_eq!(config.theme.git_ignored.as_deref(), Some("#888888"));
    }

    #[test]
    fn theme_config_new_token_overrides_default_to_none() {
        let config: AppConfig = toml::from_str("editor = \"\"").unwrap();
        assert_eq!(config.theme.active_line_bg, None);
        assert_eq!(config.theme.git_created, None);
        assert_eq!(config.theme.git_modified, None);
        assert_eq!(config.theme.git_deleted, None);
        assert_eq!(config.theme.git_conflict, None);
        assert_eq!(config.theme.git_renamed, None);
        assert_eq!(config.theme.git_untracked, None);
        assert_eq!(config.theme.git_ignored, None);
    }

    #[test]
    fn theme_config_parses_zed_parity_token_overrides() {
        let config: AppConfig = toml::from_str(
            "[theme]\n\
             border_variant = \"#111111\"\n\
             text_accent = \"#222222\"\n\
             editor_gutter_bg = \"#333333\"\n\
             editor_line_number = \"#444444\"\n\
             editor_active_line_number = \"#555555\"\n\
             cursor = \"#666666\"\n",
        )
        .unwrap();
        assert_eq!(config.theme.border_variant.as_deref(), Some("#111111"));
        assert_eq!(config.theme.text_accent.as_deref(), Some("#222222"));
        assert_eq!(config.theme.editor_gutter_bg.as_deref(), Some("#333333"));
        assert_eq!(config.theme.editor_line_number.as_deref(), Some("#444444"));
        assert_eq!(
            config.theme.editor_active_line_number.as_deref(),
            Some("#555555")
        );
        assert_eq!(config.theme.cursor.as_deref(), Some("#666666"));
    }

    #[test]
    fn theme_config_ignores_removed_keys() {
        // `status_bar_bg`, `input_bg`, `cursor_selection`, and `icon_accent` were
        // removed from the `[theme]` surface. Serde stays tolerant of unknown
        // keys — the file still parses — but the values land nowhere: the parsed
        // ThemeConfig is indistinguishable from one that never mentioned them.
        let config: AppConfig = toml::from_str(
            "[theme]\n\
             status_bar_bg = \"#111111\"\n\
             input_bg = \"#222222\"\n\
             cursor_selection = \"#333333\"\n\
             icon_accent = \"#444444\"\n",
        )
        .unwrap();
        assert_eq!(config.theme, ThemeConfig::default());
    }

    #[test]
    fn default_config_template_parses_with_sizing_keys_none() {
        let dir = std::env::temp_dir().join(format!(
            "fff-gpui-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, DEFAULT_CONFIG).unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert!(loaded.config.sync_zed_settings);
        assert_eq!(loaded.config.window_width, None);
        assert_eq!(loaded.config.window_height, None);
        assert_eq!(loaded.config.picker_pane_width, None);
        // The template ships `global_keybind = ""`; normalization drops the
        // blank binding to None so no empty hotkey is registered.
        assert_eq!(loaded.config.global_keybind, None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_config_from_trims_whitespace_only_global_keybind_to_none() {
        let dir = temp_config_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "global_keybind = \"   \"\n").unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.config.global_keybind, None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_config_from_keeps_non_blank_global_keybind() {
        let dir = temp_config_dir();
        let path = dir.join("config.toml");
        fs::write(&path, "global_keybind = \"cmd-shift-f\"\n").unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.config.global_keybind.as_deref(), Some("cmd-shift-f"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_config_from_drops_empty_exclude_dirs_entries() {
        let dir = temp_config_dir();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "exclude_dirs = [\"\", \"node_modules\", \"\", \"target\"]\n",
        )
        .unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(
            loaded.config.exclude_dirs,
            vec![PathBuf::from("node_modules"), PathBuf::from("target")]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_config_path_prefers_xdg_config_home() {
        let path = resolve_config_path(
            Some(std::ffi::OsString::from("/xdg/config")),
            PathBuf::from("/home/user"),
        );
        assert_eq!(path, PathBuf::from("/xdg/config/fff-gpui/config.toml"));
    }

    #[test]
    fn resolve_config_path_falls_back_to_home_dotconfig() {
        let path = resolve_config_path(None, PathBuf::from("/home/user"));
        assert_eq!(
            path,
            PathBuf::from("/home/user/.config/fff-gpui/config.toml")
        );
    }

    #[test]
    fn ensure_config_file_writes_default_when_absent() {
        let dir = temp_config_dir();
        let path = dir.join("config.toml");
        assert!(!path.exists());

        ensure_config_file(&path).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ensure_config_file_leaves_existing_file_untouched() {
        let dir = temp_config_dir();
        let path = dir.join("config.toml");
        // A pre-existing file with custom content must survive the no-op branch
        // unmodified — `ensure_config_file` never clobbers user config.
        let existing = "editor = \"nvim\"\n";
        fs::write(&path, existing).unwrap();

        ensure_config_file(&path).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), existing);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_config_from_sanitizes_invalid_sizing_to_none() {
        let dir = std::env::temp_dir().join(format!(
            "fff-gpui-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "window_width = -5.0\nwindow_height = 700.0\npicker_pane_width = 0.0\n",
        )
        .unwrap();

        let loaded = load_config_from(&path).unwrap();
        assert_eq!(loaded.config.window_width, None);
        assert_eq!(loaded.config.window_height, Some(700.0));
        assert_eq!(loaded.config.picker_pane_width, None);

        fs::remove_dir_all(&dir).unwrap();
    }
}
