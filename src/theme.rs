use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    OnceLock, RwLock,
    atomic::{AtomicU64, Ordering},
};

use anyhow::{Context as _, Result};
use gpui::{App, Global, SharedString, WindowAppearance};
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, warn};

use crate::assets::register_external_asset_path;
use crate::config::{AppConfig, ThemeConfig, home_dir};

// All colors are stored as 0xRRGGBBAA and painted via `rgba(...)`.
//
// Zed-parity resolution: every palette token resolves from its OWN theme key,
// else the per-appearance STATIC default below — never from another key's
// resolved value. Zed merges missing theme keys from the static
// `ThemeColors::dark()`/`light()` tables via `Refineable` with no inter-key
// inheritance, so a theme that flattens `editor.background` but never authors
// `elevated_surface.background` still renders a two-tone picker there. The
// tables are ported from the Zed checkout's
// `crates/theme/src/default_colors.rs`; the color scales live in the same file
// (`neutral()` = sand, and `step_N()` is one-based: step_2 = array index 1).

/// Which half of Zed's static default table a theme resolves against. Theme
/// JSON variants carry `"appearance": "dark" | "light"`; absent or
/// unrecognized values fall back to `Dark`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Appearance {
    Dark,
    Light,
}

fn appearance_from_json(value: Option<&str>) -> Appearance {
    match value {
        Some(value) if value.eq_ignore_ascii_case("light") => Appearance::Light,
        _ => Appearance::Dark,
    }
}

/// Zed's static per-appearance defaults for every theme key fff consumes,
/// with resolved RGBA hexes extracted from `ThemeColors::dark()`/`light()` in
/// zed `crates/theme/src/default_colors.rs` (line references below). Values
/// from the `dark_alpha()`/`light_alpha()` scales genuinely carry alpha and
/// blend live at paint time.
struct ZedDefaults {
    elevated_surface_background: u32,
    editor_background: u32,
    editor_gutter_background: u32,
    border: u32,
    border_variant: u32,
    ghost_element_selected: u32,
    ghost_element_hover: u32,
    text: u32,
    text_muted: u32,
    text_placeholder: u32,
    text_accent: u32,
    icon_muted: u32,
    search_match_background: u32,
    editor_active_line_background: u32,
    editor_line_number: u32,
    editor_active_line_number: u32,
    editor_foreground: u32,
    version_control_added: u32,
    version_control_modified: u32,
    version_control_deleted: u32,
    version_control_renamed: u32,
    version_control_conflict: u32,
    version_control_ignored: u32,
}

// `ThemeColors::dark()` (default_colors.rs:202-330), resolved through the
// sand/blue/orange/gray scales defined in the same file.
const ZED_DARK_DEFAULTS: ZedDefaults = ZedDefaults {
    elevated_surface_background: 0x191918FF, // L212 neutral().dark().step_2()
    editor_background: 0x111110FF,           // L268 neutral().dark().step_1()
    editor_gutter_background: 0x111110FF,    // L269 neutral().dark().step_1()
    border: 0x3B3A37FF,                      // L206 neutral().dark().step_6()
    border_variant: 0x31312EFF,              // L207 neutral().dark().step_5()
    ghost_element_selected: 0xFBFBEB23,      // L226 neutral().dark_alpha().step_5()
    ghost_element_hover: 0xFEFEF31B,         // L224 neutral().dark_alpha().step_4()
    text: 0xEEEEECFF,                        // L228 neutral().dark().step_12()
    text_muted: 0xB5B3ADFF,                  // L229 neutral().dark().step_11()
    text_placeholder: 0x7C7B74FF,            // L230 neutral().dark().step_10()
    text_accent: 0x70B8FFFF,                 // L232 blue().dark().step_11()
    icon_muted: 0x7C7B74FF,                  // L234 neutral().dark().step_10()
    search_match_background: 0x31312EFF,     // L246 neutral().dark().step_5()
    editor_active_line_background: 0xF6F6F513, // L271 neutral().dark_alpha().step_3()
    editor_line_number: 0xFFFDEE73,          // L274 neutral().dark_alpha().step_10()
    editor_active_line_number: 0xFFFCF4B0,   // L276 neutral().dark_alpha().step_11()
    editor_foreground: 0xEEEEECFF,           // L267 neutral().dark().step_12()
    version_control_added: 0x2E9E48FF,       // L321 ADDED_COLOR hsla(134°,.55,.40)
    version_control_modified: 0xD3AF1DFF,    // L323 MODIFIED_COLOR hsla(48°,.76,.47)
    version_control_deleted: 0x78081AFF,     // L322 REMOVED_COLOR hsla(350°,.88,.25)
    version_control_renamed: 0xD3AF1DFF,     // L324 MODIFIED_COLOR
    version_control_conflict: 0xFFE0C2FF,    // L325 orange().dark().step_12()
    version_control_ignored: 0xEEEEEEFF,     // L326 gray().dark().step_12()
};

// `ThemeColors::light()` (default_colors.rs:49-177). Note the light table uses
// the PLAIN neutral scale for line numbers (opaque), unlike the dark table.
const ZED_LIGHT_DEFAULTS: ZedDefaults = ZedDefaults {
    elevated_surface_background: 0xF9F9F8FF, // L59 neutral().light().step_2()
    editor_background: 0xFDFDFCFF,           // L115 neutral().light().step_1()
    editor_gutter_background: 0xFDFDFCFF,    // L116 neutral().light().step_1()
    border: 0xDAD9D6FF,                      // L53 neutral().light().step_6()
    border_variant: 0xE2E1DEFF,              // L54 neutral().light().step_5()
    ghost_element_selected: 0x1F180021,      // L73 neutral().light_alpha().step_5()
    ghost_element_hover: 0x20100010,         // L71 neutral().light_alpha().step_3()
    text: 0x21201CFF,                        // L75 neutral().light().step_12()
    text_muted: 0x82827CFF,                  // L76 neutral().light().step_10()
    text_placeholder: 0x82827CFF,            // L77 neutral().light().step_10()
    text_accent: 0x0D74CEFF,                 // L79 blue().light().step_11()
    icon_muted: 0x82827CFF,                  // L81 neutral().light().step_10()
    search_match_background: 0xE2E1DEFF,     // L93 neutral().light().step_5()
    editor_active_line_background: 0x20100010, // L118 neutral().light_alpha().step_3()
    editor_line_number: 0x82827CFF,          // L121 neutral().light().step_10()
    editor_active_line_number: 0x63635EFF,   // L123 neutral().light().step_11()
    editor_foreground: 0x21201CFF,           // L114 neutral().light().step_12()
    version_control_added: 0x2E9E48FF,       // L168 ADDED_COLOR hsla(134°,.55,.40)
    version_control_modified: 0xD3AF1DFF,    // L170 MODIFIED_COLOR hsla(48°,.76,.47)
    version_control_deleted: 0x78081AFF,     // L169 REMOVED_COLOR hsla(350°,.88,.25)
    version_control_renamed: 0xD3AF1DFF,     // L171 MODIFIED_COLOR
    version_control_conflict: 0x582D1DFF,    // L172 orange().light().step_12()
    version_control_ignored: 0x202020FF,     // L173 gray().light().step_12()
};

const fn zed_defaults(appearance: Appearance) -> &'static ZedDefaults {
    match appearance {
        Appearance::Dark => &ZED_DARK_DEFAULTS,
        Appearance::Light => &ZED_LIGHT_DEFAULTS,
    }
}

// Untracked has NO Zed default (Zed's `version_control_*` table lacks it, and
// its status color remaps untracked to created, which would lose the distinct
// purple). fff keeps its own appearance-independent hex.
const DEFAULT_GIT_UNTRACKED: u32 = 0xA48EFFFF;
const DEFAULT_UI_FONT_FAMILY: &str = ".SystemUIFont";
const DEFAULT_BUFFER_FONT_FAMILY: &str = "UbuntuMono Nerd Font";
pub const DEFAULT_UI_FONT_SIZE: f32 = 16.0;
pub const DEFAULT_BUFFER_FONT_SIZE: f32 = 15.0;
static ACTIVE_THEME: OnceLock<RwLock<AppTheme>> = OnceLock::new();
static ACTIVE_FILE_ICON_THEME: OnceLock<RwLock<FileIconTheme>> = OnceLock::new();
static THEME_VERSION: AtomicU64 = AtomicU64::new(1);

/// Replace the alpha byte of an `0xRRGGBBAA` color with `alpha`, keeping the RGB
/// bytes. Masks the old alpha off rather than shifting: the pre-RGBA `<< 8` form
/// silently corrupted colors once the palette started carrying a real alpha byte
/// (a nonzero top byte would leak into RGB). Blended live at paint time.
pub const fn with_alpha(base: u32, alpha: u8) -> u32 {
    (base & 0xFFFF_FF00) | alpha as u32
}

/// Force `0xRRGGBBAA` fully opaque. The full-bleed surface tokens (`bg`,
/// `preview_bg`, `editor_gutter_bg`) paint edge-to-edge over the opaque OS
/// window; a translucent authored value (config override or third-party Zed
/// theme) would render the picker partially see-through — visibly broken
/// against the design's opaque edge-to-edge window. Clamp their alpha byte at
/// resolution time so only the blend-at-paint tokens stay translucent.
const fn opaque(value: u32) -> u32 {
    with_alpha(value, 0xFF)
}

