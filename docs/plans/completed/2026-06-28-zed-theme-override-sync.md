# Zed `theme_overrides` Sync

> **For Claude:** use `/planning:execute` to implement this plan task-by-task with fresh subagents.

**Goal:** Mirror Zed's `theme_overrides` (palette colors + syntax styles) for the active theme into fff-gpui's picker/preview.

**Architecture:** Add `theme_overrides` (theme-name → partial style map) to `ZedSettings`, store each theme's raw `style` JSON in `ThemeCatalogEntry`, and when a theme is applied deep-merge its override onto the base style before re-deriving palette/syntax through the existing resolution path. Overrides sit below fff `[theme]` config (config still wins last) and are keyed by the finally-selected theme name.

**Tech Stack:** Rust (edition 2024), serde / serde_json (`Value`), json5, gpui.

**Design:** docs/plans/2026-06-28-zed-theme-override-sync-design.md

## Overview
- fff-gpui resolves the active Zed theme and pulls palette + syntax from its catalog, but **ignores `theme_overrides`** — user tweaks to the active theme don't reach the picker.
- Zed stores `theme_overrides` as `{ "<theme name>": { <partial style> } }`, each value shaped like a theme's `style` (color keys + optional `syntax` block).
- This change reads that map, deep-merges the entry for the active/selected theme onto the base style (per-token syntax merge, `null` = keep base), and re-derives the theme — reusing the existing `palette_from_style` / `syntax_styles_from_style` logic.
- Gated by the existing `sync_zed_settings` flag (no new toggle). fff `[theme]` per-key config overrides still apply last.

## Context (from discovery)
- files/components involved: `src/theme.rs` (all logic + tests), `README.md` (docs). No change needed in `src/config.rs`.
- related patterns found:
  - `ZedSettings` (theme.rs:217) deserialized via `load_zed_settings()` (json5).
  - `ThemeCatalogEntry` (theme.rs:670) currently holds derived `palette`, `syntax_styles`, `syntax_default_color`; raw `style` is discarded in `load_theme_family_contents` (theme.rs:1371).
  - `resolve_from_zed_settings` (theme.rs:957) applies the Zed theme via `apply_catalog_entry`; `sync_from_config` (theme.rs:737) then applies fff `[theme].name` via `apply_named_theme`, then per-key `apply_color`.
  - `palette_from_style` (theme.rs:1449), `syntax_styles_from_style` (theme.rs:1500), `parse_color_rgb` (theme.rs:1634) — robust to malformed values (return `None`).
  - Inline `#[cfg(test)] mod tests` at theme.rs:1661.
- dependencies identified: `serde_json::Value` already imported; `HashMap` already imported.

## Development Approach
- **testing approach**: TDD (tests first)
- complete each task fully before moving to the next
- make small, focused changes
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task
  - write unit tests for new functions/methods
  - write unit tests for modified functions/methods
  - add new test cases for new code paths
  - update existing test cases if behavior changes
  - tests cover both success and error scenarios
- **CRITICAL: all tests must pass before starting next task** - no exceptions
- **CRITICAL: update this plan file when scope changes during implementation**
- run tests after each change
- maintain backward compatibility (no-override and sync-off paths unchanged)

## Testing Strategy
- **unit tests**: required for every task — inline `#[cfg(test)] mod tests` in `src/theme.rs`, following the existing style (`syntax_style` helper, `AppTheme { ..Default::default() }` construction).
- **e2e tests**: none — this project has no UI e2e harness. Behavior is validated through unit tests on the pure resolution functions.

## Progress Tracking
- mark completed items with `[x]` immediately when done
- add newly discovered tasks with ➕ prefix
- document issues/blockers with ⚠️ prefix
- update plan if implementation deviates from original scope

## Solution Overview
- **Deep-merge + re-derive (Approach A):** keep the raw `style` `Value` per catalog entry; on an override hit, `merge_json` the override onto a clone of the base style, then resolve palette/syntax from the merged style. Avoids duplicating the Zed-key→field mapping.
- **Precedence:** Zed theme → Zed `theme_overrides` → fff `[theme]` per-key config (unchanged `apply_color` stays last).
- **Key lookup:** by the finally-applied theme name. fff `[theme].name` selecting a different theme uses that theme's override (if any); the Zed theme's override does not bleed across.

