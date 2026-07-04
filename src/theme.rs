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
use crate::config::{AppConfig, DEFAULT_PICKER_PANE_WIDTH};

const DEFAULT_BG: u32 = 0x1C1C1E;
const DEFAULT_BORDER: u32 = 0x2F2F31;
const DEFAULT_SELECTED_ROW: u32 = 0x2C3F59;
const DEFAULT_HOVER_ROW: u32 = 0x2A2A2C;
const DEFAULT_TEXT_PRIMARY: u32 = 0xFFFFFF;
const DEFAULT_TEXT_SECONDARY: u32 = 0x8E8E93;
const DEFAULT_TEXT_DIM: u32 = 0x6C6C70;
const DEFAULT_STATUS_BAR_BG: u32 = 0x18181A;
const DEFAULT_MATCH_HIGHLIGHT: u32 = 0x4A9EFF;
const DEFAULT_PREVIEW_BG: u32 = 0x161618;
const DEFAULT_UI_FONT_FAMILY: &str = ".SystemUIFont";
const DEFAULT_BUFFER_FONT_FAMILY: &str = "UbuntuMono Nerd Font";
pub const DEFAULT_UI_FONT_SIZE: f32 = 16.0;
pub const DEFAULT_BUFFER_FONT_SIZE: f32 = 15.0;
static ACTIVE_THEME: OnceLock<RwLock<AppTheme>> = OnceLock::new();
static ACTIVE_FILE_ICON_THEME: OnceLock<RwLock<FileIconTheme>> = OnceLock::new();
static THEME_VERSION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub bg: u32,
    pub border: u32,
    pub selected_row: u32,
    pub hover_row: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_dim: u32,
    pub status_bar_bg: u32,
    pub match_highlight: u32,
    pub match_highlight_bg: u32,
    pub preview_bg: u32,
    pub input_bg: u32,
    pub input_text: u32,
    pub cursor: u32,
    pub cursor_selection: u32,
    pub icon_muted: u32,
    pub icon_accent: u32,
    pub picker_pane_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppTheme {
    pub bg: u32,
    pub border: u32,
    pub selected_row: u32,
    pub hover_row: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_dim: u32,
    pub status_bar_bg: u32,
    pub match_highlight: u32,
    pub match_highlight_bg: u32,
    pub preview_bg: u32,
    pub input_bg: u32,
    pub input_text: u32,
    pub cursor: u32,
    pub cursor_selection: u32,
    pub icon_muted: u32,
    pub icon_accent: u32,
    pub ui_font_family: Option<String>,
    pub buffer_font_family: Option<String>,
    pub ui_font_size: f32,
    pub buffer_font_size: f32,
    pub picker_pane_width: f32,
    pub syntax_styles: Vec<(String, SyntaxStyle)>,
    pub syntax_default_color: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyntaxRenderStyle {
    pub color: u32,
    pub italic: bool,
    pub bold: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg: DEFAULT_BG,
            border: DEFAULT_BORDER,
            selected_row: DEFAULT_SELECTED_ROW,
            hover_row: DEFAULT_HOVER_ROW,
            text_primary: DEFAULT_TEXT_PRIMARY,
            text_secondary: DEFAULT_TEXT_SECONDARY,
            text_dim: DEFAULT_TEXT_DIM,
            status_bar_bg: DEFAULT_STATUS_BAR_BG,
            match_highlight: DEFAULT_MATCH_HIGHLIGHT,
            match_highlight_bg: 0x2C4870,
            preview_bg: DEFAULT_PREVIEW_BG,
            input_bg: 0x232326,
            input_text: 0xE5E5EA,
            cursor: 0x0A84FF,
            cursor_selection: 0x0A84FF44,
            icon_muted: DEFAULT_TEXT_SECONDARY,
            icon_accent: DEFAULT_MATCH_HIGHLIGHT,
            picker_pane_width: DEFAULT_PICKER_PANE_WIDTH,
        }
    }
}