/// Adding a color token means touching NINE synchronized places. A `pub` field
/// going unused never trips `dead_code`, so nothing here fails at compile time —
/// the `palette_round_trips_every_color_field` test is the field-swap safety net:
///   1. `Palette` field (below)         2. `AppTheme` field
///   3. `impl Default for Palette`       4. `impl Default for AppTheme`
///   5. `palette_from_theme` getter      6. `apply_palette`
///   7. `palette_from_style`             8. `apply_color` line in `apply_theme_config_colors`
///   9. `ThemeConfig` in `config.rs`
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub bg: u32,
    pub border: u32,
    pub border_variant: u32,
    pub selected_row: u32,
    pub hover_row: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_dim: u32,
    pub text_accent: u32,
    pub match_highlight: u32,
    pub match_highlight_bg: u32,
    pub preview_bg: u32,
    pub editor_gutter_bg: u32,
    pub editor_line_number: u32,
    pub editor_active_line_number: u32,
    pub input_text: u32,
    pub cursor: u32,
    pub icon_muted: u32,
    pub active_line_bg: u32,
    pub git_created: u32,
    pub git_modified: u32,
    pub git_deleted: u32,
    pub git_conflict: u32,
    pub git_renamed: u32,
    pub git_untracked: u32,
    pub git_ignored: u32,
    pub picker_pane_width: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppTheme {
    pub bg: u32,
    pub border: u32,
    pub border_variant: u32,
    pub selected_row: u32,
    pub hover_row: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_dim: u32,
    pub text_accent: u32,
    pub match_highlight: u32,
    pub match_highlight_bg: u32,
    pub preview_bg: u32,
    pub editor_gutter_bg: u32,
    pub editor_line_number: u32,
    pub editor_active_line_number: u32,
    pub input_text: u32,
    pub cursor: u32,
    pub icon_muted: u32,
    // Populated by the theme pipeline and consumed by the picker: `active_line_bg`
    // by the preview active-line highlight, the `git_*` tokens by the per-row git
    // status edge bars.
    pub active_line_bg: u32,
    pub git_created: u32,
    pub git_modified: u32,
    pub git_deleted: u32,
    pub git_conflict: u32,
    pub git_renamed: u32,
    pub git_untracked: u32,
    pub git_ignored: u32,
    pub ui_font_family: Option<String>,
    pub buffer_font_family: Option<String>,
    pub ui_font_size: f32,
    pub buffer_font_size: f32,
    pub picker_pane_width: Option<f32>,
    pub syntax_styles: Vec<(String, SyntaxStyle)>,
    pub syntax_default_color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyntaxRenderStyle {
    pub color: u32,
    pub bg: Option<u32>,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for Palette {
    // When no theme applies at all, the palette IS Zed's dark default table —
    // exactly what `palette_from_style` resolves for an empty style (the
    // `default_palette_is_the_zed_dark_table` test pins that equivalence).
    fn default() -> Self {
        let d = &ZED_DARK_DEFAULTS;
        Self {
            bg: d.elevated_surface_background,
            border: d.border,
            border_variant: d.border_variant,
            selected_row: d.ghost_element_selected,
            hover_row: d.ghost_element_hover,
            text_primary: d.text,
            text_secondary: d.text_muted,
            text_dim: d.text_placeholder,
            text_accent: d.text_accent,
            // `match_highlight` follows `text.accent` by design.
            match_highlight: d.text_accent,
            match_highlight_bg: d.search_match_background,
            preview_bg: d.editor_background,
            editor_gutter_bg: d.editor_gutter_background,
            editor_line_number: d.editor_line_number,
            editor_active_line_number: d.editor_active_line_number,
            // Fallback tracks `text_primary`: typed input text renders with
            // `input_text` (text_field.rs), so this keeps default rendering
            // identical to the pre-wiring `text_primary` path.
            input_text: d.text,
            cursor: d.text_accent,
            icon_muted: d.icon_muted,
            active_line_bg: d.editor_active_line_background,
            git_created: d.version_control_added,
            git_modified: d.version_control_modified,
            git_deleted: d.version_control_deleted,
            git_conflict: d.version_control_conflict,
            git_renamed: d.version_control_renamed,
            git_untracked: DEFAULT_GIT_UNTRACKED,
            git_ignored: d.version_control_ignored,
            picker_pane_width: None,
        }
    }
}

impl Default for AppTheme {
    // Color tokens mirror `Palette::default()` (Zed's dark default table).
    fn default() -> Self {
        let d = &ZED_DARK_DEFAULTS;
        Self {
            bg: d.elevated_surface_background,
            border: d.border,
            border_variant: d.border_variant,
            selected_row: d.ghost_element_selected,
            hover_row: d.ghost_element_hover,
            text_primary: d.text,
            text_secondary: d.text_muted,
            text_dim: d.text_placeholder,
            text_accent: d.text_accent,
            // `match_highlight` follows `text.accent` by design.
            match_highlight: d.text_accent,
            match_highlight_bg: d.search_match_background,
            preview_bg: d.editor_background,
            editor_gutter_bg: d.editor_gutter_background,
            editor_line_number: d.editor_line_number,
            editor_active_line_number: d.editor_active_line_number,
            // Fallback tracks `text_primary`: typed input text renders with
            // `input_text` (text_field.rs), so this keeps default rendering
            // identical to the pre-wiring `text_primary` path.
            input_text: d.text,
            cursor: d.text_accent,
            icon_muted: d.icon_muted,
            active_line_bg: d.editor_active_line_background,
            git_created: d.version_control_added,
            git_modified: d.version_control_modified,
            git_deleted: d.version_control_deleted,
            git_conflict: d.version_control_conflict,
            git_renamed: d.version_control_renamed,
            git_untracked: DEFAULT_GIT_UNTRACKED,
            git_ignored: d.version_control_ignored,
            ui_font_family: Some(DEFAULT_UI_FONT_FAMILY.to_string()),
            buffer_font_family: Some(DEFAULT_BUFFER_FONT_FAMILY.to_string()),
            ui_font_size: DEFAULT_UI_FONT_SIZE,
            buffer_font_size: DEFAULT_BUFFER_FONT_SIZE,
            picker_pane_width: None,
            syntax_styles: Vec::new(),
            syntax_default_color: d.editor_foreground,
        }
    }
}

impl Global for AppTheme {}

impl AppTheme {
    fn syntax_color(&self, capture_name: &str) -> u32 {
        if syntax_capture_is_punctuation(capture_name) {
            return self.syntax_default_color;
        }

        if syntax_capture_uses_variable_color(capture_name) {
            return syntax_color_from_styles(
                &self.syntax_styles,
                "variable",
                self.syntax_default_color,
            );
        }

        syntax_color_from_styles(&self.syntax_styles, capture_name, self.syntax_default_color)
    }

    fn syntax_render_style(&self, capture_name: &str) -> SyntaxRenderStyle {
        if syntax_capture_is_punctuation(capture_name) {
            return SyntaxRenderStyle {
                color: self.syntax_default_color,
                ..Default::default()
            };
        }

        let resolved_name = if syntax_capture_uses_variable_color(capture_name) {
            "variable"
        } else {
            capture_name
        };

        let style = syntax_style_for_capture(&self.syntax_styles, resolved_name);
        SyntaxRenderStyle {
            color: syntax_style_color(&style).unwrap_or(self.syntax_default_color),
            bg: syntax_style_bg(&style),
            italic: matches!(style.font_style.as_deref(), Some("italic")),
            bold: matches!(style.font_style.as_deref(), Some("bold"))
                || style.font_weight.is_some_and(|w| w >= 600.0),
            underline: matches!(style.font_style.as_deref(), Some("underline")),
            strikethrough: matches!(style.font_style.as_deref(), Some("strikethrough")),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
enum ThemeMode {
    Dark,
    Light,
    #[default]
    System,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum ThemeSelection {
    Static(String),
    Dynamic {
        #[serde(default)]
        mode: ThemeMode,
        light: String,
        dark: String,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ZedSettings {
    #[serde(default)]
    theme: Option<ThemeSelection>,
    #[serde(default)]
    icon_theme: Option<ThemeSelection>,
    #[serde(default)]
    ui_font_family: Option<String>,
    #[serde(default)]
    buffer_font_family: Option<String>,
    #[serde(default)]
    ui_font_size: Option<f32>,
    #[serde(default)]
    buffer_font_size: Option<f32>,
    #[serde(default)]
    theme_overrides: HashMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct IconThemeFamilyFile {
    #[serde(default)]
    themes: Vec<IconThemeVariant>,
}

#[derive(Debug, Clone, Deserialize)]
struct IconThemeVariant {
    name: String,
    #[serde(default)]
    file_stems: HashMap<String, String>,
    #[serde(default)]
    file_suffixes: HashMap<String, String>,
    #[serde(default)]
    file_icons: HashMap<String, IconDefinitionContent>,
}

#[derive(Debug, Clone, Deserialize)]
struct IconDefinitionContent {
    path: SharedString,
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeFamilyFile {
    #[serde(default)]
    themes: Vec<ThemeVariant>,
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeVariant {
    name: String,
    // Parsed leniently as a plain string (not an enum) so an unrecognized
    // appearance value degrades to Dark instead of failing the whole family
    // file — see `appearance_from_json`.
    #[serde(default)]
    appearance: Option<String>,
    #[serde(default)]
    style: Value,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ExtensionManifest {
    #[serde(default)]
    themes: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileIconPath {
    Embedded(SharedString),
    External(SharedString),
}

#[derive(Debug, Clone, PartialEq)]
struct FileIconDefinition {
    path: FileIconPath,
}

#[derive(Debug, Clone, PartialEq)]
struct FileIconTheme {
    file_stems: HashMap<String, String>,
    file_suffixes: HashMap<String, String>,
    file_icons: HashMap<String, FileIconDefinition>,
}

const BUILTIN_THEME_FAMILIES: &[(&str, &str)] = &[
    (
        "vendor/zed/themes/ayu/ayu.json",
        include_str!("../vendor/zed/themes/ayu/ayu.json"),
    ),
    (
        "vendor/zed/themes/gruvbox/gruvbox.json",
        include_str!("../vendor/zed/themes/gruvbox/gruvbox.json"),
    ),
    (
        "vendor/zed/themes/one/one.json",
        include_str!("../vendor/zed/themes/one/one.json"),
    ),
];

const FILE_STEMS_BY_ICON_KEY: &[(&str, &[&str])] = &[
    ("docker", &["Containerfile", "Dockerfile"]),
    ("ruby", &["Podfile"]),
    ("heroku", &["Procfile"]),
];

const FILE_SUFFIXES_BY_ICON_KEY: &[(&str, &[&str])] = &[
    ("astro", &["astro"]),
    (
        "audio",
        &[
            "aac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav", "wma", "wv",
        ],
    ),
    ("backup", &["bak"]),
    ("bicep", &["bicep"]),
    ("bun", &["lockb"]),
    ("c", &["c", "h"]),
    ("cairo", &["cairo"]),
    ("code", &["handlebars", "metadata", "rkt", "scm"]),
    ("coffeescript", &["coffee"]),
    (
        "cpp",
        &[
            "c++", "h++", "cc", "cpp", "cppm", "cxx", "hh", "hpp", "hxx", "inl", "ixx",
        ],
    ),
    ("crystal", &["cr", "ecr"]),
    ("csharp", &["cs"]),
    ("csproj", &["csproj"]),
    ("css", &["css", "pcss", "postcss"]),
    ("cue", &["cue"]),
    ("dart", &["dart"]),
    ("diff", &["diff"]),
    (
        "docker",
        &[
            "docker-compose.yml",
            "docker-compose.yaml",
            "compose.yml",
            "compose.yaml",
        ],
    ),
    (
        "document",
        &[
            "doc", "docx", "mdx", "odp", "ods", "odt", "pdf", "ppt", "pptx", "rtf", "txt", "xls",
            "xlsx",
        ],
    ),
    ("elixir", &["eex", "ex", "exs", "heex"]),
    ("elm", &["elm"]),
    (
        "erlang",
        &[
            "Emakefile",
            "app.src",
            "erl",
            "escript",
            "hrl",
            "rebar.config",
            "xrl",
            "yrl",
        ],
    ),
    (
        "eslint",
        &[
            "eslint.config.cjs",
            "eslint.config.cts",
            "eslint.config.js",
            "eslint.config.mjs",
            "eslint.config.mts",
            "eslint.config.ts",
            "eslintrc",
            "eslintrc.js",
            "eslintrc.json",
        ],
    ),
    ("font", &["otf", "ttf", "woff", "woff2"]),
    ("fsharp", &["fs"]),
    ("fsproj", &["fsproj"]),
    ("gitlab", &["gitlab-ci.yml", "gitlab-ci.yaml"]),
    ("gleam", &["gleam"]),
    ("go", &["go", "mod", "work"]),
    ("graphql", &["gql", "graphql", "graphqls"]),
    ("haskell", &["hs"]),
    ("hcl", &["hcl"]),
    (
        "helm",
        &[
            "helmfile.yaml",
            "helmfile.yml",
            "Chart.yaml",
            "Chart.yml",
            "Chart.lock",
            "values.yaml",
            "values.yml",
            "requirements.yaml",
            "requirements.yml",
            "tpl",
        ],
    ),
    ("html", &["htm", "html"]),
    (
        "image",
        &[
            "avif", "bmp", "gif", "heic", "heif", "ico", "j2k", "jfif", "jp2", "jpeg", "jpg",
            "jxl", "png", "psd", "qoi", "svg", "tiff", "webp",
        ],
    ),
    ("ipynb", &["ipynb"]),
    ("java", &["java"]),
    ("javascript", &["cjs", "js", "mjs"]),
    ("json", &["json", "jsonc"]),
    ("julia", &["jl"]),
    ("kdl", &["kdl"]),
    ("kotlin", &["kt"]),
    ("lock", &["lock"]),
    ("log", &["log"]),
    ("lua", &["lua"]),
    ("luau", &["luau"]),
    ("markdown", &["markdown", "md"]),
    ("metal", &["metal"]),
    ("nim", &["nim", "nims", "nimble"]),
    ("nix", &["nix"]),
    ("ocaml", &["ml", "mli"]),
    ("odin", &["odin"]),
    ("php", &["php"]),
    (
        "prettier",
        &[
            "prettier.config.cjs",
            "prettier.config.js",
            "prettier.config.mjs",
            "prettierignore",
            "prettierrc",
            "prettierrc.cjs",
            "prettierrc.js",
            "prettierrc.json",
            "prettierrc.json5",
            "prettierrc.mjs",
            "prettierrc.toml",
            "prettierrc.yaml",
            "prettierrc.yml",
        ],
    ),
    ("prisma", &["prisma"]),
    ("puppet", &["pp"]),
    ("python", &["py"]),
    ("r", &["r", "R"]),
    ("react", &["cjsx", "ctsx", "jsx", "mjsx", "mtsx", "tsx"]),
    ("roc", &["roc"]),
    ("ruby", &["rb"]),
    ("rust", &["rs"]),
    ("sass", &["sass", "scss"]),
    ("scala", &["scala", "sc"]),
    ("settings", &["conf", "ini"]),
    ("solidity", &["sol"]),
    (
        "storage",
        &[
            "accdb", "csv", "dat", "db", "dbf", "dll", "fmp", "fp7", "frm", "gdb", "ib", "ldf",
            "mdb", "mdf", "myd", "myi", "pdb", "RData", "rdata", "sav", "sdf", "sql", "sqlite",
            "tsv",
        ],
    ),
    (
        "stylelint",
        &[
            "stylelint.config.cjs",
            "stylelint.config.js",
            "stylelint.config.mjs",
            "stylelintignore",
            "stylelintrc",
            "stylelintrc.cjs",
            "stylelintrc.js",
            "stylelintrc.json",
            "stylelintrc.mjs",
            "stylelintrc.yaml",
            "stylelintrc.yml",
        ],
    ),
    ("surrealql", &["surql"]),
    ("svelte", &["svelte"]),
    ("swift", &["swift"]),
    ("tcl", &["tcl"]),
    ("template", &["hbs", "plist", "xml"]),
    (
        "terminal",
        &[
            "bash",
            "bash_aliases",
            "bash_login",
            "bash_logout",
            "bash_profile",
            "bashrc",
            "fish",
            "nu",
            "profile",
            "ps1",
            "sh",
            "zlogin",
            "zlogout",
            "zprofile",
            "zsh",
            "zsh_aliases",
            "zsh_histfile",
            "zsh_history",
            "zshenv",
            "zshrc",
        ],
    ),
    ("terraform", &["tf", "tfvars"]),
    ("toml", &["toml"]),
    ("typescript", &["cts", "mts", "ts"]),
    ("v", &["v", "vsh", "vv"]),
    (
        "vcs",
        &[
            "COMMIT_EDITMSG",
            "EDIT_DESCRIPTION",
            "MERGE_MSG",
            "NOTES_EDITMSG",
            "TAG_EDITMSG",
            "gitattributes",
            "gitignore",
            "gitkeep",
            "gitmodules",
        ],
    ),
    ("vbproj", &["vbproj"]),
    ("video", &["avi", "m4v", "mkv", "mov", "mp4", "webm", "wmv"]),
    ("vs_sln", &["sln"]),
    ("vs_suo", &["suo"]),
    ("vue", &["vue"]),
    ("vyper", &["vy", "vyi"]),
    ("wgsl", &["wgsl"]),
    ("yaml", &["yaml", "yml"]),
    ("zig", &["zig"]),
];

const FILE_ICONS: &[(&str, &str)] = &[
    ("astro", "file_icons/astro.svg"),
    ("audio", "file_icons/audio.svg"),
    ("bicep", "file_icons/file.svg"),
    ("bun", "file_icons/bun.svg"),
    ("c", "file_icons/c.svg"),
    ("cairo", "file_icons/cairo.svg"),
    ("code", "file_icons/code.svg"),
    ("coffeescript", "file_icons/coffeescript.svg"),
    ("cpp", "file_icons/cpp.svg"),
    ("crystal", "file_icons/file.svg"),
    ("csharp", "file_icons/file.svg"),
    ("csproj", "file_icons/file.svg"),
    ("css", "file_icons/css.svg"),
    ("cue", "file_icons/file.svg"),
    ("dart", "file_icons/dart.svg"),
    ("default", "file_icons/file.svg"),
    ("diff", "file_icons/diff.svg"),
    ("docker", "file_icons/docker.svg"),
    ("document", "file_icons/book.svg"),
    ("elixir", "file_icons/elixir.svg"),
    ("elm", "file_icons/elm.svg"),
    ("erlang", "file_icons/erlang.svg"),
    ("eslint", "file_icons/eslint.svg"),
    ("font", "file_icons/font.svg"),
    ("fsharp", "file_icons/fsharp.svg"),
    ("fsproj", "file_icons/file.svg"),
    ("gitlab", "file_icons/gitlab.svg"),
    ("gleam", "file_icons/gleam.svg"),
    ("go", "file_icons/go.svg"),
    ("graphql", "file_icons/graphql.svg"),
    ("haskell", "file_icons/haskell.svg"),
    ("hcl", "file_icons/hcl.svg"),
    ("helm", "file_icons/helm.svg"),
    ("heroku", "file_icons/heroku.svg"),
    ("html", "file_icons/html.svg"),
    ("image", "file_icons/image.svg"),
    ("ipynb", "file_icons/jupyter.svg"),
    ("java", "file_icons/java.svg"),
    ("javascript", "file_icons/javascript.svg"),
    ("json", "file_icons/code.svg"),
    ("julia", "file_icons/julia.svg"),
    ("kdl", "file_icons/kdl.svg"),
    ("kotlin", "file_icons/kotlin.svg"),
    ("lock", "file_icons/lock.svg"),
    ("log", "file_icons/info.svg"),
    ("lua", "file_icons/lua.svg"),
    ("luau", "file_icons/luau.svg"),
    ("markdown", "file_icons/book.svg"),
    ("metal", "file_icons/metal.svg"),
    ("nim", "file_icons/nim.svg"),
    ("nix", "file_icons/nix.svg"),
    ("ocaml", "file_icons/ocaml.svg"),
    ("odin", "file_icons/odin.svg"),
    ("phoenix", "file_icons/phoenix.svg"),
    ("php", "file_icons/php.svg"),
    ("prettier", "file_icons/prettier.svg"),
    ("prisma", "file_icons/prisma.svg"),
    ("puppet", "file_icons/puppet.svg"),
    ("python", "file_icons/python.svg"),
    ("r", "file_icons/r.svg"),
    ("react", "file_icons/react.svg"),
    ("roc", "file_icons/roc.svg"),
    ("ruby", "file_icons/ruby.svg"),
    ("rust", "file_icons/rust.svg"),
    ("sass", "file_icons/sass.svg"),
    ("scala", "file_icons/scala.svg"),
    ("settings", "file_icons/settings.svg"),
    ("solidity", "file_icons/file.svg"),
    ("storage", "file_icons/database.svg"),
    ("stylelint", "file_icons/javascript.svg"),
    ("surrealql", "file_icons/surrealql.svg"),
    ("svelte", "file_icons/html.svg"),
    ("swift", "file_icons/swift.svg"),
    ("tcl", "file_icons/tcl.svg"),
    ("template", "file_icons/html.svg"),
    ("terminal", "file_icons/terminal.svg"),
    ("terraform", "file_icons/terraform.svg"),
    ("toml", "file_icons/toml.svg"),
    ("typescript", "file_icons/typescript.svg"),
    ("v", "file_icons/v.svg"),
    ("vbproj", "file_icons/file.svg"),
    ("vcs", "file_icons/git.svg"),
    ("video", "file_icons/video.svg"),
    ("vs_sln", "file_icons/file.svg"),
    ("vs_suo", "file_icons/file.svg"),
    ("vue", "file_icons/vue.svg"),
    ("vyper", "file_icons/vyper.svg"),
    ("wgsl", "file_icons/wgsl.svg"),
    ("yaml", "file_icons/yaml.svg"),
    ("zig", "file_icons/zig.svg"),
];

/// The name of the default icon theme.
const DEFAULT_ICON_THEME_NAME: &str = "Zed (Default)";

fn icon_keys_by_association(
    associations_by_icon_key: &[(&str, &[&str])],
) -> HashMap<String, String> {
    let mut icon_keys_by_association = HashMap::default();
    for (icon_key, associations) in associations_by_icon_key {
        for association in *associations {
            icon_keys_by_association.insert(association.to_string(), icon_key.to_string());
        }
    }

    icon_keys_by_association
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
pub struct SyntaxStyle {
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    background_color: Option<String>,
    #[serde(default)]
    font_style: Option<String>,
    #[serde(default)]
    font_weight: Option<f32>,
}

#[derive(Debug, Clone)]
struct ThemeCatalogEntry {
    style: Value,
    // Which static default table missing keys resolve against — kept on the
    // entry so the override path re-resolves the merged style with the same
    // appearance the base variant declared.
    appearance: Appearance,
    palette: Palette,
    syntax_styles: Vec<(String, SyntaxStyle)>,
    syntax_default_color: u32,
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

fn active_theme_lock() -> &'static RwLock<AppTheme> {
    ACTIVE_THEME.get_or_init(|| RwLock::new(AppTheme::default()))
}

pub fn current() -> AppTheme {
    match active_theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(_) => AppTheme::default(),
    }
}

pub fn palette() -> Palette {
    palette_from_theme(&current())
}

// AppTheme -> Palette copy, split out from `palette()` so the 27-field field-swap
// guard (`palette_round_trips_every_color_field`) can exercise it gpui-free.
fn palette_from_theme(theme: &AppTheme) -> Palette {
    Palette {
        bg: theme.bg,
        border: theme.border,
        border_variant: theme.border_variant,
        selected_row: theme.selected_row,
        hover_row: theme.hover_row,
        text_primary: theme.text_primary,
        text_secondary: theme.text_secondary,
        text_dim: theme.text_dim,
        text_accent: theme.text_accent,
        match_highlight: theme.match_highlight,
        match_highlight_bg: theme.match_highlight_bg,
        preview_bg: theme.preview_bg,
        editor_gutter_bg: theme.editor_gutter_bg,
        editor_line_number: theme.editor_line_number,
        editor_active_line_number: theme.editor_active_line_number,
        input_text: theme.input_text,
        cursor: theme.cursor,
        icon_muted: theme.icon_muted,
        active_line_bg: theme.active_line_bg,
        git_created: theme.git_created,
        git_modified: theme.git_modified,
        git_deleted: theme.git_deleted,
        git_conflict: theme.git_conflict,
        git_renamed: theme.git_renamed,
        git_untracked: theme.git_untracked,
        git_ignored: theme.git_ignored,
        picker_pane_width: theme.picker_pane_width,
    }
}

pub fn syntax_color(capture_name: &str) -> u32 {
    match active_theme_lock().read() {
        Ok(theme) => theme.syntax_color(capture_name),
        Err(_) => ZED_DARK_DEFAULTS.editor_foreground,
    }
}

pub fn syntax_render_style(capture_name: &str) -> SyntaxRenderStyle {
    match active_theme_lock().read() {
        Ok(theme) => theme.syntax_render_style(capture_name),
        Err(_) => SyntaxRenderStyle {
            color: ZED_DARK_DEFAULTS.editor_foreground,
            ..Default::default()
        },
    }
}

pub fn version() -> u64 {
    THEME_VERSION.load(Ordering::SeqCst)
}

pub fn sync_from_config(config: &AppConfig, appearance: WindowAppearance, cx: &mut App) {
    let need_theme_catalog = config.sync_zed_settings || config.theme.name.is_some();
    let theme_catalog = if need_theme_catalog {
        match load_theme_catalog() {
            Ok(catalog) => Some(catalog),
            Err(err) => {
                warn!(error = %err, "failed to load theme catalog; falling back to defaults");
                None
            }
        }
    } else {
        None
    };
    let icon_theme_catalog = if config.sync_zed_settings {
        match load_icon_theme_catalog() {
            Ok(catalog) => Some(catalog),
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to load icon theme catalog; falling back to defaults"
                );
                None
            }
        }
    } else {
        None
    };

    let (mut resolved, resolved_icon_theme, overrides) = if config.sync_zed_settings {
        match resolve_from_zed_settings(
            appearance,
            theme_catalog.as_ref(),
            icon_theme_catalog.as_ref(),
        ) {
            Ok(result) => result,
            Err(err) => {
                warn!(
                    error = %err,
                    "failed to sync Zed theme settings; falling back to defaults"
                );
                (
                    AppTheme::default(),
                    default_file_icon_theme(),
                    HashMap::new(),
                )
            }
        }
    } else {
        (
            AppTheme::default(),
            default_file_icon_theme(),
            HashMap::new(),
        )
    };

    if let Some(catalog) = theme_catalog.as_ref()
        && let Some(name) = config.theme.name.as_deref()
        && !name.trim().is_empty()
    {
        apply_theme_with_overrides(name, catalog, &overrides, &mut resolved);
    }

    if let Some(family) = resolve_optional_string(
        config.font.ui_family.as_deref(),
        config.font.family.as_deref(),
    ) {
        resolved.ui_font_family = Some(family);
    }
    if let Some(family) = resolve_optional_string(
        config.font.buffer_family.as_deref(),
        config.font.family.as_deref(),
    ) {
        resolved.buffer_font_family = Some(family);
    }
    if let Some(size) = resolve_optional_font_size(config.font.ui_size, config.font.size) {
        resolved.ui_font_size = size;
    }
    if let Some(size) = resolve_optional_font_size(config.font.buffer_size, config.font.size) {
        resolved.buffer_font_size = size;
    }
    // `None` means no px override: the picker resolves the pane width from the
    // viewport-relative layout math instead.
    resolved.picker_pane_width = config
        .picker_pane_width
        .filter(|width| crate::config::is_valid_dimension(*width));
    apply_theme_config_colors(&config.theme, &mut resolved);

    cx.set_global(resolved.clone());
    if let Ok(mut guard) = active_theme_lock().write() {
        *guard = resolved;
    }
    if let Ok(mut guard) = active_file_icon_theme_lock().write() {
        *guard = resolved_icon_theme;
    }
    THEME_VERSION.fetch_add(1, Ordering::SeqCst);

    refresh_windows(cx);
}

// Apply the fff `[theme]` config color overrides onto the Zed-resolved theme —
// the last precedence step (explicit config wins over Zed sync). Split out of
// `sync_from_config` so the field-by-field wiring is unit-testable gpui-free: a
// copy-paste field mismatch (e.g. writing `resolved.border` for the
// `border_variant` override) fails a table assertion in the tests below. The
// full-bleed surface tokens (`bg`, `preview_bg`, `editor_gutter_bg`) are
// re-clamped opaque after their override so a translucent config value can't
// punch a see-through hole in the edge-to-edge window.
fn apply_theme_config_colors(config: &ThemeConfig, resolved: &mut AppTheme) {
    apply_color(&config.bg, &mut resolved.bg);
    resolved.bg = opaque(resolved.bg);
    apply_color(&config.border, &mut resolved.border);
    apply_color(&config.border_variant, &mut resolved.border_variant);
    apply_color(&config.selected_row, &mut resolved.selected_row);
    apply_color(&config.hover_row, &mut resolved.hover_row);
    apply_color(&config.text_primary, &mut resolved.text_primary);
    apply_color(&config.text_secondary, &mut resolved.text_secondary);
    apply_color(&config.text_dim, &mut resolved.text_dim);
    apply_color(&config.text_accent, &mut resolved.text_accent);
    apply_color(&config.match_highlight, &mut resolved.match_highlight);
    apply_color(&config.match_highlight_bg, &mut resolved.match_highlight_bg);
    apply_color(&config.preview_bg, &mut resolved.preview_bg);
    resolved.preview_bg = opaque(resolved.preview_bg);
    apply_color(&config.editor_gutter_bg, &mut resolved.editor_gutter_bg);
    resolved.editor_gutter_bg = opaque(resolved.editor_gutter_bg);
    apply_color(&config.editor_line_number, &mut resolved.editor_line_number);
    apply_color(
        &config.editor_active_line_number,
        &mut resolved.editor_active_line_number,
    );
    apply_color(&config.input_text, &mut resolved.input_text);
    apply_color(&config.cursor, &mut resolved.cursor);
    apply_color(&config.icon_muted, &mut resolved.icon_muted);
    apply_color(&config.active_line_bg, &mut resolved.active_line_bg);
    apply_color(&config.git_created, &mut resolved.git_created);
    apply_color(&config.git_modified, &mut resolved.git_modified);
    apply_color(&config.git_deleted, &mut resolved.git_deleted);
    apply_color(&config.git_conflict, &mut resolved.git_conflict);
    apply_color(&config.git_renamed, &mut resolved.git_renamed);
    apply_color(&config.git_untracked, &mut resolved.git_untracked);
    apply_color(&config.git_ignored, &mut resolved.git_ignored);
}

fn refresh_windows(cx: &mut App) {
    for window in cx.windows() {
        let _ = window.update(cx, |_, window, _| {
            window.refresh();
        });
    }
}

fn zed_config_dir() -> PathBuf {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(config_home).join("zed")
    } else {
        home_dir().join(".config/zed")
    }
}

fn zed_settings_path() -> PathBuf {
    zed_config_dir().join("settings.json")
}

fn zed_local_themes_dir() -> PathBuf {
    zed_config_dir().join("themes")
}

fn zed_icon_themes_dir() -> PathBuf {
    zed_config_dir().join("icon_themes")
}

fn zed_installed_themes_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library/Application Support/Zed/extensions/installed")
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(data_home).join("zed/extensions/installed");
        }

        home_dir().join(".local/share/zed/extensions/installed")
    }
}

fn read_to_string(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
}

fn load_zed_settings() -> Result<ZedSettings> {
    let defaults = hardcoded_zed_settings_defaults();
    let path = zed_settings_path();
    if !path.exists() {
        return Ok(defaults);
    }

    let contents = read_to_string(&path)?;
    if contents.trim().is_empty() {
        return Ok(defaults);
    }

    let mut settings: ZedSettings = json5::from_str(&contents)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))?;
    merge_zed_settings(&mut settings, defaults);
    Ok(settings)
}

fn hardcoded_zed_settings_defaults() -> ZedSettings {
    ZedSettings {
        theme: Some(ThemeSelection::Dynamic {
            mode: ThemeMode::System,
            light: "One Light".to_string(),
            dark: "One Dark".to_string(),
        }),
        icon_theme: Some(ThemeSelection::Static("Zed (Default)".to_string())),
        ui_font_family: Some(DEFAULT_UI_FONT_FAMILY.to_string()),
        buffer_font_family: Some(DEFAULT_BUFFER_FONT_FAMILY.to_string()),
        ui_font_size: Some(DEFAULT_UI_FONT_SIZE),
        buffer_font_size: Some(DEFAULT_BUFFER_FONT_SIZE),
        theme_overrides: HashMap::new(),
    }
}

fn merge_zed_settings(settings: &mut ZedSettings, defaults: ZedSettings) {
    if settings.theme.is_none() {
        settings.theme = defaults.theme;
    }
    if settings.icon_theme.is_none() {
        settings.icon_theme = defaults.icon_theme;
    }
    if settings.ui_font_family.is_none() {
        settings.ui_font_family = defaults.ui_font_family;
    }
    if settings.buffer_font_family.is_none() {
        settings.buffer_font_family = defaults.buffer_font_family;
    }
    if settings.ui_font_size.is_none() {
        settings.ui_font_size = defaults.ui_font_size;
    }
    if settings.buffer_font_size.is_none() {
        settings.buffer_font_size = defaults.buffer_font_size;
    }
    // `theme_overrides` is intentionally not defaults-merged: the hardcoded
    // defaults carry an empty overrides map, so there is nothing to merge in and
    // the user's parsed overrides (already keyed via `#[serde(default)]`) stand
    // on their own.
}

fn resolve_from_zed_settings(
    appearance: WindowAppearance,
    theme_catalog: Option<&HashMap<String, ThemeCatalogEntry>>,
    icon_theme_catalog: Option<&HashMap<String, FileIconTheme>>,
) -> Result<(AppTheme, FileIconTheme, HashMap<String, Value>)> {
    let settings = load_zed_settings()?;
    let overrides = normalized_theme_overrides(&settings.theme_overrides);

    let mut theme = AppTheme::default();
    if let Some(name) = settings
        .theme
        .as_ref()
        .map(|theme| resolve_theme_name(theme, appearance))
    {
        if let Some(catalog) = theme_catalog {
            apply_theme_with_overrides(&name, catalog, &overrides, &mut theme);
        }
    } else {
        debug!(
            settings_path = %zed_settings_path().display(),
            "no Zed theme configured; using built-in fallback theme"
        );
    }

    apply_font_settings(&settings, &mut theme);

    if let Some(icon_name) = settings
        .icon_theme
        .as_ref()
        .map(|selection| resolve_theme_name(selection, appearance))
    {
        let resolved_icon_theme = icon_theme_catalog
            .and_then(|catalog| catalog.get(&normalize_name(&icon_name)).cloned())
            .unwrap_or_else(|| {
                warn!(
                    icon_theme = %icon_name,
                    "Zed icon theme not found; using built-in default icons"
                );
                default_file_icon_theme()
            });
        Ok((theme, resolved_icon_theme, overrides))
    } else {
        debug!(
            settings_path = %zed_settings_path().display(),
            "no Zed icon theme configured; using built-in default icons"
        );
        Ok((theme, default_file_icon_theme(), overrides))
    }
}

fn apply_font_settings(settings: &ZedSettings, theme: &mut AppTheme) {
    theme.ui_font_family = Some(
        resolve_optional_string(settings.ui_font_family.as_deref(), None)
            .unwrap_or_else(|| DEFAULT_UI_FONT_FAMILY.to_string()),
    );
    theme.buffer_font_family = Some(
        resolve_optional_string(settings.buffer_font_family.as_deref(), None)
            .unwrap_or_else(|| DEFAULT_BUFFER_FONT_FAMILY.to_string()),
    );
    theme.ui_font_size =
        resolve_optional_font_size(settings.ui_font_size, None).unwrap_or(DEFAULT_UI_FONT_SIZE);
    theme.buffer_font_size = resolve_optional_font_size(settings.buffer_font_size, None)
        .unwrap_or(DEFAULT_BUFFER_FONT_SIZE);
}

fn resolve_theme_name(selection: &ThemeSelection, appearance: WindowAppearance) -> String {
    match selection {
        ThemeSelection::Static(name) => name.clone(),
        ThemeSelection::Dynamic { mode, light, dark } => match mode {
            ThemeMode::Light => light.clone(),
            ThemeMode::Dark => dark.clone(),
            ThemeMode::System => match appearance {
                WindowAppearance::Dark | WindowAppearance::VibrantDark => dark.clone(),
                WindowAppearance::Light | WindowAppearance::VibrantLight => light.clone(),
            },
        },
    }
}

fn active_file_icon_theme_lock() -> &'static RwLock<FileIconTheme> {
    ACTIVE_FILE_ICON_THEME.get_or_init(|| RwLock::new(default_file_icon_theme()))
}

fn default_file_icon_theme() -> FileIconTheme {
    let file_stems = icon_keys_by_association(FILE_STEMS_BY_ICON_KEY);
    let file_suffixes = icon_keys_by_association(FILE_SUFFIXES_BY_ICON_KEY);
    let file_icons = HashMap::from_iter(FILE_ICONS.iter().map(|(ty, path)| {
        (
            ty.to_string(),
            FileIconDefinition {
                path: FileIconPath::Embedded((*path).into()),
            },
        )
    }));

    FileIconTheme {
        file_stems,
        file_suffixes,
        file_icons,
    }
}

fn current_file_icon_theme() -> FileIconTheme {
    match active_file_icon_theme_lock().read() {
        Ok(theme) => theme.clone(),
        Err(_) => default_file_icon_theme(),
    }
}

pub fn file_icon_for_path(path: &Path) -> Option<FileIconPath> {
    let theme = current_file_icon_theme();
    theme.file_icon_for_path(path)
}

impl FileIconTheme {
    fn file_icon_for_path(&self, path: &Path) -> Option<FileIconPath> {
        let get_icon_from_suffix = |suffix: &str| -> Option<FileIconPath> {
            self.file_stems
                .get(suffix)
                .or_else(|| self.file_suffixes.get(suffix))
                .and_then(|typ| self.get_icon_for_type(typ))
        };

        if let Some(mut typ) = path.file_name().and_then(|typ| typ.to_str()) {
            let maybe_path = get_icon_from_suffix(typ);
            if maybe_path.is_some() {
                return maybe_path;
            }

            while let Some((_, suffix)) = typ.split_once('.') {
                let maybe_path = get_icon_from_suffix(suffix);
                if maybe_path.is_some() {
                    return maybe_path;
                }
                typ = suffix;
            }
        }

        if let Some(suffix) = multiple_extensions(path) {
            let maybe_path = get_icon_from_suffix(&suffix);
            if maybe_path.is_some() {
                return maybe_path;
            }
        }

        if let Some(suffix) = extension_or_hidden_file_name(path) {
            let maybe_path = get_icon_from_suffix(&suffix);
            if maybe_path.is_some() {
                return maybe_path;
            }
        }

        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            let maybe_path = get_icon_from_suffix(extension);
            if maybe_path.is_some() {
                return maybe_path;
            }
        }

        self.get_icon_for_type("default")
    }

    fn get_icon_for_type(&self, typ: &str) -> Option<FileIconPath> {
        self.file_icons
            .get(typ)
            .map(|icon_definition| icon_definition.path.clone())
    }
}

fn multiple_extensions(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let mut parts = file_name.split('.');
    let _ = parts.next()?;
    let _ = parts.next()?;
    let mut suffix = String::new();
    for part in file_name.split('.').skip(1) {
        if !suffix.is_empty() {
            suffix.push('.');
        }
        suffix.push_str(part);
    }
    (!suffix.is_empty()).then_some(suffix)
}

fn extension_or_hidden_file_name(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    if file_name.starts_with('.') && file_name.len() > 1 {
        let hidden = file_name.trim_start_matches('.');
        if !hidden.is_empty() {
            return Some(hidden.to_string());
        }
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(ToOwned::to_owned)
}

fn load_theme_catalog() -> Result<HashMap<String, ThemeCatalogEntry>> {
    let mut catalog = builtin_theme_catalog()?;
    load_installed_theme_catalog(&mut catalog)?;
    load_local_theme_catalog(&mut catalog)?;
    Ok(catalog)
}

// The vendored theme catalog only — no user/host directories are read, so it is
// deterministic regardless of the machine's Zed install. Production loading goes
// through `load_theme_catalog`, which merges the installed/local dirs on top;
// tests that assert vendored-theme parity use this to avoid host-state flake.
fn builtin_theme_catalog() -> Result<HashMap<String, ThemeCatalogEntry>> {
    let mut catalog = HashMap::new();
    load_builtin_theme_catalog(&mut catalog)?;
    Ok(catalog)
}

fn load_icon_theme_catalog() -> Result<HashMap<String, FileIconTheme>> {
    let mut catalog = HashMap::new();
    load_builtin_icon_theme_catalog(&mut catalog)?;
    load_installed_icon_theme_catalog(&mut catalog)?;
    load_local_icon_theme_catalog(&mut catalog)?;
    Ok(catalog)
}

fn load_builtin_icon_theme_catalog(catalog: &mut HashMap<String, FileIconTheme>) -> Result<()> {
    catalog.insert(
        normalize_name(DEFAULT_ICON_THEME_NAME),
        default_file_icon_theme(),
    );
    Ok(())
}

fn load_local_icon_theme_catalog(catalog: &mut HashMap<String, FileIconTheme>) -> Result<()> {
    let dir = zed_icon_themes_dir();
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading local Zed icon themes from {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        load_icon_theme_family_file(&path, &zed_config_dir(), catalog)?;
    }

    Ok(())
}

fn load_installed_icon_theme_catalog(catalog: &mut HashMap<String, FileIconTheme>) -> Result<()> {
    let dir = zed_installed_themes_dir();
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading installed Zed icon themes from {}", dir.display()))?
    {
        let entry = entry?;
        let extension_dir = entry.path();
        if !extension_dir.is_dir() {
            continue;
        }

        let icon_theme_dir = extension_dir.join("icon_themes");
        if !icon_theme_dir.exists() {
            continue;
        }

        for icon_theme_entry in fs::read_dir(&icon_theme_dir)
            .with_context(|| format!("reading icon themes from {}", icon_theme_dir.display()))?
        {
            let icon_theme_entry = icon_theme_entry?;
            let icon_theme_path = icon_theme_entry.path();
            if icon_theme_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            load_icon_theme_family_file(&icon_theme_path, &extension_dir, catalog)?;
        }
    }

    Ok(())
}

fn load_icon_theme_family_file(
    path: &Path,
    icons_root_dir: &Path,
    catalog: &mut HashMap<String, FileIconTheme>,
) -> Result<()> {
    let contents = read_to_string(path)?;
    load_icon_theme_family_contents(
        &path.display().to_string(),
        &contents,
        icons_root_dir,
        catalog,
    )
}

fn load_icon_theme_family_contents(
    label: &str,
    contents: &str,
    icons_root_dir: &Path,
    catalog: &mut HashMap<String, FileIconTheme>,
) -> Result<()> {
    let family: IconThemeFamilyFile =
        json5::from_str(contents).with_context(|| format!("failed to parse JSON from {label}"))?;

    for variant in family.themes {
        let theme_key = normalize_name(&variant.name);
        let mut theme = default_file_icon_theme();
        apply_icon_theme_variant(&variant, icons_root_dir, &mut theme);
        catalog.insert(theme_key, theme);
    }

    Ok(())
}

fn apply_icon_theme_variant(
    variant: &IconThemeVariant,
    icons_root_dir: &Path,
    theme: &mut FileIconTheme,
) {
    theme.file_stems.extend(variant.file_stems.clone());
    theme.file_suffixes.extend(variant.file_suffixes.clone());
    theme
        .file_icons
        .extend(variant.file_icons.iter().map(|(key, icon)| {
            let resolved_path = icons_root_dir.join(icon.path.as_ref());
            let asset_path = register_external_asset_path(resolved_path);
            (
                key.clone(),
                FileIconDefinition {
                    path: FileIconPath::External(asset_path),
                },
            )
        }));
}

fn load_builtin_theme_catalog(catalog: &mut HashMap<String, ThemeCatalogEntry>) -> Result<()> {
    for &(label, contents) in BUILTIN_THEME_FAMILIES {
        load_theme_family_contents(label, contents, catalog)?;
    }

    Ok(())
}

fn load_local_theme_catalog(catalog: &mut HashMap<String, ThemeCatalogEntry>) -> Result<()> {
    let dir = zed_local_themes_dir();
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading local Zed themes from {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        load_theme_family_file(&path, catalog)?;
    }

    Ok(())
}

fn load_installed_theme_catalog(catalog: &mut HashMap<String, ThemeCatalogEntry>) -> Result<()> {
    let dir = zed_installed_themes_dir();
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(&dir)
        .with_context(|| format!("reading installed Zed themes from {}", dir.display()))?
    {
        let entry = entry?;
        let extension_dir = entry.path();
        if !extension_dir.is_dir() {
            continue;
        }

        let manifest_path = extension_dir.join("extension.toml");
        if !manifest_path.exists() {
            continue;
        }

        let manifest: ExtensionManifest = match read_toml(&manifest_path) {
            Ok(manifest) => manifest,
            Err(err) => {
                warn!(error = %err, path = %manifest_path.display(), "skipping theme extension");
                continue;
            }
        };

        for relative_theme_path in manifest.themes {
            let theme_path = extension_dir.join(relative_theme_path);
            if let Err(err) = load_theme_family_file(&theme_path, catalog) {
                warn!(error = %err, path = %theme_path.display(), "failed to load installed Zed theme");
            }
        }
    }

    Ok(())
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let contents = read_to_string(path)?;
    toml::from_str(&contents)
        .with_context(|| format!("failed to parse TOML from {}", path.display()))
}

fn load_theme_family_file(
    path: &Path,
    catalog: &mut HashMap<String, ThemeCatalogEntry>,
) -> Result<()> {
    let contents = read_to_string(path)?;
    load_theme_family_contents(&path.display().to_string(), &contents, catalog)
}

fn load_theme_family_contents(
    label: &str,
    contents: &str,
    catalog: &mut HashMap<String, ThemeCatalogEntry>,
) -> Result<()> {
    let family: ThemeFamilyFile =
        json5::from_str(contents).with_context(|| format!("failed to parse JSON from {label}"))?;

    for variant in family.themes {
        let theme_key = normalize_name(&variant.name);
        let appearance = appearance_from_json(variant.appearance.as_deref());
        let entry = ThemeCatalogEntry {
            style: variant.style.clone(),
            appearance,
            palette: palette_from_style(&variant.style, appearance),
            syntax_styles: syntax_styles_from_style(&variant.style),
            syntax_default_color: color_from_style(&variant.style, "editor.foreground")
                .or_else(|| color_from_style(&variant.style, "text"))
                .unwrap_or(zed_defaults(appearance).editor_foreground),
        };

        catalog.insert(theme_key, entry);
    }

    Ok(())
}

fn apply_theme_with_overrides(
    name: &str,
    catalog: &HashMap<String, ThemeCatalogEntry>,
    overrides: &HashMap<String, Value>,
    theme: &mut AppTheme,
) {
    let key = normalize_name(name);
    let Some(entry) = catalog.get(&key) else {
        warn!(theme = %name, "theme not found in catalog");
        return;
    };
    if let Some(override_style) = overrides.get(&key) {
        let mut merged = entry.style.clone();
        merge_json(&mut merged, override_style);
        apply_style_to_theme(&merged, entry.appearance, theme);
    } else {
        apply_catalog_entry(entry, theme);
    }
}

fn normalized_theme_overrides(overrides: &HashMap<String, Value>) -> HashMap<String, Value> {
    overrides
        .iter()
        .map(|(name, style)| (normalize_name(name), style.clone()))
        .collect()
}

fn apply_palette(palette: &Palette, theme: &mut AppTheme) {
    // `picker_pane_width` is intentionally not copied here: it is owned by config
    // (resolved in `sync_from_config`), not the theme palette. Writing it would
    // clobber the config-resolved value with the palette's default.
    theme.bg = palette.bg;
    theme.border = palette.border;
    theme.border_variant = palette.border_variant;
    theme.selected_row = palette.selected_row;
    theme.hover_row = palette.hover_row;
    theme.text_primary = palette.text_primary;
    theme.text_secondary = palette.text_secondary;
    theme.text_dim = palette.text_dim;
    theme.text_accent = palette.text_accent;
    theme.match_highlight = palette.match_highlight;
    theme.match_highlight_bg = palette.match_highlight_bg;
    theme.preview_bg = palette.preview_bg;
    theme.editor_gutter_bg = palette.editor_gutter_bg;
    theme.editor_line_number = palette.editor_line_number;
    theme.editor_active_line_number = palette.editor_active_line_number;
    theme.input_text = palette.input_text;
    theme.cursor = palette.cursor;
    theme.icon_muted = palette.icon_muted;
    theme.active_line_bg = palette.active_line_bg;
    theme.git_created = palette.git_created;
    theme.git_modified = palette.git_modified;
    theme.git_deleted = palette.git_deleted;
    theme.git_conflict = palette.git_conflict;
    theme.git_renamed = palette.git_renamed;
    theme.git_untracked = palette.git_untracked;
    theme.git_ignored = palette.git_ignored;
}

fn apply_catalog_entry(entry: &ThemeCatalogEntry, theme: &mut AppTheme) {
    apply_palette(&entry.palette, theme);
    theme.syntax_styles = entry.syntax_styles.clone();
    theme.syntax_default_color = entry.syntax_default_color;
}

fn apply_style_to_theme(style: &Value, appearance: Appearance, theme: &mut AppTheme) {
    apply_palette(&palette_from_style(style, appearance), theme);
    theme.syntax_styles = syntax_styles_from_style(style);
    theme.syntax_default_color = color_from_style(style, "editor.foreground")
        .or_else(|| color_from_style(style, "text"))
        .unwrap_or(zed_defaults(appearance).editor_foreground);
}

fn resolve_optional_string(primary: Option<&str>, fallback: Option<&str>) -> Option<String> {
    primary
        .and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        })
        .or_else(|| {
            fallback.and_then(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_owned())
            })
        })
}