## Technical Details
- **`merge_json(base: &mut Value, overlay: &Value)`** — recursive: both objects → merge key-by-key (recurse); overlay `Null` → skip (keep base); else overlay replaces base. Object-of-objects recursion yields per-token + per-field syntax merge for free and satisfies "ignore null = keep base".
- **`ThemeCatalogEntry.style: Value`** — raw theme style, populated in `load_theme_family_contents`.
- **`apply_style_to_theme(style: &Value, theme: &mut AppTheme)`** — derives palette (via `palette_from_style`) + `syntax_styles` (via `syntax_styles_from_style`) + `syntax_default_color`, writing onto the theme. Shares an extracted `apply_palette(&Palette, &mut AppTheme)` helper with `apply_catalog_entry`.
- **`ZedSettings.theme_overrides: HashMap<String, Value>`** (serde `default`) — normalized (`normalize_name`) into a lookup map for case/whitespace-insensitive matching.
- **`apply_theme_with_overrides(name, catalog, overrides, theme)`** — entry lookup; if an override exists, merge then `apply_style_to_theme`; else `apply_catalog_entry`.
- **Threading:** `resolve_from_zed_settings` returns the normalized overrides map as a 3rd tuple element and applies the Zed-theme override; `sync_from_config` reuses the map for the `[theme].name` branch. Sync off → empty map → transparent fallback to `apply_named_theme`.

## What Goes Where
- **Implementation Steps** (checkboxes): all code + tests in `src/theme.rs`, README docs.
- **Post-Completion**: manual visual check of the running overlay against the real Zed config.

## Implementation Steps

### Task 1: Add `merge_json` deep-merge helper

**Files:**
- Modify: `src/theme.rs`

**Step 1: Write the failing tests** (in `#[cfg(test)] mod tests`)
```rust
#[test]
fn merge_json_deep_merges_objects() {
    let mut base = serde_json::json!({ "a": 1, "nested": { "x": 1, "y": 2 } });
    let overlay = serde_json::json!({ "b": 2, "nested": { "y": 9, "z": 3 } });
    merge_json(&mut base, &overlay);
    assert_eq!(base, serde_json::json!({ "a": 1, "b": 2, "nested": { "x": 1, "y": 9, "z": 3 } }));
}

#[test]
fn merge_json_null_overlay_keeps_base() {
    let mut base = serde_json::json!({ "font_style": "italic", "color": "#fff" });
    let overlay = serde_json::json!({ "font_style": null });
    merge_json(&mut base, &overlay);
    assert_eq!(base, serde_json::json!({ "font_style": "italic", "color": "#fff" }));
}

#[test]
fn merge_json_non_object_overlay_replaces() {
    let mut base = serde_json::json!({ "color": "#000" });
    let overlay = serde_json::json!({ "color": "#fff" });
    merge_json(&mut base, &overlay);
    assert_eq!(base, serde_json::json!({ "color": "#fff" }));
}

#[test]
fn merge_json_adds_missing_object_key() {
    let mut base = serde_json::json!({ "syntax": { "keyword": { "color": "#111" } } });
    let overlay = serde_json::json!({ "syntax": { "boolean": { "color": "#222" } } });
    merge_json(&mut base, &overlay);
    assert_eq!(
        base,
        serde_json::json!({ "syntax": { "keyword": { "color": "#111" }, "boolean": { "color": "#222" } } })
    );
}
```

**Step 2: Run tests to verify they fail**
Run: `cargo test --lib theme::tests::merge_json`
Expected: FAIL — `merge_json` not defined

**Step 3: Write minimal implementation**
```rust
fn merge_json(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (key, overlay_value) in overlay_map {
                if overlay_value.is_null() {
                    continue; // null = keep base
                }
                match base_map.get_mut(key) {
                    Some(base_value) => merge_json(base_value, overlay_value),
                    None => {
                        base_map.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (base_slot, overlay_value) if !overlay_value.is_null() => {
            *base_slot = overlay_value.clone();
        }
        _ => {}
    }
}
```
Note: when `base_map.get_mut(key)` recurses with a non-object base and object overlay, the catch-all replace arm handles it. Verify the top-level "adds missing object key" + nested cases pass; adjust the match arms if a non-object base needs replacing by an object overlay (it should replace).

**Step 4: Run tests to verify they pass**
Run: `cargo test --lib theme::tests::merge_json`
Expected: PASS

- [ ] write the four `merge_json` failing tests
- [ ] verify they fail (function undefined)
- [ ] implement `merge_json`
- [ ] verify all four pass
- [ ] run `cargo test --lib theme::` — must pass before Task 2

### Task 2: Store raw `style` in catalog + `apply_style_to_theme`

**Files:**
- Modify: `src/theme.rs`

