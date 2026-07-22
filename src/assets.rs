use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};
use std::{collections::hash_map::DefaultHasher, hash::Hasher};

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "vendor/zed"]
#[include = "file_icons/**/*"]
#[include = "icons/**/*"]
pub struct Assets;

#[derive(RustEmbed)]
#[folder = "vendor/font"]
#[include = "*.ttf"]
pub struct FontAssets;

static EXTERNAL_ASSETS: OnceLock<RwLock<HashMap<String, PathBuf>>> = OnceLock::new();

fn external_assets() -> &'static RwLock<HashMap<String, PathBuf>> {
    EXTERNAL_ASSETS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn register_external_asset(path: impl Into<String>, actual_path: PathBuf) -> SharedString {
    let path = path.into();
    if let Ok(mut guard) = external_assets().write() {
        guard.insert(path.clone(), actual_path);
    }
    SharedString::from(path)
}

pub fn register_external_asset_path(actual_path: PathBuf) -> SharedString {
    let mut hasher = DefaultHasher::new();
    hasher.write(actual_path.to_string_lossy().as_bytes());
    let key = format!("external/{:016x}.svg", hasher.finish());
    register_external_asset(key, actual_path)
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = Assets::get(path) {
            return Ok(Some(asset.data));
        }

        let Some(actual_path) = external_assets()
            .read()
            .ok()
            .and_then(|assets| assets.get(path).cloned())
        else {
            return Ok(None);
        };

        Ok(Some(Cow::Owned(std::fs::read(actual_path)?)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = if path.is_empty() {
            String::new()
        } else if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };

        let mut entries = BTreeSet::new();
        for asset_path in Assets::iter() {
            let asset_path = asset_path.as_ref();
            if !asset_path.starts_with(&prefix) {
                continue;
            }

            let remainder = &asset_path[prefix.len()..];
            if remainder.is_empty() {
                continue;
            }

            if let Some(entry) = remainder.split('/').next()
                && !entry.is_empty()
            {
                entries.insert(entry.to_string());
            }
        }

        if let Ok(assets) = external_assets().read() {
            for asset_path in assets.keys() {
                if !asset_path.starts_with(&prefix) {
                    continue;
                }

                let remainder = &asset_path[prefix.len()..];
                if remainder.is_empty() {
                    continue;
                }

                if let Some(entry) = remainder.split('/').next()
                    && !entry.is_empty()
                {
                    entries.insert(entry.to_string());
                }
            }
        }

        Ok(entries.into_iter().map(SharedString::from).collect())
    }
}

impl FontAssets {
    pub fn load_fonts(cx: &App) -> Result<()> {
        let mut embedded_fonts = Vec::new();

        for font_path in Self::iter() {
            if !font_path.ends_with(".ttf") {
                continue;
            }

            let Some(font_bytes) = Self::get(&font_path) else {
                continue;
            };

            embedded_fonts.push(font_bytes.data);
        }

        cx.text_system().add_fonts(embedded_fonts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_icons_all_exist() {
        for icon in [
            "icons/chevron_down.svg",
            "icons/chevron_right.svg",
            "icons/check.svg",
            "icons/file_multiple.svg",
        ] {
            assert!(Assets::get(icon).is_some(), "missing embedded icon: {icon}");
        }
    }

    #[test]
    fn missing_icon_is_none() {
        assert!(Assets::get("icons/nonexistent.svg").is_none());
    }

    // `list` reports the immediate child entry names under a prefix (one level
    // deep), deduped and sorted; it merges embedded and externally registered
    // assets. These exercise the prefix normalization and hierarchy folding.

    #[test]
    fn list_empty_prefix_returns_top_level_dirs() {
        let entries = Assets.list("").unwrap();
        // The embedded folders are `file_icons` and `icons` (see the RustEmbed
        // include globs). Only the first path segment is surfaced.
        assert!(entries.iter().any(|e| e.as_ref() == "icons"));
        assert!(entries.iter().any(|e| e.as_ref() == "file_icons"));
        // Nested paths collapse to their first segment — no slashes leak out.
        assert!(entries.iter().all(|e| !e.contains('/')));
    }

    #[test]
    fn list_normalizes_missing_trailing_slash() {
        // A prefix without a trailing slash resolves the same as one with it.
        let without = Assets.list("icons").unwrap();
        let with = Assets.list("icons/").unwrap();
        assert_eq!(without, with);
        assert!(without.iter().any(|e| e.as_ref() == "check.svg"));
    }

    #[test]
    fn list_is_sorted_and_deduped() {
        let entries = Assets.list("icons/").unwrap();
        let mut sorted = entries.clone();
        sorted.sort();
        assert_eq!(entries, sorted, "entries come back sorted");
        let mut unique = entries.clone();
        unique.dedup();
        assert_eq!(entries, unique, "entries come back deduped");
    }

    #[test]
    fn list_merges_external_assets() {
        // Registering an external asset under a fresh prefix makes it visible to
        // `list` alongside the embedded entries.
        register_external_asset(
            "listtest/generated.svg",
            PathBuf::from("/tmp/does-not-need-to-exist.svg"),
        );
        let entries = Assets.list("listtest/").unwrap();
        assert!(entries.iter().any(|e| e.as_ref() == "generated.svg"));
    }

    #[test]
    fn list_unknown_prefix_is_empty() {
        let entries = Assets.list("no_such_prefix/").unwrap();
        assert!(entries.is_empty());
    }
}