fn resolve_optional_font_size(primary: Option<f32>, fallback: Option<f32>) -> Option<f32> {
    primary
        .filter(|value| value.is_finite() && *value > 0.0)
        .or_else(|| fallback.filter(|value| value.is_finite() && *value > 0.0))
}

fn palette_from_style(style: &Value, appearance: Appearance) -> Palette {
    let d = zed_defaults(appearance);
    // Zed-parity: every token resolves from its own key(s), else the static
    // per-appearance default — never from another token's RESOLVED value. The
    // only derived-from-resolved tokens are by design: `cursor` and
    // `match_highlight` follow the resolved `text_accent`, and `input_text`
    // follows the resolved `text_primary`.
    let text_primary = color_from_style(style, "text").unwrap_or(d.text);
    let text_accent = color_from_style(style, "text.accent").unwrap_or(d.text_accent);

    Palette {
        // The modal chrome uses Zed's elevated surface color (what Zed paints
        // its pickers with). A missing key means the STATIC default, not the
        // window/editor background: themes like RustRover Dark flatten
        // `editor.background` without authoring `elevated_surface.background`,
        // and Zed renders them two-tone. Full-bleed surface: clamped opaque
        // (see `opaque`).
        bg: opaque(
            color_from_style(style, "elevated_surface.background")
                .unwrap_or(d.elevated_surface_background),
        ),
        border: color_from_style(style, "border").unwrap_or(d.border),
        border_variant: color_from_style(style, "border.variant").unwrap_or(d.border_variant),
        selected_row: color_from_style(style, "ghost_element.selected")
            .unwrap_or(d.ghost_element_selected),
        hover_row: color_from_style(style, "ghost_element.hover").unwrap_or(d.ghost_element_hover),
        text_primary,
        // Same-concept muted-foreground chain (text.muted → icon.muted) kept;
        // the final fallback is the static default.
        text_secondary: color_from_style(style, "text.muted")
            .or_else(|| color_from_style(style, "icon.muted"))
            .unwrap_or(d.text_muted),
        // Same-concept placeholder/disabled chain kept.
        text_dim: color_from_style(style, "text.placeholder")
            .or_else(|| color_from_style(style, "text.disabled"))
            .or_else(|| color_from_style(style, "icon.placeholder"))
            .unwrap_or(d.text_placeholder),
        text_accent,
        // The fuzzy tint / checkbox accent follows `text.accent` (Zed never
        // recolors matched text with the search background).
        match_highlight: text_accent,
        match_highlight_bg: color_from_style(style, "search.match_background")
            .or_else(|| color_from_style(style, "search.active_match_background"))
            .unwrap_or(d.search_match_background),
        // Full-bleed surface: clamped opaque (see `opaque`).
        preview_bg: opaque(
            color_from_style(style, "editor.background").unwrap_or(d.editor_background),
        ),
        // Full-bleed surface: clamped opaque (see `opaque`). No longer falls
        // back to the resolved `preview_bg`; Zed's own gutter default happens
        // to equal its editor background default.
        editor_gutter_bg: opaque(
            color_from_style(style, "editor.gutter.background")
                .unwrap_or(d.editor_gutter_background),
        ),
        editor_line_number: color_from_style(style, "editor.line_number")
            .unwrap_or(d.editor_line_number),
        editor_active_line_number: color_from_style(style, "editor.active_line_number")
            .unwrap_or(d.editor_active_line_number),
        // Fallback tracks the resolved `text_primary`: typed input renders with
        // `input_text` (text_field.rs), so themes lacking `input.foreground`
        // (all bundled ones) paint typed text exactly as the pre-wiring path did.
        input_text: color_from_style(style, "input.foreground").unwrap_or(text_primary),
        cursor: color_from_style(style, "editor.cursor").unwrap_or(text_accent),
        // Same-concept muted-icon chain kept.
        icon_muted: color_from_style(style, "icon.muted")
            .or_else(|| color_from_style(style, "icon.placeholder"))
            .or_else(|| color_from_style(style, "text.muted"))
            .unwrap_or(d.icon_muted),
        // The Zed default carries a genuinely translucent alpha (dark_alpha /
        // light_alpha scale) and blends live at paint time.
        active_line_bg: color_from_style(style, "editor.active_line.background")
            .unwrap_or(d.editor_active_line_background),
        // Git tokens: newer-schema `version_control.*` keys first, then the flat
        // status key still present in Zed theme JSONs, then Zed's default.
        git_created: color_from_style(style, "version_control.added")
            .or_else(|| color_from_style(style, "created"))
            .unwrap_or(d.version_control_added),
        git_modified: color_from_style(style, "version_control.modified")
            .or_else(|| color_from_style(style, "modified"))
            .unwrap_or(d.version_control_modified),
        git_deleted: color_from_style(style, "version_control.deleted")
            .or_else(|| color_from_style(style, "deleted"))
            .unwrap_or(d.version_control_deleted),
        git_conflict: color_from_style(style, "version_control.conflict")
            .or_else(|| color_from_style(style, "conflict"))
            .unwrap_or(d.version_control_conflict),
        git_renamed: color_from_style(style, "version_control.renamed")
            .or_else(|| color_from_style(style, "renamed"))
            .unwrap_or(d.version_control_renamed),
        // No flat status key: Zed remaps untracked to created, which would lose
        // the distinct purple. Only the explicit `version_control.untracked`
        // key may restyle it.
        git_untracked: color_from_style(style, "version_control.untracked")
            .unwrap_or(DEFAULT_GIT_UNTRACKED),
        git_ignored: color_from_style(style, "version_control.ignored")
            .or_else(|| color_from_style(style, "ignored"))
            .unwrap_or(d.version_control_ignored),
        picker_pane_width: None,
    }
}

