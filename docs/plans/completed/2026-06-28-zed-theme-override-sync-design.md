# Zed `theme_overrides` Sync — Design

**Status:** Approved design (brainstorm output). Implementation to follow via `/planning:make`.

## Problem

fff-gpui resolves the active Zed theme name and pulls palette colors + syntax styles
from its theme catalog, but it **ignores Zed's `theme_overrides`**. Users who tweak
their active theme via `theme_overrides` in `~/.config/zed/settings.json` see those
tweaks in Zed but not in the fff-gpui picker/preview.

Zed stores `theme_overrides` as a **map of theme name → partial style object**, where
each value has the same shape as a theme's `style` (palette color keys plus an optional
`syntax` block). Example (from the user's real settings):

```jsonc
"theme_overrides": {
  "RustRover Dark": {
    "editor.background": "#1e1f22",
    "border": "#1E1F22FF",
    "syntax": {
      "attribute": { "color": "#b3ae60ff", "font_style": null, "font_weight": null },
      "boolean":   { "color": "#cf8e6dff" }
    }
  },
  "New Darcula Darker": { "...": "..." }
}
```

## Decisions (from brainstorm Q&A)

- **Coverage:** mirror both palette colors AND syntax-token styles.
- **Gating:** reuse the existing `sync_zed_settings` flag — no new config toggle.
- **Precedence:** Zed theme → **Zed `theme_overrides`** → fff `[theme]` per-key config.
  fff config still wins last ("explicit config values win").
- **Key lookup:** overrides are looked up by the *finally-selected* theme name. If fff
  `[theme].name` forces a different theme, that theme's overrides apply (if any); the
  Zed theme's overrides do not bleed onto a different theme.
- **Syntax merge:** per-token — override only the listed tokens, keep the rest of the
  base theme's syntax styles.
- **Null handling:** an explicit `null` field is treated as "not overridden" (keep the
  base theme's value).
- **Approach:** A — store the raw `style` JSON in the catalog, deep-merge the override
  onto it, and re-derive palette/syntax through the existing resolution path. Avoids
  duplicating the Zed-key→field mapping.

## Architecture & Data Flow

**New input.** Add `theme_overrides: HashMap<String, Value>` to `ZedSettings`
(serde `default`). It deserializes directly from the existing `load_zed_settings()`
parse — no extra file read. Keys normalized via `normalize_name` into a lookup map.

**Catalog change.** `ThemeCatalogEntry` gains a `style: Value` field holding the
theme's raw style JSON (currently discarded after deriving palette/syntax). The derived
`palette` / `syntax_styles` / `syntax_default_color` stay for the common no-override
fast path.

**Resolution flow** (`sync_from_config`), new layer marked **→**:

1. `resolve_from_zed_settings` resolves Zed's active theme name (static or light/dark).
   When applying it, **→ if `theme_overrides` has an entry for that name, deep-merge it
   onto the base style and re-derive**; else apply the catalog entry as today. It now
   also returns the normalized overrides map.
2. If fff `[theme].name` is set, it *replaces* the theme — and **→ the same override
   step runs for that name**, so the finally-selected theme's overrides apply.
3. fff `[font]` config applied.
4. fff per-key `apply_color` overrides applied — **unchanged, still last**.

When `sync_zed_settings` is off, the overrides map is empty, so step 2 transparently
falls back to plain `apply_named_theme`.

## Components (all in `src/theme.rs`)

- `merge_json(base: &mut Value, overlay: &Value)` — recursive deep merge. Both objects →
  merge key-by-key (recursing); overlay value is `Null` → skip (keep base); otherwise
  overlay replaces base. Object-of-objects recursion yields per-token + per-field syntax
  merge for free.
- `apply_style_to_theme(style: &Value, theme: &mut AppTheme)` — derives palette +
  `syntax_styles` + `syntax_default_color` from a style `Value` and writes them onto the
  theme. Shared by catalog load and the override path (no duplicated mapping).
  `apply_catalog_entry` retained for the pre-derived fast path.
- `apply_theme_with_overrides(name, catalog, overrides, theme)` — look up the entry; if
  an override exists, `merge_json(base_style.clone(), override)` then
  `apply_style_to_theme`; otherwise `apply_catalog_entry`.
- `ThemeCatalogEntry.style` populated in `load_theme_family_contents`.
- `ZedSettings.theme_overrides` added (normalized into a lookup map).
- `resolve_from_zed_settings` returns the normalized map; `sync_from_config` threads it
  into the config-name branch.

## Error Handling

- Missing `theme_overrides` key → empty map (serde default).
- No override for the active theme → fast path, unchanged behavior.
- Override present but theme name not in catalog → existing "theme not found" warning;
  overrides skipped.
- Malformed values within an override (bad hex, wrong types) → already tolerated:
  `parse_color_rgb` returns `None`, `SyntaxStyle::from_value` ignores unknown fields, so
  bad keys fall back to the base theme's value. No new failure modes; nothing panics.

## Testing (unit tests, same module)

- `merge_json`: object deep-merge, null-skip-keeps-base, non-object replace, nested
  per-token syntax merge.
- Override changes `editor.background` + one syntax token → resolved theme reflects both;
  non-overridden tokens keep base colors.
- Key lookup: override for theme A does **not** apply when theme B is selected.
- Precedence: fff config per-key color still overrides a Zed override for the same field.
- No-override path and sync-off path preserve current behavior.

## Docs

- Update the README theme-sync section to note that `theme_overrides` for the active
  theme are mirrored (colors + syntax).