**Step 1: Write the failing tests**
```rust
#[test]
fn apply_style_to_theme_sets_palette_and_syntax() {
    let style = serde_json::json!({
        "background": "#101010",
        "border": "#202020",
        "editor.foreground": "#fafafa",
        "syntax": { "keyword": { "color": "#abcdef" } }
    });
    let mut theme = AppTheme::default();
    apply_style_to_theme(&style, &mut theme);
    assert_eq!(theme.bg, 0x101010);
    assert_eq!(theme.border, 0x202020);
    assert_eq!(theme.syntax_default_color, 0xfafafa);
    assert_eq!(theme.syntax_color("keyword"), 0xabcdef);
}

#[test]
fn catalog_entries_retain_raw_style() {
    let catalog = load_theme_catalog().expect("catalog loads");
    let entry = catalog.get("one dark").expect("one dark present");
    assert!(entry.style.is_object());
    assert!(entry.style.get("syntax").is_some());
}
```

**Step 2: Run tests to verify they fail**
Run: `cargo test --lib theme::tests::apply_style_to_theme theme::tests::catalog_entries_retain_raw_style`
Expected: FAIL — `apply_style_to_theme` undefined / `ThemeCatalogEntry` has no `style` field

**Step 3: Write minimal implementation**
- Add `style: Value` to `ThemeCatalogEntry` (theme.rs:670).
- In `load_theme_family_contents` (theme.rs:1371), set `style: variant.style.clone()` when constructing the entry (palette/syntax still derived from `&variant.style`).
- Extract `fn apply_palette(palette: &Palette, theme: &mut AppTheme)` from the body of `apply_catalog_entry` (theme.rs:1407) and have `apply_catalog_entry` call it.
- Add:
```rust
fn apply_style_to_theme(style: &Value, theme: &mut AppTheme) {
    apply_palette(&palette_from_style(style), theme);
    theme.syntax_styles = syntax_styles_from_style(style);
    theme.syntax_default_color = color_from_style(style, "editor.foreground")
        .or_else(|| color_from_style(style, "text"))
        .unwrap_or(DEFAULT_TEXT_PRIMARY);
}
```

**Step 4: Run tests to verify they pass**
Run: `cargo test --lib theme::`
Expected: PASS (including existing `builtin_theme_catalog_includes_zed_themes`)

- [ ] add `style: Value` field to `ThemeCatalogEntry` and populate it in `load_theme_family_contents`
- [ ] extract `apply_palette` helper shared by `apply_catalog_entry`
- [ ] add `apply_style_to_theme`
- [ ] write tests for `apply_style_to_theme` + raw-style retention
- [ ] verify fail → implement → verify pass
- [ ] run `cargo test --lib theme::` — must pass before Task 3

### Task 3: `theme_overrides` field, `apply_theme_with_overrides`, and threading

**Files:**
- Modify: `src/theme.rs`

**Step 1: Write the failing tests**
```rust
fn one_dark_catalog() -> HashMap<String, ThemeCatalogEntry> {
    load_theme_catalog().expect("catalog loads")
}

#[test]
fn override_merges_color_and_syntax_onto_base() {
    let catalog = one_dark_catalog();
    let mut overrides = HashMap::new();
    overrides.insert(
        normalize_name("One Dark"),
        serde_json::json!({
            "background": "#123456",
            "syntax": { "keyword": { "color": "#0f0f0f" } }
        }),
    );
    let mut theme = AppTheme::default();
    apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
    assert_eq!(theme.bg, 0x123456);                 // overridden color
    assert_eq!(theme.syntax_color("keyword"), 0x0f0f0f); // overridden token
    // a non-overridden token still comes from the base One Dark theme (not default)
    assert_ne!(theme.syntax_color("string"), DEFAULT_TEXT_PRIMARY);
}

#[test]
fn override_for_other_theme_does_not_apply() {
    let catalog = one_dark_catalog();
    let mut overrides = HashMap::new();
    overrides.insert(normalize_name("Ayu Dark"), serde_json::json!({ "background": "#123456" }));
    let mut theme = AppTheme::default();
    apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
    assert_ne!(theme.bg, 0x123456); // One Dark selected; Ayu override must NOT apply
}

#[test]
fn no_override_matches_plain_catalog_application() {
    let catalog = one_dark_catalog();
    let empty = HashMap::new();
    let mut with_helper = AppTheme::default();
    apply_theme_with_overrides("One Dark", &catalog, &empty, &mut with_helper);
    let mut plain = AppTheme::default();
    apply_named_theme("One Dark", &catalog, &mut plain);
    assert_eq!(with_helper.bg, plain.bg);
    assert_eq!(with_helper.syntax_default_color, plain.syntax_default_color);
}

#[test]
fn zed_settings_parses_theme_overrides() {
    let settings: ZedSettings = json5::from_str(
        r#"{ "theme": "One Dark", "theme_overrides": { "One Dark": { "background": "#123456" } } }"#,
    )
    .expect("parses");
    assert!(settings.theme_overrides.contains_key("One Dark"));
}
```
Also add a precedence assertion (fff per-key config wins): exercised in Task 4's full-suite verification via existing `sync_from_config` path, or add a focused test if a unit seam exists. If no seam, note it and rely on Task 4 manual check.

