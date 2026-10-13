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
