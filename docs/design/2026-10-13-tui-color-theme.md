# TUI Color Theme (0.7.0): Design

- **Status:** approved by Jon on 2026-10-13, from a rendered mockup.
- **Scope:** the TUI chrome. The command output already has color from `scope::display`. The monitor keeps its own color schemes.

## Rules

1. One module holds every TUI color: `crates/scope-cli/src/cli/theme.rs`. No other TUI file names a color. The contrast guard (`crates/scope-cli/tests/tui_contrast.rs`) enforces this.
2. Only the 16 named ANSI colors are used, so the terminal's own palette decides the exact shade. This works on dark and light themes.
3. `NO_COLOR`, `TERM=dumb` and `--no-color` strip all color (`plain::finish_frame`). Each state also shows a word or glyph, so no meaning depends on color.

## Named styles

| Name | Style | Used for |
|---|---|---|
| `ACCENT` | cyan | title, idle pane rule, prompt `scope`, status values, palette names |
| `KEY` | bold magenta | key names in hints and help, prompt `:<chain>` |
| `OK` | green | `✓ Done in N.Ns.` |
| `WARN` | yellow | running pane rule and title, `running Ns`, spinner, `Cancelled after` |
| `ERR` | red | failed pane rule, `✗ Failed in N.Ns.` (bold) |
| `INFO` | blue | the `ℹ` glyph on notes |
| `SECONDARY` | default fg + dim | descriptions, hint text, note text |
| `SELECTED` | reverse cyan | selected palette or menu row |
| `MATCH` | reverse yellow | search matches |

## Chrome

- **Pane rule and title** (`scope v0.7.0`, bold): `ACCENT` when idle, `WARN` while a command runs, `ERR` after a failed command until the next command starts. The state word (`idle`, `running`, `failed`) is in the rule.
- **Prompt:** `scope` in `ACCENT`, `:<chain>` in `KEY`, `>` and the input in the default color.
- **Status bar:** labels `SECONDARY`, values `ACCENT`, `running Ns` in `WARN`, `Esc cancels` bold.
- **Hint line and help:** key names `KEY`, descriptions `SECONDARY`.
- **Footers:** `✓ Done in` (`OK`; the `✓` is new), `✗ Failed in` (`ERR`, bold), `Cancelled after` (`WARN`).
- **Command output:** table header rows bold `ACCENT` (the only change to command output).

## Amendment (2026-10-13): theme is a system, not a TUI feature

Jon: theme colors are controlled and housed centrally for every surface. Clean Code and Clean Architecture are the standard.

### Layers (dependency rule: inward only)

| Layer | Module | Knows | Must not know |
|---|---|---|---|
| Domain (market, chains, compliance, health) | `scope::{market, chains, …}` | business data | any color, style or theme |
| Presentation policy | `scope::display::theme` | semantic roles, their tones, the monitor schemes, the color-off decision | ratatui, `colored`, axum, CSS |
| Adapters (outer) | `scope::display` text painter; `scope_cli::cli::theme` (ratatui); `scope-web` theme CSS | how to render a role on their medium | role choices (they only translate) |

Today the domain already holds no color (checked: no `colored` use outside `scope::display`).

### `scope::display::theme`
- `enum Role { Accent, Key, Ok, Warn, Err, Info, Secondary, Neutral, Selected, Match }`.
- `struct Tone { color: Option<AnsiColor>, bold: bool, dim: bool, reverse: bool, web: &'static str }`, where `AnsiColor` is the 16 named colors, as a plain enum with no framework type.
- `fn tone(role) -> Tone` is the single mapping. The table in this document is its specification.
- Monitor schemes: `enum Scheme { GreenRed, BlueOrange, Monochrome }`, with `fn scheme_tone(scheme, Direction) -> Tone`. Schemes are data in the same module.
- `fn color_enabled()` lives here or is re-exported here. No adapter decides color on its own.

### Adapters (translate only)
- **Text (CLI display):** `theme::paint(role, &str) -> String`. It replaces every `.cyan()` / `.green()` / `.dimmed()` call in `display/terminal.rs` and `cli/errors.rs`. Plain output stays byte-identical. Colored output keeps the same color per role, and a golden test pins it.
- **ratatui (TUI and monitor):** `scope_cli::cli::theme::style(Role) -> Style` and `scheme_style(Scheme, Direction)`. They hold no color literals. Help text uses `paint`, so the hand-written ANSI constants go away.
- **Web:** scope-web serves `/theme.css` built from `tone(role).web` as CSS variables (`--scope-accent`, …). `style.css` uses only `var(--scope-…)`.

### Guard
One test scans every crate (and `scope-web/src/static/*.css`). It allows color literals (`Color::`, `.cyan()`-style calls, hex colors, raw SGR codes) only in `scope::display::theme` and the two adapter files. It reports `file:line`.