**Step 2: Run tests to verify they fail**
Run: `cargo test --lib theme::`
Expected: FAIL — `apply_theme_with_overrides` undefined / `ZedSettings` has no `theme_overrides`

**Step 3: Write minimal implementation**
- Add `#[serde(default)] theme_overrides: HashMap<String, Value>` to `ZedSettings` (theme.rs:217). Leave `merge_zed_settings` unchanged (empty default is fine).
- Add:
```rust
fn apply_theme_with_overrides(
    name: &str,
    catalog: &HashMap<String, ThemeCatalogEntry>,
    overrides: &HashMap<String, Value>,
    theme: &mut AppTheme,
) {
    let Some(entry) = catalog.get(&normalize_name(name)) else {
        warn!(theme = %name, "theme not found in catalog");
        return;
    };
    if let Some(override_style) = overrides.get(&normalize_name(name)) {
        let mut merged = entry.style.clone();
        merge_json(&mut merged, override_style);
        apply_style_to_theme(&merged, theme);
    } else {
        apply_catalog_entry(entry, theme);
    }
}
```
- Build a normalized overrides map from `settings.theme_overrides` (keys via `normalize_name`).
- Change `resolve_from_zed_settings` to return `(AppTheme, FileIconTheme, HashMap<String, Value>)`; apply the Zed-resolved theme via `apply_theme_with_overrides(&name, catalog, &overrides, &mut theme)` instead of the inline `apply_catalog_entry`; return the normalized map (empty when no catalog).
- In `sync_from_config` (theme.rs:737): capture the overrides map from `resolve_from_zed_settings`; in the `config.theme.name` branch (theme.rs:784) call `apply_theme_with_overrides(name, catalog, &overrides, &mut resolved)` instead of `apply_named_theme`. Per-key `apply_color` block stays last (unchanged). Sync-off branch uses an empty map.

**Step 4: Run tests to verify they pass**
Run: `cargo test --lib theme::`
Expected: PASS

- [ ] add `theme_overrides` to `ZedSettings`
- [ ] implement `apply_theme_with_overrides`
- [ ] normalize overrides map and return it from `resolve_from_zed_settings`
- [ ] wire override application into Zed-theme + `config.theme.name` branches; keep `apply_color` last
- [ ] write tests (merge, key-lookup isolation, no-override parity, settings parse)
- [ ] verify fail → implement → verify pass
- [ ] run `cargo test --lib theme::` — must pass before Task 4

### Task 4: Verify acceptance criteria
- [ ] verify colors + syntax overrides for the active theme are mirrored (Task 3 tests green)
- [ ] verify precedence: fff `[theme]` per-key color overrides a Zed override for the same field (add/confirm a `sync_from_config`-level check or document the manual verification)
- [ ] verify key lookup: override keyed to a non-selected theme does not apply
- [ ] verify sync-off and no-override paths preserve current behavior
- [ ] run full suite: `cargo test`
- [ ] run `cargo clippy --all-targets` — no new warnings
- [ ] run `cargo fmt --check`

### Task 5: [Final] Update documentation
- [ ] update `README.md` theme-sync section (around line 96) to note that Zed `theme_overrides` for the active theme are mirrored (colors + syntax), applied below explicit fff config
- [ ] update `CLAUDE.md` only if a new pattern warrants it (likely skip)
- [ ] `mkdir -p docs/plans/completed`
- [ ] move this plan to `docs/plans/completed/`
- [ ] move the linked design file too: extract via `grep -E '^\*\*Design:\*\*' <plan-file> | sed 's/^\*\*Design:\*\* *//'`; if non-empty and not `none`, `test -f <design-path>` and `mv <design-path> docs/plans/completed/` (warn + continue if missing)

## Post-Completion
*Items requiring manual intervention or external systems — no checkboxes, informational only*

**Manual verification:**
- Run the overlay against the real `~/.config/zed/settings.json` (active theme `RustRover Dark`, which has live `theme_overrides`) and confirm the picker background/border and the `attribute` / `boolean` syntax colors match Zed.
- Toggle `sync_zed_settings = false` and confirm overrides no longer apply.