fn syntax_styles_from_style(style: &Value) -> Vec<(String, SyntaxStyle)> {
    let Some(syntax) = style.get("syntax").and_then(Value::as_object) else {
        return Vec::new();
    };

    syntax
        .iter()
        .filter_map(|(name, style_value)| {
            if name == "background_color" {
                return None;
            }

            if let Some(color) = style_value.as_str() {
                return Some((
                    name.clone(),
                    SyntaxStyle {
                        color: Some(color.to_owned()),
                        ..SyntaxStyle::default()
                    },
                ));
            }

            let style_value = style_value.as_object()?;
            Some((name.clone(), SyntaxStyle::from_value(style_value)))
        })
        .collect()
}

fn syntax_color_from_styles(
    styles: &[(String, SyntaxStyle)],
    capture_name: &str,
    default_color: u32,
) -> u32 {
    let mut best_match: Option<(usize, usize, u32)> = None;

    for (index, (token, style)) in styles.iter().enumerate() {
        let mut specificity = 0;
        if syntax_token_matches_capture(token, capture_name, &mut specificity) {
            let candidate = (
                specificity,
                index,
                syntax_style_color(style).unwrap_or(default_color),
            );
            if best_match.as_ref().is_none_or(|best| candidate > *best) {
                best_match = Some(candidate);
            }
        }
    }

    best_match.map_or(default_color, |(_, _, color)| color)
}

fn syntax_style_for_capture(styles: &[(String, SyntaxStyle)], capture_name: &str) -> SyntaxStyle {
    let mut best_match: Option<(usize, usize, SyntaxStyle)> = None;

    for (index, (token, style)) in styles.iter().enumerate() {
        let mut specificity = 0;
        if syntax_token_matches_capture(token, capture_name, &mut specificity) {
            let candidate = (specificity, index, style.clone());
            if best_match.as_ref().is_none_or(|best| {
                candidate.0 > best.0 || (candidate.0 == best.0 && candidate.1 > best.1)
            }) {
                best_match = Some(candidate);
            }
        }
    }

    best_match.map_or_else(SyntaxStyle::default, |(_, _, style)| style)
}

fn syntax_style_color(style: &SyntaxStyle) -> Option<u32> {
    style.color.as_deref().and_then(parse_color_rgba)
}

fn syntax_style_bg(style: &SyntaxStyle) -> Option<u32> {
    style.background_color.as_deref().and_then(parse_color_rgba)
}

fn syntax_token_matches_capture(token: &str, capture_name: &str, specificity: &mut usize) -> bool {
    let capture_parts: Vec<&str> = capture_name.split('.').collect();
    let mut matched_parts = 0;

    for token_part in token.split('.') {
        if capture_parts
            .iter()
            .any(|capture_part| capture_part == &token_part)
        {
            matched_parts += 1;
        } else {
            return false;
        }
    }

    *specificity = matched_parts;
    true
}

fn syntax_capture_is_punctuation(capture_name: &str) -> bool {
    matches!(capture_name, "punctuation" | "operator") || capture_name.starts_with("punctuation.")
}

fn syntax_capture_uses_variable_color(capture_name: &str) -> bool {
    matches!(capture_name, "constant" | "constructor" | "type")
        || capture_name.starts_with("constant.")
        || capture_name.starts_with("constructor.")
}

impl SyntaxStyle {
    fn from_value(value: &serde_json::Map<String, Value>) -> Self {
        let color = value
            .get("color")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let background_color = value
            .get("background_color")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let font_style = value
            .get("font_style")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let font_weight = value
            .get("font_weight")
            .and_then(Value::as_f64)
            .map(|w| w as f32);

        Self {
            color,
            background_color,
            font_style,
            font_weight,
        }
    }
}

fn color_from_style(style: &Value, key: &str) -> Option<u32> {
    style
        .get(key)
        .and_then(Value::as_str)
        .and_then(parse_color_rgba)
}

fn apply_color(source: &Option<String>, target: &mut u32) {
    if let Some(color) = source.as_deref().and_then(parse_color_rgba) {
        *target = color;
    }
}

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

