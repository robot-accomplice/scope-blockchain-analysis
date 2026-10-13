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

Jon: theme colors are controlled and housed centrally. Clean Code and Clean Architecture are the standard.

### Scope

The theme covers the terminal surfaces: CLI output, the TUI and the monitor. The web UI (`scope-web`) is not in scope. It keeps its own CSS.

### Layers (dependency rule: inward only)

| Layer | Module | Knows | Must not know |
|---|---|---|---|
| Domain (market, chains, compliance, health) | `scope::{market, chains, …}` | business data | any color, style or theme |
| Presentation policy | `scope::display::theme` | semantic roles, their tones, the monitor schemes, the color-off decision | ratatui, `colored`/`owo-colors` |
| Adapters (outer) | `scope::display::theme::text` (SGR text painter); `scope_cli::cli::theme` (ratatui) | how to render a tone on their medium | role choices (they only translate) |

Today the domain already holds no color (checked: no color crate use outside `scope::display`).

### Roles

`tone(role)` is the single mapping. This table is its specification; the test `tones_match_the_design_table` pins it.

| Role | ANSI tone | Used for |
|---|---|---|
| `Accent` | cyan | box rules, section and table headers (bold), low severity, TUI title and idle rule |
| `Key` | bold magenta | key names in hints and help, prompt `:<chain>` |
| `Ok` | green | passed checks, gains, `HEALTHY` (bold), `✓ Done` |
| `Warn` | yellow | warnings (glyph bold), medium severity, running state |
| `Err` | red | failed checks, losses, `UNHEALTHY` and critical (bold), high severity |
| `Info` | blue | the `ℹ` glyph on notes |
| `Secondary` | default fg + dim | labels, descriptions, notes, informational severity |
| `Neutral` | default style | plain text |
| `Selected` | reverse cyan | selected palette or menu row |
| `Match` | reverse yellow | search matches |
| `Link` | default fg + underline | URLs in link rows |

The strong (bold) form of a role is `tone(role).strong()`. The text painter offers it as `paint_strong(role, …)`. It is used for verdicts and headings.

### `scope::display::theme`
- `enum Role` as in the table above.
- `struct Tone { color: Option<AnsiColor>, bold, dim, underline, reverse }`, where `AnsiColor` is the 16 named colors, as a plain enum with no framework type.
- Monitor schemes: `Scheme` is the existing `scope::config::ColorScheme` (`GreenRed`, `BlueOrange`, `Monochrome`), so one enum names the schemes. `fn scheme_tone(scheme, Direction) -> Tone` with `Direction { Up, Down, Flat }`:

| Scheme | Up | Down | Flat |
|---|---|---|---|
| `GreenRed` | green | red | default |
| `BlueOrange` | blue | yellow (was RGB orange; not a named color) | default |
| `Monochrome` | default | default | default |

- `color_enabled()`, `color_enabled_stderr()`, `color_enabled_with()` and `set_color_override()` live here. `scope::display::terminal` re-exports them for existing callers. No adapter decides color on its own.

### Adapters (translate only)
- **Text (CLI display):** `theme::paint(role, &str) -> String` paints for stdout. `theme::Painter` paints for a given decision (`Painter::new(enabled)`, `Painter::stdout()`, `Painter::stderr()`), with `paint`, `paint_strong` and `paint_tone`. It replaces every `.cyan()` / `.green()` / `.dimmed()` call in `display/terminal.rs` and `cli/errors.rs`. Plain output stays byte-identical. Colored output keeps the same look per role, and golden tests pin it. One intended change: the section title moves from bold bright white to bold `Accent` (bright white breaks the no-white-text rule).
- **ratatui (TUI and monitor):** `scope_cli::cli::theme::style(Role) -> Style` and `scheme_style(Scheme, Direction)`. They hold no color literals. Help text uses `paint`, so the hand-written ANSI constants go away.

### Guard
One test scans every Rust crate. It allows color literals (`Color::`, `.cyan()`-style calls, raw SGR codes) only in `scope::display::theme` and the ratatui adapter file. It reports `file:line`. It does not scan `scope-web`.
