# fff-gpui

A fast, keyboard-driven file finder for macOS built on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) — the same UI framework that powers [Zed](https://zed.dev). It runs as a system-wide overlay you can summon instantly with a keybind, and integrates seamlessly into Zed as a custom task.

Under the hood it uses [fff](https://crates.io/crates/fff-search) for fuzzy file search and grep, with frecency-based ranking so the files you actually use rise to the top.

<img width="1072" height="633" alt="Screenshot 2026-05-05 at 4 49 11 PM" src="https://github.com/user-attachments/assets/dec3811f-2de0-4bd3-b90b-812d9c36d124" />

## Features

- Fuzzy file search and grep across your project
- Grep results grouped per match under collapsible file headers, with syntax-highlighted match lines
- Multi-select mode with checkboxes — mark individual matches and open each file once at its first marked match
- Frecency ranking — frequently and recently opened files are prioritised
- Syntax-highlighted file preview
- Resizable results/preview split — drag the divider, double-click to reset
- Per-row git status indicators, colored from your theme
- Global keybind support for system-wide access
- Deep Zed integration via custom tasks — works across all projects

<details>
<summary>
<h2>Installation</h2>
</summary>

### Homebrew (recommended)

Apple Silicon and Intel supported
```sh
brew tap th0jensen/fff-gpui
brew install fff-gpui
brew services start fff-gpui
```

### Build from source

**Requirements:**
- macOS (Apple Silicon and Intel)
- Latest stable Rust via [rustup](https://rustup.rs)
- Xcode Command Line Tools (`xcode-select --install`)
- CMake ([required by wasmtime](https://docs.rs/wasmtime-c-api-impl/latest/wasmtime_c_api/))
- Zig 0.16.0 ([required by zlob](https://crates.io/crates/zlob))

To compile without Zig, disable zlob in `Cargo.toml`. This will lead to slightly slower performance, but it's not required for the app to work.

In `Cargo.toml`, replace the `-` lines with the `+` lines:

```diff
+ fff-search = "0.10.1"
+ fff-query-parser = "0.10.1"
- fff-search = { version = "0.10.1", features = ["zlob"] }
- fff-query-parser = { version = "0.10.1", features = ["zlob"] }
```

`fff-grep` (`version = "0.10.1"`) has no zlob feature, so leave it unchanged.

GPUI is consumed as a git dependency on the [zed](https://github.com/zed-industries/zed) repository (the exact revision is pinned in `Cargo.lock`), so the first build fetches the full Zed repo — expect a large download and a longer initial compile. Subsequent builds reuse the cached checkout.

```sh
git clone https://github.com/th0jensen/fff-gpui
cd fff-gpui
cargo build --release
```

The binary will be at `target/release/fff-gpui`. You can move it anywhere on your `$PATH` or reference it directly in your config.

Having trouble building? Check Zed's [macOS troubleshooting guide](https://zed.dev/docs/development/macos#troubleshooting) — the build requirements are the same.
</details>

<details>
<summary>
<h2>Configuration</h2>
</summary>

Set options in `~/.config/fff-gpui/config.toml`:

```toml
editor = "/usr/local/bin/zed" # run `which $EDITOR` in your shell to find this path
sync_zed_settings = true
global_keybind = "hyper+f"
base_path = "~/Developer" # default directory when launching from the global keybind (legacy alias: base_dir)
exclude_dirs = [
    "Library",
    "node_modules"
]
follow_symlinks = false # set true to index through symlinked directories
# window sizing is optional — omit these for display-relative sizing (see below)
# window_width = 960.0
# window_height = 520.0
# picker_pane_width = 430.0

[font]
ui_family = ".SystemUIFont"
buffer_family = "UbuntuMono Nerd Font"
ui_size = 16.0
buffer_size = 15.0

[theme]
name = "One Dark"
```

`editor` is a fallback for the resident Homebrew service, which does not inherit your shell environment. If `EDITOR` or `VISUAL` is present in the current process, those still win, so custom tasks and other integrations can keep overriding it naturally.

When `sync_zed_settings` is enabled, fff-gpui reads Zed's `settings.json` and mirrors the UI font, buffer font, font sizes, light/dark theme selection, theme colors, and any `theme_overrides` you've set for the theme fff-gpui actually applies — Zed's selected theme, or the one you pin via `[theme].name` — from the bundled Zed themes plus any installed or local Zed theme.

Beyond `name`, the `[theme]` table accepts per-key color overrides. Alongside the core palette keys (`bg`, `border`, `selected_row`, `text_primary`, and so on), you can override `active_line_bg` (the highlighted preview line); the Zed-parity tokens `border_variant` (subtle chrome borders), `text_accent` (fuzzy-match tint), `cursor`, `editor_line_number`, `editor_active_line_number`, and `editor_gutter_bg`; and the git-status colors `git_created`, `git_modified`, `git_deleted`, `git_conflict`, `git_renamed`, `git_untracked`, and `git_ignored`. Each takes a hex color string, e.g. `git_modified = "#F5A524"` — both `#rrggbb` and `#rrggbbaa` are accepted (plus the `#rgb`/`#rgba` shorthands), and any authored alpha is blended live at paint time. The former keys `status_bar_bg`, `input_bg`, `cursor_selection`, and `icon_accent` are no longer accepted and are ignored if present. Match highlighting follows Zed: matched text keeps its syntax color and turns bold over a translucent accent-tinted background.

Explicit config values still win, so you can keep Zed sync enabled and override just the theme, fonts, sizes, or specific colors when needed. In practice, `[theme].name` overrides Zed's chosen theme, and `[font]` overrides the synced font families and sizes. If your Zed `settings.json` defines `theme_overrides` for the theme fff-gpui ends up applying (Zed's selected theme, or your `[theme].name`), those tweaks are deep-merged on top of it — any style key the override sets (palette colors, syntax styles, and any other Zed theme key) is honored. A `null` value for an entry leaves the base theme's value unchanged rather than resetting it. Any colors you set explicitly under `[theme]` in fff-gpui's config still take precedence.

For `global_keybind`, `hyper` is accepted as a shorthand for `shift+control+alt+super`.

`follow_symlinks` (default `false`) controls whether the indexer descends into symlinked directories. Leave it off unless you keep sources behind symlinks (e.g. a stowed dotfiles layout) that you want to search.

Window sizing is optional. When `window_width` and `window_height` are absent, the window sizes to 60% × 60% of the active display (clamped between a 409×320 minimum and 95% of the display); when `picker_pane_width` is absent, the results and preview panes split 50/50. Any of these px values, when set, override the corresponding default — but they are clamped to the same bounds as the defaults, not applied verbatim: `window_width`/`window_height` to the 409×320 floor and 95%-of-display cap, and `picker_pane_width` to the pane minimums (results ≥ 280px, preview ≥ 128px). You can also resize the panes by dragging the divider between results and preview (the results pane stays at least 280px wide, the preview at least 128px); double-click the divider to reset to the 50/50 split. A dragged size lasts for the current session only and resets on restart.

In grep mode, patterns can match across lines: include a literal `\n` in your query (for example `fn foo\n`) and matches spanning consecutive lines are returned.

`exclude_dirs` takes an array of directory paths. Relative entries are resolved from the current base path, so `["Library"]` excludes that directory anywhere under the opened scope. We do not currently support wildcard globbing here. In practice, plain directory names are enough for the system-wide picker use case this option targets, and they keep the config predictable.

Zed themes are discovered from the bundled theme set, your local Zed installation, and extension themes under `~/Library/Application Support/Zed/extensions/installed/`.

### Themes

fff-gpui comes bundled with the same themes as Zed. These are the valid options:

|       | Ayu        | Gruvbox            | One      |
|-------|------------|--------------------|----------|
| Dark  | Ayu Dark, Ayu Mirage | Gruvbox Dark, Gruvbox Dark Hard, Gruvbox Dark Soft | One Dark |
| Light | Ayu Light  | Gruvbox Light, Gruvbox Light Hard, Gruvbox Light Soft | One Light |

</details>

<details>
<summary>
<h2>Running</h2>
</summary>

Launch fff-gpui once to start it as a background service with your global keybind:

```sh
fff-gpui
```

If installed via Homebrew, `brew services start fff-gpui` handles this and re-launches it at login automatically.

The resident daemon keeps its file index live: a background watcher applies create, modify, and delete events in real time, so files added after the daemon started are searchable without a restart. The watcher installs after the initial scan and fails soft (it logs and falls back to the static index) for very broad base paths like your home or root directory.


Launch fff-gpui with the `--open <path>` flag if you don't want the daemon running:

```sh
fff-gpui --open <path>
```

### Keys

- `↑`/`↓` move the cursor, `enter` opens the current row, `esc` quits
- `cmd-f` / `cmd-g` switch between file search and grep; in grep, `shift-tab` cycles the match mode (plain text → regex → fuzzy)
- `ctrl-u` / `ctrl-d` scroll the preview
- `shift-up` / `shift-down` walk the query history for the current view; stepping
  past the newest entry restores what you were typing. `ctrl-up` is an alias for
  `shift-up`. History is per project and per view (file search and grep keep
  separate stacks), holds the last 128 queries, and records a query when it
  leads to an open — not on every keystroke

Grep results are grouped per match under collapsible file headers:

- `alt-z` collapses/expands the file group under the cursor, `alt-shift-z` toggles all groups
- Clicking a header's chevron folds that group; `alt`-clicking it toggles all groups

Multi-select:

- `cmd-shift-s` toggles multi-select mode — checkboxes appear on the rows, and switching the mode off clears the selection
- Click the multi-select icon in the search row (same as `cmd-shift-s`) to toggle the mode; once active, click a row's checkbox directly (no modifier needed) to mark it, or `cmd`-click anywhere on the row
- `tab` marks the current row and advances to the next (entering the mode automatically), `cmd`-click toggles a row, `ctrl-a` toggles everything
- In grep, marks are per match; with marks set, `enter` opens each marked file once, jumping to its first marked match
- In Files mode, marks are per file; `enter` opens each marked file once

The daemon has a memory footprint of ~150mb idle and ~400mb when actively searching. Most of the reported usage in tools like btop is macOS page reservation and will be reclaimed under memory pressure. Report issues if you experience sustained growth beyond these baselines.
</details>

<details>
<summary>
<h2>Zed integration</h2>
</summary>

This is the recommended way to use fff-gpui within a project. Add the following to your Zed config files and replace `/path/to/fff-gpui` with the actual path to your binary.

**`~/.config/zed/tasks.json`**
```json
[
  {
    "label": "fff-gpui: Files",
    "command": "/path/to/fff-gpui --open .",
    "env": { "EDITOR": "zed" },
    "use_new_terminal": false,
    "allow_concurrent_runs": false,
    "reveal": "never",
    "reveal_target": "dock",
    "hide": "always",
    "shell": "system",
    "show_summary": false,
    "show_command": false,
    "save": "none"
  },
  {
    "label": "fff-gpui: Grep",
    "command": "/path/to/fff-gpui --open . --grep",
    "env": { "EDITOR": "zed" },
    "use_new_terminal": false,
    "allow_concurrent_runs": false,
    "reveal": "never",
    "reveal_target": "dock",
    "hide": "always",
    "shell": "system",
    "show_summary": false,
    "show_command": false,
    "save": "none"
  }
]
```

**`~/.config/zed/keymap.json`**
```json
{
  "context": "Workspace",
  "bindings": {
    "cmd-k cmd-p": ["task::Spawn", { "task_name": "fff-gpui: Files" }],
    "cmd-k cmd-f": ["task::Spawn", { "task_name": "fff-gpui: Grep" }]
  }
}
```


This opens fff-gpui scoped to the current project root. `cmd-k cmd-p` launches in file-search mode, `cmd-k cmd-f` launches directly in grep mode. Selected files open in Zed; with grep, the editor jumps to the matched line.
</details>

## License

MIT