// Parse a hex color string into 0xRRGGBBAA. Accepts `#rrggbb` (implied opaque),
// `#rrggbbaa` (authored alpha preserved), and the `#rgb`/`#rgba` shorthands
// (each digit doubles). The leading `#` is optional; invalid input is `None`.
fn parse_color_rgba(color: &str) -> Option<u32> {
    let color = color.trim();
    let color = color.strip_prefix('#').unwrap_or(color);

    match color.len() {
        3 | 4 => {
            let mut expanded = String::with_capacity(8);
            for ch in color.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            if color.len() == 3 {
                expanded.push_str("ff");
            }
            u32::from_str_radix(&expanded, 16).ok()
        }
        6 => u32::from_str_radix(color, 16)
            .ok()
            .map(|rgb| (rgb << 8) | 0xFF),
        8 => u32::from_str_radix(color, 16).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syntax_style(color: u32) -> SyntaxStyle {
        SyntaxStyle {
            color: Some(format!("#{color:06x}")),
            ..SyntaxStyle::default()
        }
    }

    #[test]
    fn picker_pane_width_defaults_to_none_across_the_pipeline() {
        assert_eq!(AppTheme::default().picker_pane_width, None);
        assert_eq!(Palette::default().picker_pane_width, None);
        // Theme JSON never carries a pane width — it is config-owned.
        let style = serde_json::json!({ "background": "#101010" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).picker_pane_width,
            None
        );
    }

    #[test]
    fn apply_palette_preserves_config_owned_picker_pane_width() {
        let mut theme = AppTheme {
            picker_pane_width: Some(430.0),
            ..AppTheme::default()
        };
        apply_palette(&Palette::default(), &mut theme);
        assert_eq!(theme.picker_pane_width, Some(430.0));
    }

    #[test]
    fn merge_json_deep_merges_objects() {
        let mut base = serde_json::json!({ "a": 1, "nested": { "x": 1, "y": 2 } });
        let overlay = serde_json::json!({ "b": 2, "nested": { "y": 9, "z": 3 } });
        merge_json(&mut base, &overlay);
        assert_eq!(
            base,
            serde_json::json!({ "a": 1, "b": 2, "nested": { "x": 1, "y": 9, "z": 3 } })
        );
    }

    #[test]
    fn merge_json_null_overlay_keeps_base() {
        let mut base = serde_json::json!({ "font_style": "italic", "color": "#fff" });
        let overlay = serde_json::json!({ "font_style": null });
        merge_json(&mut base, &overlay);
        assert_eq!(
            base,
            serde_json::json!({ "font_style": "italic", "color": "#fff" })
        );
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

    #[test]
    fn parses_hex_colors() {
        // 6-hex appends an opaque alpha byte.
        assert_eq!(parse_color_rgba("#ff00aa"), Some(0xff00aaff));
        // 3-hex shorthand expands each digit, then appends opaque alpha.
        assert_eq!(parse_color_rgba("#f0a"), Some(0xff00aaff));
        // 4-hex shorthand expands each digit including the alpha digit.
        assert_eq!(parse_color_rgba("#f0ab"), Some(0xff00aabb));
        assert_eq!(parse_color_rgba("#1234"), Some(0x11223344));
    }

    #[test]
    fn parse_color_rgba_preserves_authored_alpha() {
        // 8-hex keeps the authored alpha byte verbatim.
        assert_eq!(parse_color_rgba("#ff00aaff"), Some(0xff00aaff));
        assert_eq!(parse_color_rgba("#2f343ebf"), Some(0x2f343ebf));
        assert_eq!(parse_color_rgba("#74ade866"), Some(0x74ade866));
        assert_eq!(parse_color_rgba("#00000000"), Some(0x00000000));
    }

    #[test]
    fn parse_color_rgba_prefix_and_whitespace_handling() {
        // The leading `#` is optional and surrounding whitespace is trimmed.
        assert_eq!(parse_color_rgba("ff00aa"), Some(0xff00aaff));
        assert_eq!(parse_color_rgba("2f343ebf"), Some(0x2f343ebf));
        assert_eq!(parse_color_rgba("  #ff00aa  "), Some(0xff00aaff));
    }

    #[test]
    fn parse_color_rgba_rejects_invalid_input() {
        assert_eq!(parse_color_rgba(""), None);
        assert_eq!(parse_color_rgba("#"), None);
        assert_eq!(parse_color_rgba("#12345"), None); // bad length
        assert_eq!(parse_color_rgba("#1234567"), None); // bad length
        assert_eq!(parse_color_rgba("#123456789"), None); // bad length
        assert_eq!(parse_color_rgba("#zzzzzz"), None); // non-hex digits
        assert_eq!(parse_color_rgba("#zzz"), None);
        assert_eq!(parse_color_rgba("not a color"), None);
    }

    #[test]
    fn resolves_dynamic_theme_names() {
        let selection = ThemeSelection::Dynamic {
            mode: ThemeMode::System,
            light: "Light Theme".to_string(),
            dark: "Dark Theme".to_string(),
        };

        assert_eq!(
            resolve_theme_name(&selection, WindowAppearance::Dark),
            "Dark Theme"
        );
        assert_eq!(
            resolve_theme_name(&selection, WindowAppearance::Light),
            "Light Theme"
        );
    }

    #[test]
    fn syntax_color_prefers_later_matches_on_ties() {
        let styles = vec![
            ("foo.bar".to_string(), syntax_style(0x111111)),
            ("baz.qux".to_string(), syntax_style(0x222222)),
        ];

        assert_eq!(
            syntax_color_from_styles(&styles, "foo.bar.baz.qux", 0x999999),
            0x222222ff
        );
    }

    #[test]
    fn constant_and_punctuation_captures_follow_variable_and_text_colors() {
        let theme = AppTheme {
            syntax_styles: vec![
                ("variable".to_string(), syntax_style(0x112233)),
                ("constant".to_string(), syntax_style(0x445566)),
                ("constructor".to_string(), syntax_style(0x778899)),
                ("punctuation".to_string(), syntax_style(0xaabbcc)),
            ],
            syntax_default_color: 0xddeeffff,
            ..AppTheme::default()
        };

        assert_eq!(theme.syntax_color("constant"), 0x112233ff);
        assert_eq!(theme.syntax_color("constructor"), 0x112233ff);
        assert_eq!(theme.syntax_color("type"), 0x112233ff);
        assert_eq!(theme.syntax_color("punctuation"), 0xddeeffff);
        assert_eq!(theme.syntax_color("punctuation.bracket"), 0xddeeffff);
    }

    #[test]
    fn syntax_render_style_assembles_color_bg_and_font_flags() {
        let theme = AppTheme {
            syntax_styles: vec![(
                "keyword".to_string(),
                SyntaxStyle {
                    color: Some("#112233".to_string()),
                    background_color: Some("#445566".to_string()),
                    font_style: Some("italic".to_string()),
                    // Weight >= 600 => bold, exercised alongside italic.
                    font_weight: Some(700.0),
                },
            )],
            syntax_default_color: 0xddeeffff,
            ..AppTheme::default()
        };

        assert_eq!(
            theme.syntax_render_style("keyword"),
            SyntaxRenderStyle {
                color: 0x112233ff,
                bg: Some(0x445566ff),
                italic: true,
                bold: true,
                underline: false,
                strikethrough: false,
            }
        );
    }

    #[test]
    fn syntax_render_style_prefers_later_matches_on_ties() {
        let theme = AppTheme {
            syntax_styles: vec![
                (
                    "foo.bar".to_string(),
                    SyntaxStyle {
                        color: Some("#111111".to_string()),
                        background_color: Some("#aaaaaa".to_string()),
                        font_style: Some("underline".to_string()),
                        font_weight: None,
                    },
                ),
                (
                    "baz.qux".to_string(),
                    SyntaxStyle {
                        color: Some("#222222".to_string()),
                        background_color: Some("#bbbbbb".to_string()),
                        font_style: Some("strikethrough".to_string()),
                        font_weight: None,
                    },
                ),
            ],
            syntax_default_color: 0x999999ff,
            ..AppTheme::default()
        };

        // Both tokens match "foo.bar.baz.qux" with specificity 2; the later
        // entry wins the tie and supplies the whole assembled struct (mirrors
        // `syntax_color_prefers_later_matches_on_ties`).
        assert_eq!(
            theme.syntax_render_style("foo.bar.baz.qux"),
            SyntaxRenderStyle {
                color: 0x222222ff,
                bg: Some(0xbbbbbbff),
                italic: false,
                bold: false,
                underline: false,
                strikethrough: true,
            }
        );
    }

    #[test]
    fn syntax_render_style_redirects_variable_family_captures() {
        let theme = AppTheme {
            syntax_styles: vec![(
                "variable".to_string(),
                SyntaxStyle {
                    color: Some("#112233".to_string()),
                    background_color: Some("#445566".to_string()),
                    font_style: None,
                    font_weight: None,
                },
            )],
            syntax_default_color: 0xddeeffff,
            ..AppTheme::default()
        };

        let expected = SyntaxRenderStyle {
            color: 0x112233ff,
            bg: Some(0x445566ff),
            ..SyntaxRenderStyle::default()
        };
        assert_eq!(theme.syntax_render_style("constant"), expected);
        assert_eq!(theme.syntax_render_style("constructor"), expected);
        assert_eq!(theme.syntax_render_style("type"), expected);
    }

    #[test]
    fn syntax_render_style_drops_background_for_punctuation() {
        let theme = AppTheme {
            syntax_styles: vec![(
                "punctuation".to_string(),
                SyntaxStyle {
                    color: Some("#112233".to_string()),
                    background_color: Some("#445566".to_string()),
                    font_style: Some("italic".to_string()),
                    font_weight: Some(700.0),
                },
            )],
            syntax_default_color: 0xddeeffff,
            ..AppTheme::default()
        };

        // Punctuation short-circuits to the default foreground with no bg or
        // font flags, even though the theme set a background_color/font.
        let expected = SyntaxRenderStyle {
            color: 0xddeeffff,
            ..SyntaxRenderStyle::default()
        };
        assert_eq!(theme.syntax_render_style("punctuation"), expected);
        assert_eq!(theme.syntax_render_style("punctuation.bracket"), expected);
    }

    #[test]
    fn applies_explicit_font_settings_from_zed() {
        let settings = ZedSettings {
            ui_font_family: Some("Test UI Font".to_string()),
            buffer_font_family: Some("Test Buffer Font".to_string()),
            ui_font_size: Some(20.0),
            buffer_font_size: Some(13.5),
            ..ZedSettings::default()
        };

        let mut theme = AppTheme {
            ui_font_family: Some("Old UI".to_string()),
            buffer_font_family: Some("Old Buffer".to_string()),
            ui_font_size: 99.0,
            buffer_font_size: 88.0,
            ..AppTheme::default()
        };
        apply_font_settings(&settings, &mut theme);

        assert_eq!(theme.ui_font_family, Some("Test UI Font".to_string()));
        assert_eq!(
            theme.buffer_font_family,
            Some("Test Buffer Font".to_string())
        );
        assert_eq!(theme.ui_font_size, 20.0);
        assert_eq!(theme.buffer_font_size, 13.5);
    }

    #[test]
    fn falls_back_to_default_fonts_when_unset() {
        let settings = ZedSettings::default();

        let mut theme = AppTheme {
            ui_font_family: Some("Old UI".to_string()),
            buffer_font_family: Some("Old Buffer".to_string()),
            ui_font_size: 99.0,
            buffer_font_size: 88.0,
            ..AppTheme::default()
        };
        apply_font_settings(&settings, &mut theme);

        assert_eq!(
            theme.ui_font_family,
            Some(DEFAULT_UI_FONT_FAMILY.to_string())
        );
        assert_eq!(
            theme.buffer_font_family,
            Some(DEFAULT_BUFFER_FONT_FAMILY.to_string())
        );
        assert_eq!(theme.ui_font_size, DEFAULT_UI_FONT_SIZE);
        assert_eq!(theme.buffer_font_size, DEFAULT_BUFFER_FONT_SIZE);
    }

    #[test]
    fn falls_back_to_default_fonts_when_size_invalid() {
        for invalid in [0.0_f32, -5.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let settings = ZedSettings {
                ui_font_size: Some(invalid),
                buffer_font_size: Some(invalid),
                ..ZedSettings::default()
            };

            let mut theme = AppTheme {
                ui_font_size: 99.0,
                buffer_font_size: 88.0,
                ..AppTheme::default()
            };
            apply_font_settings(&settings, &mut theme);

            assert_eq!(theme.ui_font_size, DEFAULT_UI_FONT_SIZE);
            assert_eq!(theme.buffer_font_size, DEFAULT_BUFFER_FONT_SIZE);
        }
    }

    #[test]
    fn falls_back_to_default_fonts_when_family_blank() {
        for blank in ["", "   ", "\t"] {
            let settings = ZedSettings {
                ui_font_family: Some(blank.to_string()),
                buffer_font_family: Some(blank.to_string()),
                ..ZedSettings::default()
            };

            let mut theme = AppTheme {
                ui_font_family: Some("Old UI".to_string()),
                buffer_font_family: Some("Old Buffer".to_string()),
                ..AppTheme::default()
            };
            apply_font_settings(&settings, &mut theme);

            assert_eq!(
                theme.ui_font_family,
                Some(DEFAULT_UI_FONT_FAMILY.to_string())
            );
            assert_eq!(
                theme.buffer_font_family,
                Some(DEFAULT_BUFFER_FONT_FAMILY.to_string())
            );
            assert_eq!(theme.ui_font_size, DEFAULT_UI_FONT_SIZE);
            assert_eq!(theme.buffer_font_size, DEFAULT_BUFFER_FONT_SIZE);
        }
    }

    #[test]
    fn builtin_theme_catalog_includes_zed_themes() {
        let catalog = builtin_theme_catalog().expect("theme catalog should load");

        assert!(catalog.contains_key("ayu dark"));
        assert!(catalog.contains_key("gruvbox dark"));
        assert!(catalog.contains_key("one dark"));
    }

    #[test]
    fn apply_style_to_theme_sets_palette_and_syntax() {
        // `bg` sources from `elevated_surface.background` (its own key only).
        let style = serde_json::json!({
            "elevated_surface.background": "#101010",
            "border": "#202020",
            "editor.foreground": "#fafafa",
            "syntax": { "keyword": { "color": "#abcdef" } }
        });
        let mut theme = AppTheme::default();
        apply_style_to_theme(&style, Appearance::Dark, &mut theme);
        assert_eq!(theme.bg, 0x101010ff);
        assert_eq!(theme.border, 0x202020ff);
        assert_eq!(theme.syntax_default_color, 0xfafafaff);
        assert_eq!(theme.syntax_color("keyword"), 0xabcdefff);
    }

    #[test]
    fn catalog_entries_retain_raw_style() {
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let entry = catalog.get("one dark").expect("one dark present");
        assert!(entry.style.is_object());
        assert!(entry.style.get("syntax").is_some());
        // The raw style must retain a known One Dark syntax token, proving the
        // full style object survived (not just an empty `syntax` key).
        assert!(
            entry
                .style
                .get("syntax")
                .and_then(|s| s.get("keyword"))
                .is_some()
        );
    }

    #[test]
    fn override_merges_color_and_syntax_onto_base() {
        let catalog = builtin_theme_catalog().expect("catalog loads");

        // Capture the base One Dark color for a token we will NOT override, by
        // applying the theme with an empty overrides map into a separate theme.
        let mut base = AppTheme::default();
        let empty = HashMap::new();
        apply_theme_with_overrides("One Dark", &catalog, &empty, &mut base);
        let base_string_color = base.syntax_color("string");

        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("One Dark"),
            serde_json::json!({
                // `bg` sources from `elevated_surface.background` (One Dark has it).
                "elevated_surface.background": "#123456",
                "syntax": { "keyword": { "color": "#0f0f0f" } }
            }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456ff); // overridden color
        assert_eq!(theme.syntax_color("keyword"), 0x0f0f0fff); // overridden token
        // The non-overridden `string` token must still equal the captured base
        // value, proving `merge_json` merged per-token instead of replacing the
        // whole `syntax` object.
        assert_eq!(theme.syntax_color("string"), base_string_color);
    }

    #[test]
    fn synthetic_catalog_override_merges_per_syntax_token() {
        // Filesystem-independent: build a catalog entry in-memory the same way
        // the real loader (`load_theme_family_contents`) derives its fields.
        let style = serde_json::json!({
            "elevated_surface.background": "#000000",
            "editor.foreground": "#eeeeee",
            "syntax": {
                "keyword": { "color": "#111111" },
                "string": { "color": "#222222" }
            }
        });
        let entry = ThemeCatalogEntry {
            appearance: Appearance::Dark,
            palette: palette_from_style(&style, Appearance::Dark),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(ZED_DARK_DEFAULTS.editor_foreground),
            style,
        };

        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        // Override only `keyword` and the elevated surface (fff's `bg` key);
        // leave `string` untouched.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({
                "elevated_surface.background": "#333333",
                "syntax": { "keyword": { "color": "#333333" } }
            }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);

        assert_eq!(theme.syntax_color("keyword"), 0x333333ff); // overridden token
        assert_eq!(theme.syntax_color("string"), 0x222222ff); // untouched base token
        assert_eq!(theme.bg, 0x333333ff); // overridden background
    }

    #[test]
    fn fff_config_color_wins_over_zed_override() {
        // Asserts the documented precedence contract (Zed override first, then fff
        // `[theme]` config last) rather than calling `sync_from_config` directly,
        // which would require a `&mut App` (gpui) that is impractical to wire up in
        // a unit test. This manually sequences `apply_theme_with_overrides` then
        // `apply_color` exactly as `sync_from_config` does, so the explicit fff
        // config must win for the same field.
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("One Dark"),
            // `bg` sources from `elevated_surface.background` (One Dark has it).
            serde_json::json!({ "elevated_surface.background": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456ff); // Zed override applied first

        // fff `[theme].bg = "#abcdef"` applied last, exactly as `sync_from_config` does.
        apply_color(&Some("#abcdef".to_string()), &mut theme.bg);
        assert_eq!(theme.bg, 0xabcdefff); // explicit fff config wins over the Zed override
    }

    #[test]
    fn override_for_other_theme_does_not_apply() {
        let catalog = builtin_theme_catalog().expect("catalog loads");

        // Capture One Dark's real base `bg` by applying it with empty overrides.
        let mut base = AppTheme::default();
        let empty = HashMap::new();
        apply_theme_with_overrides("One Dark", &catalog, &empty, &mut base);
        let base_bg = base.bg;

        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Ayu Dark"),
            serde_json::json!({ "background": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        assert_ne!(theme.bg, 0x123456ff); // One Dark selected; Ayu override must NOT apply
        // The result must equal the real, un-overridden One Dark base color,
        // proving the Ayu-keyed override was correctly ignored.
        assert_eq!(theme.bg, base_bg);
    }

    #[test]
    fn no_override_matches_plain_catalog_application() {
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let empty = HashMap::new();
        let mut with_helper = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &empty, &mut with_helper);

        // The plain path: look up the catalog entry and apply it directly, exactly
        // as the no-override fast path inside `apply_theme_with_overrides` does.
        let mut plain = AppTheme::default();
        let entry = catalog
            .get(&normalize_name("One Dark"))
            .expect("one dark present");
        apply_catalog_entry(entry, &mut plain);

        // `apply_theme_with_overrides` (empty overrides) must produce exactly the
        // same `AppTheme` as the direct `apply_catalog_entry` path: full palette,
        // syntax styles, and default color.
        assert_eq!(with_helper, plain);
    }

    #[test]
    fn zed_settings_parses_theme_overrides() {
        let settings: ZedSettings = json5::from_str(
            r##"{ "theme": "One Dark", "theme_overrides": { "One Dark": { "background": "#123456" } } }"##,
        )
        .expect("parses");
        // The override entry is present and its nested payload survived deserialization.
        let entry = settings
            .theme_overrides
            .get("One Dark")
            .expect("One Dark override present");
        assert_eq!(
            entry.get("background").and_then(Value::as_str),
            Some("#123456")
        );
        // The normalized lookup keys by `normalize_name`, preserving the payload.
        let normalized = normalized_theme_overrides(&settings.theme_overrides);
        let normalized_entry = normalized
            .get(&normalize_name("One Dark"))
            .expect("normalized override keyed by \"one dark\"");
        assert_eq!(
            normalized_entry.get("background").and_then(Value::as_str),
            Some("#123456")
        );
    }

    #[test]
    fn syntax_styles_from_style_accepts_bare_string_color() {
        // Zed `theme_overrides` (and `merge_json` results) can produce a syntax
        // token written as a bare color string instead of a `{ "color": ... }`
        // object. It must still be picked up as the token's color.
        let style = serde_json::json!({
            "syntax": {
                "keyword": "#aabbcc",
                "string": { "color": "#112233" }
            }
        });
        let styles = syntax_styles_from_style(&style);
        let keyword = styles
            .iter()
            .find(|(name, _)| name == "keyword")
            .map(|(_, style)| style)
            .expect("keyword present");
        assert_eq!(keyword.color.as_deref(), Some("#aabbcc"));
        assert_eq!(syntax_style_color(keyword), Some(0xaabbccff));
    }

    #[test]
    fn mixed_case_override_key_applies_via_normalized_overrides() {
        let catalog = builtin_theme_catalog().expect("catalog loads");
        // Raw, mixed-case override key as it might appear in Zed's settings.json.
        let mut raw_overrides = HashMap::new();
        raw_overrides.insert(
            "ONE DARK".to_string(),
            // `bg` sources from `elevated_surface.background` (One Dark has it).
            serde_json::json!({ "elevated_surface.background": "#123456" }),
        );
        let overrides = normalized_theme_overrides(&raw_overrides);

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        // Case-insensitive match works end-to-end: the override applied.
        assert_eq!(theme.bg, 0x123456ff);
    }

    #[test]
    fn apply_theme_with_overrides_leaves_theme_unchanged_when_absent() {
        let catalog = builtin_theme_catalog().expect("catalog loads");

        // Mutate `theme` into a clearly non-default state first, so the assertion
        // can distinguish the early-return guard actually firing from a no-op on a
        // freshly-defaulted theme. Apply a real bundled theme (One Dark).
        let empty = HashMap::new();
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &empty, &mut theme);
        assert_ne!(
            theme,
            AppTheme::default(),
            "precondition: theme must be non-default before the absent lookup"
        );
        let before = theme.clone();

        // A non-empty overrides map keyed on the (also absent) target proves that
        // the absent-theme path mutates nothing even when a matching override key
        // exists, since the catalog lookup misses and returns early.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Nonexistent Theme XYZ"),
            serde_json::json!({ "background": "#123456" }),
        );

        apply_theme_with_overrides("Nonexistent Theme XYZ", &catalog, &overrides, &mut theme);
        // Theme not in catalog: nothing is applied and nothing panics.
        assert_eq!(theme, before);
    }

    #[test]
    fn dynamic_selection_applies_override_for_resolved_variant_only() {
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let selection = ThemeSelection::Dynamic {
            mode: ThemeMode::System,
            light: "One Light".to_string(),
            dark: "One Dark".to_string(),
        };
        // In Dark appearance the resolved variant is "One Dark".
        let resolved = resolve_theme_name(&selection, WindowAppearance::Dark);
        assert_eq!(resolved, "One Dark");

        // An override keyed on the RESOLVED variant applies.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("One Dark"),
            // `bg` sources from `elevated_surface.background` (One Dark has it).
            serde_json::json!({ "elevated_surface.background": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides(&resolved, &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456ff);

        // Capture the resolved variant's real base bg (apply One Dark with EMPTY
        // overrides) so the negative half can prove the base was applied, not just
        // that the wrong-variant sentinel is absent.
        let mut base = AppTheme::default();
        apply_theme_with_overrides(&resolved, &catalog, &HashMap::new(), &mut base);
        let base_bg = base.bg;

        // An override keyed on the OTHER variant ("One Light") does not apply; the
        // resolved base ("One Dark") is applied via `apply_catalog_entry` instead.
        let sentinel = 0x654321ff_u32;
        let mut other_overrides = HashMap::new();
        other_overrides.insert(
            normalize_name("One Light"),
            serde_json::json!({ "background": "#654321" }),
        );
        let mut other_theme = AppTheme::default();
        apply_theme_with_overrides(&resolved, &catalog, &other_overrides, &mut other_theme);
        assert_ne!(other_theme.bg, sentinel);
        // Proves the resolved base ran and the wrong-variant override did NOT apply.
        assert_eq!(other_theme.bg, base_bg);
    }

    #[test]
    fn malformed_hex_in_syntax_override_falls_back_to_default() {
        // Filesystem-independent synthetic catalog entry. A syntax override with an
        // invalid color string must silently fall back to `syntax_default_color`
        // (the documented behavior) rather than panicking or corrupting the token.
        let style = serde_json::json!({
            "background": "#000000",
            "editor.foreground": "#eeeeee",
            "syntax": {
                "keyword": { "color": "#111111" },
                "string": { "color": "#222222" }
            }
        });
        let entry = ThemeCatalogEntry {
            appearance: Appearance::Dark,
            palette: palette_from_style(&style, Appearance::Dark),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(ZED_DARK_DEFAULTS.editor_foreground),
            style,
        };

        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        // Override two tokens in the SAME payload: `keyword` with a malformed color,
        // and `string` with a valid one. The valid token proves overrides in this
        // payload actually reach the merge, so a regression that silently drops
        // overrides would fail here rather than masquerading as the fallback.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({ "syntax": {
                "keyword": { "color": "#zzzzzz" },
                "string": { "color": "#44aa55" }
            } }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);

        // The valid override reached the merge and applied.
        assert_eq!(theme.syntax_color("string"), 0x44aa55ff);
        // The malformed color is unparseable, so `keyword` resolves to the theme's
        // default foreground color (0xeeeeeeff), not the bogus override.
        assert_eq!(theme.syntax_default_color, 0xeeeeeeff);
        assert_eq!(theme.syntax_color("keyword"), 0xeeeeeeff);
    }

    #[test]
    fn zed_settings_parses_empty_theme_overrides() {
        // An explicit empty object must deserialize cleanly and yield an empty map,
        // matching the `#[serde(default)]` (omitted) boundary case.
        let settings: ZedSettings =
            json5::from_str(r##"{ "theme": "One Dark", "theme_overrides": {} }"##).expect("parses");
        assert!(settings.theme_overrides.is_empty());
    }

    #[test]
    fn merge_json_scalar_base_replaced_by_object_overlay() {
        let mut base = serde_json::json!({ "k": "scalar" });
        let overlay = serde_json::json!({ "k": { "a": 1 } });
        merge_json(&mut base, &overlay);
        assert_eq!(base, serde_json::json!({ "k": { "a": 1 } }));
    }

    #[test]
    fn merge_json_object_base_replaced_by_scalar_overlay() {
        // The realistic "bare string stomps object" case: an override writes a
        // syntax token (or whole `syntax` map) as a bare color string, replacing
        // the base object wholesale.
        let mut base = serde_json::json!({ "syntax": { "keyword": { "color": "#111" } } });
        let overlay = serde_json::json!({ "syntax": "#aabbcc" });
        merge_json(&mut base, &overlay);
        assert_eq!(base["syntax"], serde_json::json!("#aabbcc"));
    }

    #[test]
    fn null_override_leaves_base_unchanged_through_pipeline() {
        // Exercises the `null` = no-op contract through the full
        // `apply_theme_with_overrides` pipeline (not just `merge_json`): a null
        // override leaves the base value intact, while a sibling non-null override
        // in the same payload still applies.
        let style = serde_json::json!({
            "elevated_surface.background": "#000000",
            "editor.foreground": "#eeeeee",
            "syntax": {
                "keyword": { "color": "#111111" },
                "string": { "color": "#222222" }
            }
        });
        let entry = ThemeCatalogEntry {
            appearance: Appearance::Dark,
            palette: palette_from_style(&style, Appearance::Dark),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(ZED_DARK_DEFAULTS.editor_foreground),
            style,
        };

        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        // `keyword.color` and the elevated surface (fff's `bg` key) are nulled
        // (no-op); `string` is a real sibling override that must still apply.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({
                "elevated_surface.background": null,
                "syntax": {
                    "keyword": { "color": null },
                    "string": { "color": "#44aa55" }
                }
            }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);

        // Null left the base values untouched.
        assert_eq!(theme.syntax_color("keyword"), 0x111111ff);
        assert_eq!(theme.bg, 0x000000ff);
        // The sibling non-null override still applied.
        assert_eq!(theme.syntax_color("string"), 0x44aa55ff);
    }

    #[test]
    fn palette_maps_new_tokens_from_theme_json() {
        // Both the newer `version_control.*` schema and the flat status keys are
        // present: the `version_control.*` key must win for every git token.
        let style = serde_json::json!({
            "background": "#101010",
            "elevated_surface.background": "#181818",
            "editor.active_line.background": "#2f343ebf",
            "version_control.added": "#27a657",
            "version_control.modified": "#d3b020",
            "version_control.deleted": "#e06c76",
            "version_control.conflict": "#dec184",
            "version_control.renamed": "#74ade8",
            "version_control.untracked": "#9b8aec",
            "version_control.ignored": "#878a98",
            "created": "#111111",
            "modified": "#222222",
            "deleted": "#333333",
            "conflict": "#444444",
            "renamed": "#555555",
            "ignored": "#666666"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        // `bg` sources its own key: the elevated surface.
        assert_eq!(palette.bg, 0x181818ff);
        // The authored alpha suffix is preserved by `parse_color_rgba`.
        assert_eq!(palette.active_line_bg, 0x2f343ebf);
        assert_eq!(palette.git_created, 0x27a657ff);
        assert_eq!(palette.git_modified, 0xd3b020ff);
        assert_eq!(palette.git_deleted, 0xe06c76ff);
        assert_eq!(palette.git_conflict, 0xdec184ff);
        assert_eq!(palette.git_renamed, 0x74ade8ff);
        assert_eq!(palette.git_untracked, 0x9b8aecff);
        assert_eq!(palette.git_ignored, 0x878a98ff);
    }

    #[test]
    fn palette_git_tokens_fall_back_to_flat_status_keys() {
        // Only the flat status keys (as real Zed theme JSONs like one.json carry
        // at the top level) are present: they are the second lookup choice.
        let style = serde_json::json!({
            "created": "#111111",
            "modified": "#222222",
            "deleted": "#333333",
            "conflict": "#444444",
            "renamed": "#555555",
            "ignored": "#666666"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.git_created, 0x111111ff);
        assert_eq!(palette.git_modified, 0x222222ff);
        assert_eq!(palette.git_deleted, 0x333333ff);
        assert_eq!(palette.git_conflict, 0x444444ff);
        assert_eq!(palette.git_renamed, 0x555555ff);
        assert_eq!(palette.git_ignored, 0x666666ff);
        // Untracked has NO flat key on purpose (Zed remaps untracked to created,
        // which would lose the distinct purple): `created` must not leak into it.
        assert_eq!(palette.git_untracked, DEFAULT_GIT_UNTRACKED);
    }

    #[test]
    fn palette_new_tokens_fall_back_when_keys_missing() {
        let style = serde_json::json!({ "background": "#101010" });
        let palette = palette_from_style(&style, Appearance::Dark);
        // No elevated surface key: `bg` is the STATIC dark default — the plain
        // `background` key is no longer part of any chain (Zed semantics).
        assert_eq!(palette.bg, ZED_DARK_DEFAULTS.elevated_surface_background);
        // Git fallbacks are Zed's `version_control_*` dark defaults.
        assert_eq!(palette.git_created, 0x2E9E48FF);
        assert_eq!(palette.git_modified, 0xD3AF1DFF);
        assert_eq!(palette.git_deleted, 0x78081AFF);
        assert_eq!(palette.git_conflict, 0xFFE0C2FF);
        assert_eq!(palette.git_renamed, 0xD3AF1DFF); // Zed reuses MODIFIED_COLOR
        assert_eq!(palette.git_untracked, 0xA48EFFFF); // distinct purple kept (no Zed default)
        assert_eq!(palette.git_ignored, 0xEEEEEEFF);
    }

    #[test]
    fn active_line_bg_falls_back_to_zed_static_default() {
        // Without `editor.active_line.background`, the token is Zed's static
        // default (dark_alpha scale — genuinely translucent, blended live at
        // paint time via `rgba(...)`) — NOT derived from the selection color.
        let style = serde_json::json!({
            "ghost_element.selected": "#404040",
            "editor.background": "#000000"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(
            palette.active_line_bg,
            ZED_DARK_DEFAULTS.editor_active_line_background
        );
        assert_eq!(palette.active_line_bg, 0xF6F6F513);
        // Same value in the hardcoded defaults.
        assert_eq!(Palette::default().active_line_bg, 0xF6F6F513);
    }

    #[test]
    fn active_line_bg_sync_preserves_authored_alpha() {
        // One Dark authors `editor.active_line.background` as `#2f343ebf` — the
        // same RGB as its elevated surface `bg` but at 75% alpha. The old parser
        // dropped the alpha, collapsing the token to exactly `bg` (invisible
        // active line). It must now stay translucent and distinct from `bg`.
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &HashMap::new(), &mut theme);
        assert_eq!(theme.bg, 0x2f343eff);
        assert_eq!(theme.active_line_bg, 0x2f343ebf);
        assert_ne!(theme.active_line_bg, theme.bg);
        // The translucent match background keeps its authored 40% alpha too.
        assert_eq!(theme.match_highlight_bg, 0x74ade866);
    }

    #[test]
    fn apply_color_override_accepts_opaque_and_alpha_hex() {
        // `[theme]` config overrides run through `apply_color`: 6-hex is
        // implied-opaque, 8-hex keeps its authored alpha byte, and invalid or
        // absent input leaves the previous value untouched.
        let mut color = 0x000000ff;
        apply_color(&Some("#abcdef".to_string()), &mut color);
        assert_eq!(color, 0xabcdefff);
        apply_color(&Some("#11223344".to_string()), &mut color);
        assert_eq!(color, 0x11223344);
        apply_color(&Some("not a color".to_string()), &mut color);
        assert_eq!(color, 0x11223344);
        apply_color(&None, &mut color);
        assert_eq!(color, 0x11223344);
    }

    #[test]
    fn all_opaque_theme_yields_fully_opaque_palette() {
        // A theme authored entirely in 6-hex (no alpha anywhere) must resolve
        // every palette token fully opaque — live `rgba()` blending of `..ff`
        // values paints identically to the old pre-refactor `rgb()` path.
        // Every token's primary key is authored, so the static Zed defaults
        // (several of which genuinely carry alpha: ghost elements,
        // active_line_bg, dark line numbers) never engage.
        let style = serde_json::json!({
            "elevated_surface.background": "#101010",
            "ghost_element.selected": "#202020",
            "ghost_element.hover": "#303030",
            "editor.background": "#050505",
            "border": "#404040",
            "border.variant": "#353535",
            "text": "#e0e0e0",
            "text.muted": "#a0a0a0",
            "text.placeholder": "#606060",
            "text.accent": "#4a9eff",
            "search.match_background": "#2c4870",
            "editor.gutter.background": "#070707",
            "editor.line_number": "#707070",
            "editor.active_line_number": "#d0d0d0",
            "editor.active_line.background": "#151515",
            "editor.cursor": "#4a9eff",
            "input.foreground": "#e5e5ea",
            "icon.muted": "#909090",
            "version_control.added": "#27a657",
            "version_control.modified": "#d3b020",
            "version_control.deleted": "#e06c76",
            "version_control.conflict": "#dec184",
            "version_control.renamed": "#74ade8",
            "version_control.untracked": "#9b8aec",
            "version_control.ignored": "#878a98"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        for (name, value) in [
            ("bg", palette.bg),
            ("border", palette.border),
            ("border_variant", palette.border_variant),
            ("selected_row", palette.selected_row),
            ("hover_row", palette.hover_row),
            ("text_primary", palette.text_primary),
            ("text_secondary", palette.text_secondary),
            ("text_dim", palette.text_dim),
            ("text_accent", palette.text_accent),
            ("match_highlight", palette.match_highlight),
            ("match_highlight_bg", palette.match_highlight_bg),
            ("preview_bg", palette.preview_bg),
            ("editor_gutter_bg", palette.editor_gutter_bg),
            ("editor_line_number", palette.editor_line_number),
            (
                "editor_active_line_number",
                palette.editor_active_line_number,
            ),
            ("input_text", palette.input_text),
            ("cursor", palette.cursor),
            ("icon_muted", palette.icon_muted),
            ("active_line_bg", palette.active_line_bg),
            ("git_created", palette.git_created),
            ("git_modified", palette.git_modified),
            ("git_deleted", palette.git_deleted),
            ("git_conflict", palette.git_conflict),
            ("git_renamed", palette.git_renamed),
            ("git_untracked", palette.git_untracked),
            ("git_ignored", palette.git_ignored),
        ] {
            assert_eq!(value & 0xFF, 0xFF, "{name} must resolve fully opaque");
        }
    }

    #[test]
    fn border_variant_resolves_own_key_else_static_default() {
        // Key present: `border.variant` wins.
        let style = serde_json::json!({ "border.variant": "#363c46", "border": "#464b57" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).border_variant,
            0x363c46ff
        );
        // Key absent: the STATIC default — no longer a copy of the resolved
        // `border` (Zed semantics: no inter-key inheritance).
        let style = serde_json::json!({ "border": "#464b57" });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.border_variant, ZED_DARK_DEFAULTS.border_variant);
        assert_ne!(palette.border_variant, palette.border);
        // All absent: same static default.
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).border_variant,
            0x31312EFF
        );
        // 8-hex authored alpha threads through unclamped: `border_variant` is
        // subtle chrome, not a full-bleed surface, so it blends live at paint.
        let style = serde_json::json!({ "border.variant": "#363c4680" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).border_variant,
            0x363c4680
        );
    }

    #[test]
    fn text_accent_maps_key_with_zed_blue_fallback() {
        let style = serde_json::json!({ "text.accent": "#74ade8" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).text_accent,
            0x74ade8ff
        );
        // Key absent: Zed's static default (blue dark step_11).
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).text_accent,
            0x70B8FFFF
        );
        // 8-hex authored alpha survives the chain (accent tint blends live).
        let style = serde_json::json!({ "text.accent": "#74ade866" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).text_accent,
            0x74ade866
        );
    }

    #[test]
    fn editor_line_number_falls_back_to_zed_static_default() {
        // Key present.
        let style = serde_json::json!({ "editor.line_number": "#4e5a5f" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_line_number,
            0x4e5a5fff
        );
        // Key absent: the STATIC default (dark_alpha step_10) — `text_dim` no
        // longer leaks into the gutter.
        let style = serde_json::json!({ "text.placeholder": "#aabbcc" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_line_number,
            0xFFFDEE73
        );
        // All absent: same static default.
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).editor_line_number,
            ZED_DARK_DEFAULTS.editor_line_number
        );
        // 8-hex authored alpha survives: line numbers paint over the gutter and
        // blend live at paint time (not a full-bleed surface).
        let style = serde_json::json!({ "editor.line_number": "#4e5a5f80" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_line_number,
            0x4e5a5f80
        );
    }

    #[test]
    fn editor_active_line_number_falls_back_to_zed_static_default() {
        // Key present.
        let style = serde_json::json!({ "editor.active_line_number": "#d0d4da" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_active_line_number,
            0xd0d4daff
        );
        // Key absent: the STATIC default (dark_alpha step_11) — `text` no
        // longer leaks into the active line number.
        let style = serde_json::json!({ "text": "#c8ccd4" });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.editor_active_line_number, 0xFFFCF4B0);
        assert_ne!(palette.editor_active_line_number, palette.text_primary);
        // All absent: same static default.
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).editor_active_line_number,
            ZED_DARK_DEFAULTS.editor_active_line_number
        );
        // 8-hex authored alpha survives (active-line number blends live at paint).
        let style = serde_json::json!({ "editor.active_line_number": "#d0d4da80" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_active_line_number,
            0xd0d4da80
        );
    }

    #[test]
    fn editor_gutter_bg_falls_back_to_zed_static_default() {
        // Key present: distinct from the editor background.
        let style = serde_json::json!({
            "editor.gutter.background": "#282c33",
            "editor.background": "#101010"
        });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_gutter_bg,
            0x282c33ff
        );
        // Key absent: the STATIC default — the resolved `preview_bg` no longer
        // leaks into the gutter (Zed's own defaults happen to make them equal,
        // but an AUTHORED editor.background does not follow).
        let style = serde_json::json!({ "editor.background": "#101010" });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.editor_gutter_bg, 0x111110FF);
        assert_ne!(palette.editor_gutter_bg, palette.preview_bg);
        // All absent: same static default.
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).editor_gutter_bg,
            ZED_DARK_DEFAULTS.editor_gutter_background
        );
        // 8-hex authored alpha is CLAMPED opaque: the gutter is a full-bleed
        // surface painted over the opaque window (see `opaque`).
        let style = serde_json::json!({ "editor.gutter.background": "#282c3380" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).editor_gutter_bg,
            0x282c33ff
        );
    }

    #[test]
    fn hover_row_resolves_own_key_else_static_default() {
        // Key present: `ghost_element.hover` (what Zed's picker rows use) wins.
        let style = serde_json::json!({
            "ghost_element.hover": "#363c46",
            "element.hover": "#111111"
        });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).hover_row,
            0x363c46ff
        );
        // Key absent: the STATIC default — `element.hover` (a different
        // component's token) no longer leaks into picker rows.
        let style = serde_json::json!({ "element.hover": "#111111" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).hover_row,
            ZED_DARK_DEFAULTS.ghost_element_hover
        );
        // All absent: same static default (dark_alpha step_4).
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).hover_row,
            0xFEFEF31B
        );
    }

    #[test]
    fn match_highlight_follows_text_accent_not_search_background() {
        // `match_highlight` (fuzzy tint / checkbox accent) sources `text.accent`.
        let style = serde_json::json!({
            "text.accent": "#74ade8",
            "search.match_background": "#74ade866"
        });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).match_highlight,
            0x74ade8ff
        );
        // The search background no longer leaks into the accent token: without
        // `text.accent` the fallback is Zed's static accent, not the search bg.
        let style = serde_json::json!({ "search.match_background": "#74ade866" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).match_highlight,
            ZED_DARK_DEFAULTS.text_accent
        );
    }

    #[test]
    fn match_highlight_bg_chain_preserves_alpha() {
        // Key present: authored alpha preserved.
        let style = serde_json::json!({ "search.match_background": "#74ade866" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).match_highlight_bg,
            0x74ade866
        );
        // Key absent: next in chain, alpha preserved there too.
        let style = serde_json::json!({ "search.active_match_background": "#11223344" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).match_highlight_bg,
            0x11223344
        );
        // All absent: Zed's static default (dark step_5, opaque — the default
        // itself authors no alpha).
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).match_highlight_bg,
            0x31312EFF
        );
    }

    #[test]
    fn cursor_falls_back_to_text_accent() {
        // Key present.
        let style = serde_json::json!({ "editor.cursor": "#ff0000", "text.accent": "#74ade8" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).cursor,
            0xff0000ff
        );
        // Key absent (e.g. One Dark has no `editor.cursor`): the resolved
        // `text_accent` is the cursor color (derived-from-resolved by design).
        let style = serde_json::json!({ "text.accent": "#74ade8" });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.cursor, 0x74ade8ff);
        assert_eq!(palette.cursor, palette.text_accent);
        // All absent: Zed's static accent default.
        assert_eq!(
            palette_from_style(&serde_json::json!({}), Appearance::Dark).cursor,
            ZED_DARK_DEFAULTS.text_accent
        );
        // 8-hex authored alpha survives (the cursor blends live at paint time).
        let style = serde_json::json!({ "editor.cursor": "#ff000080" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).cursor,
            0xff000080
        );
    }

    #[test]
    fn one_dark_sync_resolves_zed_parity_tokens() {
        // End-to-end against the bundled One Dark JSON: the exact hexes Zed uses.
        let catalog = builtin_theme_catalog().expect("catalog loads");
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &HashMap::new(), &mut theme);
        assert_eq!(theme.border_variant, 0x363c46ff);
        assert_eq!(theme.text_accent, 0x74ade8ff);
        assert_eq!(theme.match_highlight, 0x74ade8ff);
        assert_eq!(theme.hover_row, 0x363c46ff); // ghost_element.hover
        assert_eq!(theme.editor_gutter_bg, 0x282c33ff);
        assert_eq!(theme.editor_line_number, 0x4e5a5fff);
        assert_eq!(theme.editor_active_line_number, 0xd0d4daff);
        // One Dark has no `editor.cursor` key: the cursor lands on the accent.
        assert_eq!(theme.cursor, 0x74ade8ff);
    }

    #[test]
    fn apply_palette_copies_zed_parity_tokens() {
        let palette = Palette {
            border_variant: 0x01010101,
            text_accent: 0x02020202,
            editor_gutter_bg: 0x03030303,
            editor_line_number: 0x04040404,
            editor_active_line_number: 0x05050505,
            cursor: 0x06060606,
            ..Palette::default()
        };
        let mut theme = AppTheme::default();
        apply_palette(&palette, &mut theme);
        assert_eq!(theme.border_variant, 0x01010101);
        assert_eq!(theme.text_accent, 0x02020202);
        assert_eq!(theme.editor_gutter_bg, 0x03030303);
        assert_eq!(theme.editor_line_number, 0x04040404);
        assert_eq!(theme.editor_active_line_number, 0x05050505);
        assert_eq!(theme.cursor, 0x06060606);
    }

    #[test]
    fn apply_palette_copies_new_tokens() {
        let palette = Palette {
            active_line_bg: 0x010101,
            git_created: 0x020202,
            git_modified: 0x030303,
            git_deleted: 0x040404,
            git_conflict: 0x050505,
            git_renamed: 0x060606,
            git_untracked: 0x070707,
            git_ignored: 0x080808,
            ..Palette::default()
        };
        let mut theme = AppTheme::default();
        apply_palette(&palette, &mut theme);
        assert_eq!(theme.active_line_bg, 0x010101);
        assert_eq!(theme.git_created, 0x020202);
        assert_eq!(theme.git_modified, 0x030303);
        assert_eq!(theme.git_deleted, 0x040404);
        assert_eq!(theme.git_conflict, 0x050505);
        assert_eq!(theme.git_renamed, 0x060606);
        assert_eq!(theme.git_untracked, 0x070707);
        assert_eq!(theme.git_ignored, 0x080808);
    }

    #[test]
    fn with_alpha_masks_off_existing_alpha() {
        // Replaces the alpha byte, preserving RGB. The base carries a NONZERO top
        // byte on purpose: the pre-RGBA `(base << 8) | alpha` form would shift RR
        // out and leak GG..into the result, so this case pins the correct masking.
        assert_eq!(with_alpha(0xAABBCCFF, 0x80), 0xAABBCC80);
        // What the buggy `<< 8` form would have produced — must NOT match.
        assert_ne!(with_alpha(0xAABBCCFF, 0x80), (0xAABBCCFFu32 << 8) | 0x80);
        // Zero base and full alpha behave as expected.
        assert_eq!(with_alpha(0x00000000, 0xFF), 0x000000FF);
        assert_eq!(with_alpha(0x12345678, 0x00), 0x12345600);
    }

    #[test]
    fn input_text_falls_back_to_text_primary() {
        // No `input.foreground` key (every bundled theme): input text tracks the
        // resolved `text_primary`, so wiring `input_text` into typed-text
        // rendering is behavior-neutral under those themes.
        let style = serde_json::json!({ "text": "#c8ccd4" });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.input_text, 0xc8ccd4ff);
        assert_eq!(palette.input_text, palette.text_primary);
        // Key present: the theme's distinct input color wins.
        let style = serde_json::json!({ "text": "#c8ccd4", "input.foreground": "#e5e5ea" });
        assert_eq!(
            palette_from_style(&style, Appearance::Dark).input_text,
            0xe5e5eaff
        );
        // All absent: Zed's static `text` default (matching `text_primary`).
        let palette = palette_from_style(&serde_json::json!({}), Appearance::Dark);
        assert_eq!(palette.input_text, ZED_DARK_DEFAULTS.text);
        assert_eq!(palette.input_text, palette.text_primary);
    }

    #[test]
    fn palette_round_trips_every_color_field() {
        // Field-swap guard for the AppTheme -> Palette copy in `palette_from_theme`
        // (what `palette()` delegates to). Each of the 26 color tokens gets a
        // distinct sentinel; a mis-wired copy line (e.g. reading the wrong theme
        // field) fails exactly one assertion. `picker_pane_width` rides along too.
        let theme = AppTheme {
            bg: 0x01010101,
            border: 0x02020202,
            border_variant: 0x03030303,
            selected_row: 0x04040404,
            hover_row: 0x05050505,
            text_primary: 0x06060606,
            text_secondary: 0x07070707,
            text_dim: 0x08080808,
            text_accent: 0x09090909,
            match_highlight: 0x0a0a0a0a,
            match_highlight_bg: 0x0b0b0b0b,
            preview_bg: 0x0c0c0c0c,
            editor_gutter_bg: 0x0d0d0d0d,
            editor_line_number: 0x0e0e0e0e,
            editor_active_line_number: 0x0f0f0f0f,
            input_text: 0x10101010,
            cursor: 0x11111111,
            icon_muted: 0x12121212,
            active_line_bg: 0x14141414,
            git_created: 0x15151515,
            git_modified: 0x16161616,
            git_deleted: 0x17171717,
            git_conflict: 0x18181818,
            git_renamed: 0x19191919,
            git_untracked: 0x1a1a1a1a,
            git_ignored: 0x1b1b1b1b,
            picker_pane_width: Some(321.0),
            ..AppTheme::default()
        };
        let palette = palette_from_theme(&theme);
        assert_eq!(palette.bg, 0x01010101);
        assert_eq!(palette.border, 0x02020202);
        assert_eq!(palette.border_variant, 0x03030303);
        assert_eq!(palette.selected_row, 0x04040404);
        assert_eq!(palette.hover_row, 0x05050505);
        assert_eq!(palette.text_primary, 0x06060606);
        assert_eq!(palette.text_secondary, 0x07070707);
        assert_eq!(palette.text_dim, 0x08080808);
        assert_eq!(palette.text_accent, 0x09090909);
        assert_eq!(palette.match_highlight, 0x0a0a0a0a);
        assert_eq!(palette.match_highlight_bg, 0x0b0b0b0b);
        assert_eq!(palette.preview_bg, 0x0c0c0c0c);
        assert_eq!(palette.editor_gutter_bg, 0x0d0d0d0d);
        assert_eq!(palette.editor_line_number, 0x0e0e0e0e);
        assert_eq!(palette.editor_active_line_number, 0x0f0f0f0f);
        assert_eq!(palette.input_text, 0x10101010);
        assert_eq!(palette.cursor, 0x11111111);
        assert_eq!(palette.icon_muted, 0x12121212);
        assert_eq!(palette.active_line_bg, 0x14141414);
        assert_eq!(palette.git_created, 0x15151515);
        assert_eq!(palette.git_modified, 0x16161616);
        assert_eq!(palette.git_deleted, 0x17171717);
        assert_eq!(palette.git_conflict, 0x18181818);
        assert_eq!(palette.git_renamed, 0x19191919);
        assert_eq!(palette.git_untracked, 0x1a1a1a1a);
        assert_eq!(palette.git_ignored, 0x1b1b1b1b);
        assert_eq!(palette.picker_pane_width, Some(321.0));
    }

    #[test]
    fn zed_override_deep_merges_git_tokens() {
        // A Zed `theme_overrides` payload overriding ONE git key must apply to
        // that token while the base theme's other git keys stay intact (the
        // merge happens on the raw JSON before palette extraction).
        let style = serde_json::json!({
            "background": "#000000",
            "editor.foreground": "#eeeeee",
            "version_control.added": "#27a657",
            "version_control.deleted": "#e06c76"
        });
        let entry = ThemeCatalogEntry {
            appearance: Appearance::Dark,
            palette: palette_from_style(&style, Appearance::Dark),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(ZED_DARK_DEFAULTS.editor_foreground),
            style,
        };
        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({ "version_control.added": "#123456" }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);
        assert_eq!(theme.git_created, 0x123456ff); // overridden
        assert_eq!(theme.git_deleted, 0xe06c76ff); // untouched base key
        assert_eq!(theme.git_untracked, DEFAULT_GIT_UNTRACKED); // fallback intact
    }

    #[test]
    fn fff_config_git_color_wins_over_zed_override() {
        // Same precedence contract as `fff_config_color_wins_over_zed_override`,
        // exercised for a new token: Zed override first, fff `[theme]` last.
        let style = serde_json::json!({ "version_control.added": "#27a657" });
        let entry = ThemeCatalogEntry {
            appearance: Appearance::Dark,
            palette: palette_from_style(&style, Appearance::Dark),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: ZED_DARK_DEFAULTS.editor_foreground,
            style,
        };
        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({ "version_control.added": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);
        assert_eq!(theme.git_created, 0x123456ff); // Zed override applied first

        // fff `[theme].git_created` applied last, exactly as `sync_from_config` does.
        apply_color(&Some("#abcdef".to_string()), &mut theme.git_created);
        assert_eq!(theme.git_created, 0xabcdefff);
    }

    #[test]
    fn full_bleed_surface_tokens_clamp_to_opaque_at_resolution() {
        // A config override like `bg = "#10101080"` or a third-party Zed theme
        // authoring alpha on the full-bleed surfaces would render the picker
        // partially see-through over the opaque OS window. `bg`, `preview_bg`,
        // and `editor_gutter_bg` must resolve fully opaque, while the
        // blend-at-paint tokens (`active_line_bg`, `match_highlight_bg`) keep
        // their authored alpha.
        let style = serde_json::json!({
            "elevated_surface.background": "#10101080",
            "editor.background": "#20202040",
            "editor.gutter.background": "#30303010",
            "editor.active_line.background": "#2f343ebf",
            "search.match_background": "#74ade866"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.bg, 0x101010ff);
        assert_eq!(palette.preview_bg, 0x202020ff);
        assert_eq!(palette.editor_gutter_bg, 0x303030ff);
        // Blend-at-paint tokens keep their authored alpha.
        assert_eq!(palette.active_line_bg, 0x2f343ebf);
        assert_eq!(palette.match_highlight_bg, 0x74ade866);
    }

    #[test]
    fn config_override_clamps_full_bleed_surfaces_opaque() {
        // The fff `[theme]` config-override path (`apply_theme_config_colors`,
        // run last in `sync_from_config`) also forces the full-bleed surfaces
        // opaque after applying a translucent override, while the
        // blend-at-paint tokens keep their alpha.
        let config = ThemeConfig {
            bg: Some("#10101080".to_string()),
            preview_bg: Some("#20202040".to_string()),
            editor_gutter_bg: Some("#30303010".to_string()),
            active_line_bg: Some("#2f343ebf".to_string()),
            ..ThemeConfig::default()
        };
        let mut theme = AppTheme::default();
        apply_theme_config_colors(&config, &mut theme);
        assert_eq!(theme.bg, 0x101010ff);
        assert_eq!(theme.preview_bg, 0x202020ff);
        assert_eq!(theme.editor_gutter_bg, 0x303030ff);
        assert_eq!(theme.active_line_bg, 0x2f343ebf); // blend-at-paint keeps alpha
    }

    #[test]
    fn config_overrides_land_on_matching_zed_parity_fields() {
        // Each `[theme]` override must write its OWN resolved field — a table
        // guard against copy-paste field mismatches in the `apply_color` wiring
        // of `apply_theme_config_colors` (e.g. an override for `border_variant`
        // accidentally writing `border`).
        //
        // Sentinels are mutually distinct so a mis-wired `apply_color` (writing
        // the wrong field) lands the wrong value and trips an assertion. The
        // full-bleed surfaces (`bg`, `preview_bg`, `editor_gutter_bg`) clamp
        // opaque, so their sentinels are covered by the clamp tests above; every
        // other `apply_color` line in `apply_theme_config_colors` is checked here.
        let config = ThemeConfig {
            border: Some("#010101".to_string()),
            border_variant: Some("#111111".to_string()),
            selected_row: Some("#020202".to_string()),
            hover_row: Some("#030303".to_string()),
            text_primary: Some("#040404".to_string()),
            text_secondary: Some("#050505".to_string()),
            text_dim: Some("#060606".to_string()),
            text_accent: Some("#222222".to_string()),
            match_highlight: Some("#070707".to_string()),
            match_highlight_bg: Some("#080808".to_string()),
            editor_gutter_bg: Some("#333333".to_string()),
            editor_line_number: Some("#444444".to_string()),
            editor_active_line_number: Some("#555555".to_string()),
            input_text: Some("#090909".to_string()),
            cursor: Some("#666666".to_string()),
            icon_muted: Some("#0a0a0a".to_string()),
            git_created: Some("#0b0b0b".to_string()),
            git_modified: Some("#0c0c0c".to_string()),
            git_deleted: Some("#0d0d0d".to_string()),
            git_conflict: Some("#0e0e0e".to_string()),
            git_renamed: Some("#0f0f0f".to_string()),
            git_untracked: Some("#1a1a1a".to_string()),
            git_ignored: Some("#1b1b1b".to_string()),
            ..ThemeConfig::default()
        };
        let mut theme = AppTheme::default();
        apply_theme_config_colors(&config, &mut theme);
        assert_eq!(theme.border, 0x010101ff);
        assert_eq!(theme.border_variant, 0x111111ff);
        assert_eq!(theme.selected_row, 0x020202ff);
        assert_eq!(theme.hover_row, 0x030303ff);
        assert_eq!(theme.text_primary, 0x040404ff);
        assert_eq!(theme.text_secondary, 0x050505ff);
        assert_eq!(theme.text_dim, 0x060606ff);
        assert_eq!(theme.text_accent, 0x222222ff);
        assert_eq!(theme.match_highlight, 0x070707ff);
        assert_eq!(theme.match_highlight_bg, 0x080808ff);
        assert_eq!(theme.editor_gutter_bg, 0x333333ff);
        assert_eq!(theme.editor_line_number, 0x444444ff);
        assert_eq!(theme.editor_active_line_number, 0x555555ff);
        assert_eq!(theme.input_text, 0x090909ff);
        assert_eq!(theme.cursor, 0x666666ff);
        assert_eq!(theme.icon_muted, 0x0a0a0aff);
        assert_eq!(theme.git_created, 0x0b0b0bff);
        assert_eq!(theme.git_modified, 0x0c0c0cff);
        assert_eq!(theme.git_deleted, 0x0d0d0dff);
        assert_eq!(theme.git_conflict, 0x0e0e0eff);
        assert_eq!(theme.git_renamed, 0x0f0f0fff);
        assert_eq!(theme.git_untracked, 0x1a1a1aff);
        assert_eq!(theme.git_ignored, 0x1b1b1bff);
    }

    #[test]
    fn missing_keys_resolve_to_per_appearance_static_defaults() {
        // Zed semantics: a missing key resolves to the STATIC default for the
        // theme's appearance — never to another key's resolved value.
        let empty = serde_json::json!({});

        let dark = palette_from_style(&empty, Appearance::Dark);
        assert_eq!(dark.bg, 0x191918FF); // elevated_surface: sand dark step_2
        assert_eq!(dark.preview_bg, 0x111110FF); // editor bg: sand dark step_1
        assert_eq!(dark.selected_row, 0xFBFBEB23); // sand dark_alpha step_5
        assert_eq!(dark.hover_row, 0xFEFEF31B); // sand dark_alpha step_4
        assert_eq!(dark.border_variant, 0x31312EFF); // sand dark step_5

        let light = palette_from_style(&empty, Appearance::Light);
        assert_eq!(light.bg, 0xF9F9F8FF); // elevated_surface: sand light step_2
        assert_eq!(light.preview_bg, 0xFDFDFCFF); // editor bg: sand light step_1
        assert_eq!(light.selected_row, 0x1F180021); // sand light_alpha step_5
        assert_eq!(light.hover_row, 0x20100010); // sand light_alpha step_3
        assert_eq!(light.border_variant, 0xE2E1DEFF); // sand light step_5
    }

    #[test]
    fn theme_without_elevated_surface_renders_two_tone_panes() {
        // RustRover-Dark-shaped fixture: the theme (plus the user's
        // theme_overrides) flattens `background`/`surface.background`/
        // `editor.background` to #1e1f22 but never authors
        // `elevated_surface.background`. Zed resolves the missing key to the
        // static dark default and shows a two-tone picker; the old cross-key
        // chain collapsed both panes to #1e1f22.
        let style = serde_json::json!({
            "background": "#1e1f22",
            "surface.background": "#1e1f22",
            "editor.background": "#1e1f22"
        });
        let palette = palette_from_style(&style, Appearance::Dark);
        assert_eq!(palette.preview_bg, 0x1e1f22ff);
        assert_eq!(palette.bg, ZED_DARK_DEFAULTS.elevated_surface_background);
        assert_ne!(palette.bg, palette.preview_bg);
    }

    #[test]
    fn default_palette_is_the_zed_dark_table() {
        // The no-theme hardcoded palette IS Zed's dark default table: an empty
        // style resolved dark must equal `Palette::default()` field for field.
        assert_eq!(
            Palette::default(),
            palette_from_style(&serde_json::json!({}), Appearance::Dark)
        );
    }

    #[test]
    fn appearance_parses_leniently_with_dark_fallback() {
        assert_eq!(appearance_from_json(Some("light")), Appearance::Light);
        assert_eq!(appearance_from_json(Some("Light")), Appearance::Light);
        assert_eq!(appearance_from_json(Some("dark")), Appearance::Dark);
        // Unknown or absent values degrade to Dark instead of erroring.
        assert_eq!(appearance_from_json(Some("solarized")), Appearance::Dark);
        assert_eq!(appearance_from_json(None), Appearance::Dark);
    }

    #[test]
    fn catalog_variant_appearance_selects_default_table() {
        // The family loader threads each variant's `appearance` into palette
        // resolution: the same missing `elevated_surface.background` resolves
        // to the light table for a light variant and the dark table otherwise
        // (including when `appearance` is absent).
        let mut catalog = HashMap::new();
        load_theme_family_contents(
            "test family",
            r##"{ "themes": [
                { "name": "Test Light", "appearance": "light", "style": { "editor.background": "#ffffff" } },
                { "name": "Test Dark", "appearance": "dark", "style": { "editor.background": "#000000" } },
                { "name": "Test Unspecified", "style": { "editor.background": "#000000" } }
            ] }"##,
            &mut catalog,
        )
        .expect("family loads");

        let light = catalog.get("test light").expect("light variant present");
        assert_eq!(light.appearance, Appearance::Light);
        assert_eq!(light.palette.bg, 0xF9F9F8FF); // light elevated_surface default
        assert_eq!(
            light.syntax_default_color,
            ZED_LIGHT_DEFAULTS.editor_foreground
        );

        let dark = catalog.get("test dark").expect("dark variant present");
        assert_eq!(dark.appearance, Appearance::Dark);
        assert_eq!(dark.palette.bg, 0x191918FF); // dark elevated_surface default

        let unspecified = catalog
            .get("test unspecified")
            .expect("unspecified variant present");
        assert_eq!(unspecified.appearance, Appearance::Dark);
        assert_eq!(unspecified.palette.bg, 0x191918FF);
    }

    #[test]
    fn syntax_style_bg_parses_background_color() {
        let style = SyntaxStyle {
            color: Some("#112233".to_string()),
            background_color: Some("#445566".to_string()),
            ..SyntaxStyle::default()
        };
        assert_eq!(syntax_style_bg(&style), Some(0x445566ff));
        // A missing or unparsable background color yields None.
        assert_eq!(syntax_style_bg(&SyntaxStyle::default()), None);
        let bad = SyntaxStyle {
            background_color: Some("not-a-color".to_string()),
            ..SyntaxStyle::default()
        };
        assert_eq!(syntax_style_bg(&bad), None);
    }

    #[test]
    fn syntax_style_from_value_captures_background_color() {
        let value = serde_json::json!({
            "color": "#aabbcc",
            "background_color": "#001122",
            "font_style": "italic"
        });
        let map = value.as_object().expect("object");
        let style = SyntaxStyle::from_value(map);
        assert_eq!(style.background_color.as_deref(), Some("#001122"));
        assert_eq!(syntax_style_bg(&style), Some(0x001122ff));
    }

    fn icon_fixture() -> FileIconTheme {
        let file_stems = HashMap::from_iter([("Dockerfile".to_string(), "docker".to_string())]);
        let file_suffixes = HashMap::from_iter([
            ("rs".to_string(), "rust".to_string()),
            ("tar.gz".to_string(), "archive".to_string()),
            ("gitignore".to_string(), "git".to_string()),
            ("d.ts".to_string(), "typescript".to_string()),
        ]);
        let file_icons = HashMap::from_iter(
            ["docker", "rust", "archive", "git", "typescript", "default"]
                .into_iter()
                .map(|key| {
                    (
                        key.to_string(),
                        FileIconDefinition {
                            path: FileIconPath::Embedded(format!("icons/{key}.svg").into()),
                        },
                    )
                }),
        );
        FileIconTheme {
            file_stems,
            file_suffixes,
            file_icons,
        }
    }

    fn resolved_icon(theme: &FileIconTheme, path: &str) -> Option<String> {
        match theme.file_icon_for_path(Path::new(path))? {
            FileIconPath::Embedded(p) | FileIconPath::External(p) => Some(p.to_string()),
        }
    }

    #[test]
    fn icon_keys_by_association_inverts_the_association_table() {
        let table: &[(&str, &[&str])] =
            &[("rust", &["rs"]), ("docker", &["Dockerfile", "dockerfile"])];
        let map = icon_keys_by_association(table);
        assert_eq!(map.get("rs").map(String::as_str), Some("rust"));
        assert_eq!(map.get("Dockerfile").map(String::as_str), Some("docker"));
        assert_eq!(map.get("dockerfile").map(String::as_str), Some("docker"));
        assert_eq!(map.get("missing"), None);
    }

    #[test]
    fn multiple_extensions_returns_everything_after_the_first_dot() {
        assert_eq!(
            multiple_extensions(Path::new("a.tar.gz")).as_deref(),
            Some("tar.gz")
        );
        assert_eq!(
            multiple_extensions(Path::new("types.d.ts")).as_deref(),
            Some("d.ts")
        );
        assert_eq!(
            multiple_extensions(Path::new("main.rs")).as_deref(),
            Some("rs")
        );
        // A bare name with no dot has no suffix at all.
        assert_eq!(multiple_extensions(Path::new("Makefile")), None);
    }

    #[test]
    fn extension_or_hidden_file_name_prefers_hidden_name_then_extension() {
        assert_eq!(
            extension_or_hidden_file_name(Path::new(".gitignore")).as_deref(),
            Some("gitignore")
        );
        assert_eq!(
            extension_or_hidden_file_name(Path::new("main.rs")).as_deref(),
            Some("rs")
        );
        assert_eq!(extension_or_hidden_file_name(Path::new("Makefile")), None);
    }

    #[test]
    fn get_icon_for_type_returns_none_for_unknown_key() {
        let theme = icon_fixture();
        assert_eq!(theme.get_icon_for_type("nope"), None);
        assert_eq!(
            theme.get_icon_for_type("rust"),
            Some(FileIconPath::Embedded("icons/rust.svg".into()))
        );
    }

    #[test]
    fn file_icon_for_path_walks_the_priority_chain() {
        let theme = icon_fixture();
        // Stem (full file name) wins over extension resolution.
        assert_eq!(
            resolved_icon(&theme, "app/Dockerfile").as_deref(),
            Some("icons/docker.svg")
        );
        // Bare extension.
        assert_eq!(
            resolved_icon(&theme, "src/main.rs").as_deref(),
            Some("icons/rust.svg")
        );
        // Multi-component suffixes resolve via the dotted-suffix walk.
        assert_eq!(
            resolved_icon(&theme, "backup.tar.gz").as_deref(),
            Some("icons/archive.svg")
        );
        assert_eq!(
            resolved_icon(&theme, "lib/types.d.ts").as_deref(),
            Some("icons/typescript.svg")
        );
        // Hidden dotfile resolves by its name-without-dots.
        assert_eq!(
            resolved_icon(&theme, "repo/.gitignore").as_deref(),
            Some("icons/git.svg")
        );
        // Unknown extension and bare extensionless names fall back to default.
        assert_eq!(
            resolved_icon(&theme, "notes.unknownext").as_deref(),
            Some("icons/default.svg")
        );
        assert_eq!(
            resolved_icon(&theme, "README").as_deref(),
            Some("icons/default.svg")
        );
    }

    #[test]
    fn apply_icon_theme_variant_extends_maps_with_external_paths() {
        let mut theme = FileIconTheme {
            file_stems: HashMap::new(),
            file_suffixes: HashMap::new(),
            file_icons: HashMap::new(),
        };
        let variant = IconThemeVariant {
            name: "Fixture".to_string(),
            file_stems: HashMap::from_iter([("Makefile".to_string(), "make".to_string())]),
            file_suffixes: HashMap::from_iter([("rs".to_string(), "rust".to_string())]),
            file_icons: HashMap::from_iter([(
                "rust".to_string(),
                IconDefinitionContent {
                    path: "rust.svg".into(),
                },
            )]),
        };
        apply_icon_theme_variant(&variant, Path::new("/icons"), &mut theme);

        assert_eq!(
            theme.file_stems.get("Makefile").map(String::as_str),
            Some("make")
        );
        assert_eq!(
            theme.file_suffixes.get("rs").map(String::as_str),
            Some("rust")
        );
        // Icon paths are rewritten to registered external asset handles.
        assert!(matches!(
            theme.file_icons.get("rust").map(|def| &def.path),
            Some(FileIconPath::External(_))
        ));
    }

    #[test]
    fn merge_zed_settings_fills_only_unset_fields() {
        let mut settings = ZedSettings {
            ui_font_family: Some("Custom UI".to_string()),
            ui_font_size: Some(21.0),
            theme: Some(ThemeSelection::Static("My Theme".to_string())),
            ..ZedSettings::default()
        };
        merge_zed_settings(&mut settings, hardcoded_zed_settings_defaults());

        // Explicitly set fields are left untouched.
        assert_eq!(settings.ui_font_family.as_deref(), Some("Custom UI"));
        assert_eq!(settings.ui_font_size, Some(21.0));
        assert!(matches!(
            settings.theme.as_ref(),
            Some(ThemeSelection::Static(name)) if name == "My Theme"
        ));
        // Unset fields are filled from the defaults.
        assert_eq!(
            settings.buffer_font_family.as_deref(),
            Some(DEFAULT_BUFFER_FONT_FAMILY)
        );
        assert_eq!(settings.buffer_font_size, Some(DEFAULT_BUFFER_FONT_SIZE));
        assert!(settings.icon_theme.is_some());
    }

    #[test]
    fn merge_zed_settings_populates_empty_settings_from_defaults() {
        let mut settings = ZedSettings::default();
        merge_zed_settings(&mut settings, hardcoded_zed_settings_defaults());
        assert_eq!(
            settings.ui_font_family.as_deref(),
            Some(DEFAULT_UI_FONT_FAMILY)
        );
        assert_eq!(
            settings.buffer_font_family.as_deref(),
            Some(DEFAULT_BUFFER_FONT_FAMILY)
        );
        assert_eq!(settings.ui_font_size, Some(DEFAULT_UI_FONT_SIZE));
        assert_eq!(settings.buffer_font_size, Some(DEFAULT_BUFFER_FONT_SIZE));
        assert!(settings.theme.is_some());
        assert!(settings.icon_theme.is_some());
    }
}