impl Default for AppTheme {
    fn default() -> Self {
        Self {
            bg: DEFAULT_BG,
            border: DEFAULT_BORDER,
            selected_row: DEFAULT_SELECTED_ROW,
            hover_row: DEFAULT_HOVER_ROW,
            text_primary: DEFAULT_TEXT_PRIMARY,
            text_secondary: DEFAULT_TEXT_SECONDARY,
            text_dim: DEFAULT_TEXT_DIM,
            status_bar_bg: DEFAULT_STATUS_BAR_BG,
            match_highlight: DEFAULT_MATCH_HIGHLIGHT,
            match_highlight_bg: 0x2C4870,
            preview_bg: DEFAULT_PREVIEW_BG,
            input_bg: 0x232326,
            input_text: 0xE5E5EA,
            cursor: 0x0A84FF,
            cursor_selection: 0x0A84FF44,
            icon_muted: DEFAULT_TEXT_SECONDARY,
            icon_accent: DEFAULT_MATCH_HIGHLIGHT,
            ui_font_family: Some(DEFAULT_UI_FONT_FAMILY.to_string()),
            buffer_font_family: Some(DEFAULT_BUFFER_FONT_FAMILY.to_string()),
            ui_font_size: DEFAULT_UI_FONT_SIZE,
            buffer_font_size: DEFAULT_BUFFER_FONT_SIZE,
            picker_pane_width: DEFAULT_PICKER_PANE_WIDTH,
            syntax_styles: Vec::new(),
            syntax_default_color: DEFAULT_TEXT_PRIMARY,
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
    let theme = current();
    Palette {
        bg: theme.bg,
        border: theme.border,
        selected_row: theme.selected_row,
        hover_row: theme.hover_row,
        text_primary: theme.text_primary,
        text_secondary: theme.text_secondary,
        text_dim: theme.text_dim,
        status_bar_bg: theme.status_bar_bg,
        match_highlight: theme.match_highlight,
        match_highlight_bg: theme.match_highlight_bg,
        preview_bg: theme.preview_bg,
        input_bg: theme.input_bg,
        input_text: theme.input_text,
        cursor: theme.cursor,
        cursor_selection: theme.cursor_selection,
        icon_muted: theme.icon_muted,
        icon_accent: theme.icon_accent,
        picker_pane_width: theme.picker_pane_width,
    }
}

pub fn syntax_color(capture_name: &str) -> u32 {
    match active_theme_lock().read() {
        Ok(theme) => theme.syntax_color(capture_name),
        Err(_) => DEFAULT_TEXT_PRIMARY,
    }
}

pub fn syntax_render_style(capture_name: &str) -> SyntaxRenderStyle {
    match active_theme_lock().read() {
        Ok(theme) => theme.syntax_render_style(capture_name),
        Err(_) => SyntaxRenderStyle {
            color: DEFAULT_TEXT_PRIMARY,
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
    {
        if !name.trim().is_empty() {
            apply_theme_with_overrides(name, catalog, &overrides, &mut resolved);
        }
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
    if config.picker_pane_width.is_finite() && config.picker_pane_width > 0.0 {
        resolved.picker_pane_width = config.picker_pane_width;
    }
    apply_color(&config.theme.bg, &mut resolved.bg);
    apply_color(&config.theme.border, &mut resolved.border);
    apply_color(&config.theme.selected_row, &mut resolved.selected_row);
    apply_color(&config.theme.hover_row, &mut resolved.hover_row);
    apply_color(&config.theme.text_primary, &mut resolved.text_primary);
    apply_color(&config.theme.text_secondary, &mut resolved.text_secondary);
    apply_color(&config.theme.text_dim, &mut resolved.text_dim);
    apply_color(&config.theme.status_bar_bg, &mut resolved.status_bar_bg);
    apply_color(&config.theme.match_highlight, &mut resolved.match_highlight);
    apply_color(
        &config.theme.match_highlight_bg,
        &mut resolved.match_highlight_bg,
    );
    apply_color(&config.theme.preview_bg, &mut resolved.preview_bg);
    apply_color(&config.theme.input_bg, &mut resolved.input_bg);
    apply_color(&config.theme.input_text, &mut resolved.input_text);
    apply_color(&config.theme.cursor, &mut resolved.cursor);
    apply_color(
        &config.theme.cursor_selection,
        &mut resolved.cursor_selection,
    );
    apply_color(&config.theme.icon_muted, &mut resolved.icon_muted);
    apply_color(&config.theme.icon_accent, &mut resolved.icon_accent);

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
        return home_dir().join("Library/Application Support/Zed/extensions/installed");
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(data_home).join("zed/extensions/installed");
        }

        home_dir().join(".local/share/zed/extensions/installed")
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
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
    let mut catalog = HashMap::new();
    load_builtin_theme_catalog(&mut catalog)?;
    load_installed_theme_catalog(&mut catalog)?;
    load_local_theme_catalog(&mut catalog)?;
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
        let entry = ThemeCatalogEntry {
            style: variant.style.clone(),
            palette: palette_from_style(&variant.style),
            syntax_styles: syntax_styles_from_style(&variant.style),
            syntax_default_color: color_from_style(&variant.style, "editor.foreground")
                .or_else(|| color_from_style(&variant.style, "text"))
                .unwrap_or(DEFAULT_TEXT_PRIMARY),
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
        apply_style_to_theme(&merged, theme);
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
    theme.selected_row = palette.selected_row;
    theme.hover_row = palette.hover_row;
    theme.text_primary = palette.text_primary;
    theme.text_secondary = palette.text_secondary;
    theme.text_dim = palette.text_dim;
    theme.status_bar_bg = palette.status_bar_bg;
    theme.match_highlight = palette.match_highlight;
    theme.match_highlight_bg = palette.match_highlight_bg;
    theme.preview_bg = palette.preview_bg;
    theme.input_bg = palette.input_bg;
    theme.input_text = palette.input_text;
    theme.cursor = palette.cursor;
    theme.cursor_selection = palette.cursor_selection;
    theme.icon_muted = palette.icon_muted;
    theme.icon_accent = palette.icon_accent;
}

fn apply_catalog_entry(entry: &ThemeCatalogEntry, theme: &mut AppTheme) {
    apply_palette(&entry.palette, theme);
    theme.syntax_styles = entry.syntax_styles.clone();
    theme.syntax_default_color = entry.syntax_default_color;
}

fn apply_style_to_theme(style: &Value, theme: &mut AppTheme) {
    apply_palette(&palette_from_style(style), theme);
    theme.syntax_styles = syntax_styles_from_style(style);
    theme.syntax_default_color = color_from_style(style, "editor.foreground")
        .or_else(|| color_from_style(style, "text"))
        .unwrap_or(DEFAULT_TEXT_PRIMARY);
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

fn palette_from_style(style: &Value) -> Palette {
    Palette {
        bg: color_from_style(style, "background")
            .or_else(|| color_from_style(style, "surface.background"))
            .or_else(|| color_from_style(style, "editor.background"))
            .unwrap_or(DEFAULT_BG),
        border: color_from_style(style, "border").unwrap_or(DEFAULT_BORDER),
        selected_row: color_from_style(style, "ghost_element.selected")
            .or_else(|| color_from_style(style, "elevated_surface.background"))
            .or_else(|| color_from_style(style, "drop_target.background"))
            .or_else(|| color_from_style(style, "element.selected"))
            .or_else(|| color_from_style(style, "element.active"))
            .unwrap_or(DEFAULT_SELECTED_ROW),
        hover_row: color_from_style(style, "element.hover").unwrap_or(DEFAULT_HOVER_ROW),
        text_primary: color_from_style(style, "text").unwrap_or(DEFAULT_TEXT_PRIMARY),
        text_secondary: color_from_style(style, "text.muted")
            .or_else(|| color_from_style(style, "icon.muted"))
            .unwrap_or(DEFAULT_TEXT_SECONDARY),
        text_dim: color_from_style(style, "text.placeholder")
            .or_else(|| color_from_style(style, "text.disabled"))
            .or_else(|| color_from_style(style, "icon.placeholder"))
            .unwrap_or(DEFAULT_TEXT_DIM),
        status_bar_bg: color_from_style(style, "status_bar.background")
            .or_else(|| color_from_style(style, "title_bar.background"))
            .unwrap_or(DEFAULT_STATUS_BAR_BG),
        match_highlight: color_from_style(style, "search.match_background")
            .or_else(|| color_from_style(style, "search.active_match_background"))
            .or_else(|| color_from_style(style, "text.accent"))
            .unwrap_or(DEFAULT_MATCH_HIGHLIGHT),
        match_highlight_bg: color_from_style(style, "search.match_background")
            .or_else(|| color_from_style(style, "search.active_match_background"))
            .unwrap_or(0x2C4870),
        preview_bg: color_from_style(style, "editor.background")
            .or_else(|| color_from_style(style, "surface.background"))
            .unwrap_or(DEFAULT_PREVIEW_BG),
        input_bg: color_from_style(style, "input.background").unwrap_or(0x232326),
        input_text: color_from_style(style, "input.foreground").unwrap_or(0xE5E5EA),
        cursor: color_from_style(style, "editor.cursor").unwrap_or(0x0A84FF),
        cursor_selection: color_from_style(style, "editor.selectionBackground")
            .unwrap_or(0x0A84FF44),
        icon_muted: color_from_style(style, "icon.muted")
            .or_else(|| color_from_style(style, "icon.placeholder"))
            .or_else(|| color_from_style(style, "text.muted"))
            .unwrap_or(DEFAULT_TEXT_SECONDARY),
        icon_accent: color_from_style(style, "icon.accent")
            .or_else(|| color_from_style(style, "text.accent"))
            .unwrap_or(DEFAULT_MATCH_HIGHLIGHT),
        picker_pane_width: DEFAULT_PICKER_PANE_WIDTH,
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
    style.color.as_deref().and_then(parse_color_rgb)
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
        .and_then(parse_color_rgb)
}

fn apply_color(source: &Option<String>, target: &mut u32) {
    if let Some(color) = source.as_deref().and_then(parse_color_rgb) {
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

fn parse_color_rgb(color: &str) -> Option<u32> {
    let color = color.trim();
    let color = color.strip_prefix('#').unwrap_or(color);

    match color.len() {
        3 => {
            let mut expanded = String::with_capacity(6);
            for ch in color.chars().take(3) {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded, 16).ok()
        }
        4 => {
            let mut expanded = String::with_capacity(8);
            for ch in color.chars().take(4) {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded[..6], 16).ok()
        }
        6 => u32::from_str_radix(color, 16).ok(),
        8 => u32::from_str_radix(&color[..6], 16).ok(),
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
        assert_eq!(parse_color_rgb("#ff00aa"), Some(0xff00aa));
        assert_eq!(parse_color_rgb("#ff00aaff"), Some(0xff00aa));
        assert_eq!(parse_color_rgb("#f0a"), Some(0xff00aa));
        assert_eq!(parse_color_rgb("#f0ab"), Some(0xff00aa));
        assert_eq!(parse_color_rgb("#1234"), Some(0x112233));
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
            0x222222
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
            syntax_default_color: 0xddeeff,
            ..AppTheme::default()
        };

        assert_eq!(theme.syntax_color("constant"), 0x112233);
        assert_eq!(theme.syntax_color("constructor"), 0x112233);
        assert_eq!(theme.syntax_color("type"), 0x112233);
        assert_eq!(theme.syntax_color("punctuation"), 0xddeeff);
        assert_eq!(theme.syntax_color("punctuation.bracket"), 0xddeeff);
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
        let catalog = load_theme_catalog().expect("theme catalog should load");

        assert!(catalog.contains_key("ayu dark"));
        assert!(catalog.contains_key("gruvbox dark"));
        assert!(catalog.contains_key("one dark"));
    }

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
        let catalog = load_theme_catalog().expect("catalog loads");

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
                "background": "#123456",
                "syntax": { "keyword": { "color": "#0f0f0f" } }
            }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456); // overridden color
        assert_eq!(theme.syntax_color("keyword"), 0x0f0f0f); // overridden token
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
            "background": "#000000",
            "editor.foreground": "#eeeeee",
            "syntax": {
                "keyword": { "color": "#111111" },
                "string": { "color": "#222222" }
            }
        });
        let entry = ThemeCatalogEntry {
            palette: palette_from_style(&style),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(DEFAULT_TEXT_PRIMARY),
            style,
        };

        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        // Override only `keyword` and `background`; leave `string` untouched.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({
                "background": "#333333",
                "syntax": { "keyword": { "color": "#333333" } }
            }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);

        assert_eq!(theme.syntax_color("keyword"), 0x333333); // overridden token
        assert_eq!(theme.syntax_color("string"), 0x222222); // untouched base token
        assert_eq!(theme.bg, 0x333333); // overridden background
    }

    #[test]
    fn fff_config_color_wins_over_zed_override() {
        // Asserts the documented precedence contract (Zed override first, then fff
        // `[theme]` config last) rather than calling `sync_from_config` directly,
        // which would require a `&mut App` (gpui) that is impractical to wire up in
        // a unit test. This manually sequences `apply_theme_with_overrides` then
        // `apply_color` exactly as `sync_from_config` does, so the explicit fff
        // config must win for the same field.
        let catalog = load_theme_catalog().expect("catalog loads");
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("One Dark"),
            serde_json::json!({ "background": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456); // Zed override applied first

        // fff `[theme].bg = "#abcdef"` applied last, exactly as `sync_from_config` does.
        apply_color(&Some("#abcdef".to_string()), &mut theme.bg);
        assert_eq!(theme.bg, 0xabcdef); // explicit fff config wins over the Zed override
    }

    #[test]
    fn override_for_other_theme_does_not_apply() {
        let catalog = load_theme_catalog().expect("catalog loads");

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
        assert_ne!(theme.bg, 0x123456); // One Dark selected; Ayu override must NOT apply
        // The result must equal the real, un-overridden One Dark base color,
        // proving the Ayu-keyed override was correctly ignored.
        assert_eq!(theme.bg, base_bg);
    }

    #[test]
    fn no_override_matches_plain_catalog_application() {
        let catalog = load_theme_catalog().expect("catalog loads");
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
        assert_eq!(syntax_style_color(keyword), Some(0xaabbcc));
    }

    #[test]
    fn mixed_case_override_key_applies_via_normalized_overrides() {
        let catalog = load_theme_catalog().expect("catalog loads");
        // Raw, mixed-case override key as it might appear in Zed's settings.json.
        let mut raw_overrides = HashMap::new();
        raw_overrides.insert(
            "ONE DARK".to_string(),
            serde_json::json!({ "background": "#123456" }),
        );
        let overrides = normalized_theme_overrides(&raw_overrides);

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("One Dark", &catalog, &overrides, &mut theme);
        // Case-insensitive match works end-to-end: the override applied.
        assert_eq!(theme.bg, 0x123456);
    }

    #[test]
    fn apply_theme_with_overrides_leaves_theme_unchanged_when_absent() {
        let catalog = load_theme_catalog().expect("catalog loads");

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
        let catalog = load_theme_catalog().expect("catalog loads");
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
            serde_json::json!({ "background": "#123456" }),
        );
        let mut theme = AppTheme::default();
        apply_theme_with_overrides(&resolved, &catalog, &overrides, &mut theme);
        assert_eq!(theme.bg, 0x123456);

        // Capture the resolved variant's real base bg (apply One Dark with EMPTY
        // overrides) so the negative half can prove the base was applied, not just
        // that the wrong-variant sentinel is absent.
        let mut base = AppTheme::default();
        apply_theme_with_overrides(&resolved, &catalog, &HashMap::new(), &mut base);
        let base_bg = base.bg;

        // An override keyed on the OTHER variant ("One Light") does not apply; the
        // resolved base ("One Dark") is applied via `apply_catalog_entry` instead.
        let sentinel = 0x654321;
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
            palette: palette_from_style(&style),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(DEFAULT_TEXT_PRIMARY),
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
        assert_eq!(theme.syntax_color("string"), 0x44aa55);
        // The malformed color is unparseable, so `keyword` resolves to the theme's
        // default foreground color (0xeeeeee), not the bogus override.
        assert_eq!(theme.syntax_default_color, 0xeeeeee);
        assert_eq!(theme.syntax_color("keyword"), 0xeeeeee);
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
            "background": "#000000",
            "editor.foreground": "#eeeeee",
            "syntax": {
                "keyword": { "color": "#111111" },
                "string": { "color": "#222222" }
            }
        });
        let entry = ThemeCatalogEntry {
            palette: palette_from_style(&style),
            syntax_styles: syntax_styles_from_style(&style),
            syntax_default_color: color_from_style(&style, "editor.foreground")
                .or_else(|| color_from_style(&style, "text"))
                .unwrap_or(DEFAULT_TEXT_PRIMARY),
            style,
        };

        let mut catalog = HashMap::new();
        catalog.insert(normalize_name("Test Theme"), entry);

        // `keyword.color` and `background` are nulled (no-op); `string` is a real
        // sibling override that must still apply.
        let mut overrides = HashMap::new();
        overrides.insert(
            normalize_name("Test Theme"),
            serde_json::json!({
                "background": null,
                "syntax": {
                    "keyword": { "color": null },
                    "string": { "color": "#44aa55" }
                }
            }),
        );

        let mut theme = AppTheme::default();
        apply_theme_with_overrides("Test Theme", &catalog, &overrides, &mut theme);

        // Null left the base values untouched.
        assert_eq!(theme.syntax_color("keyword"), 0x111111);
        assert_eq!(theme.bg, 0x000000);
        // The sibling non-null override still applied.
        assert_eq!(theme.syntax_color("string"), 0x44aa55);
    }
}
