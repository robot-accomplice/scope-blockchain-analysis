# CLI Usability Guidelines

**Purpose:** These guidelines capture the usability philosophy and framework principles that Scope should follow for all CLI feature development. They are derived from [clig.dev](https://clig.dev), [Atlassian's 10 Design Principles for Delightful CLIs](https://www.atlassian.com/blog/it-teams/10-design-principles-for-delightful-clis), and [Evil Martians' Progress Display Patterns](https://evilmartians.com/chronicles/cli-ux-best-practices-3-patterns-for-improving-progress-displays).

**Scope:** Apply these guidelines when designing new commands, modifying existing commands, or reviewing CLI UX.

---

## Implementation Status

The following table tracks which guidelines have been implemented as of the current release:

| Guideline | Status | Details |
|-----------|--------|---------|
| Exit codes | ✅ Implemented | Non-zero on failure (`process::exit(1)`) |
| Output streams (stdout/stderr) | ✅ Implemented | Primary output to stdout, progress/errors to stderr |
| Help with examples | ✅ Implemented | `after_help` blocks on top-level and subcommands (address, tx, crawl) |
| Documentation link in help | ✅ Implemented | GitHub URL + quickstart path in `scope --help` |
| Typo suggestions | ✅ Implemented | Built-in clap fuzzy matching |
| Progress indicators | ✅ Implemented | `Spinner` and `StepProgress` in `cli::progress` module; 9 commands instrumented |
| Error remediation hints | ✅ Implemented | `display_error()` + `error_suggestion()` for 6 error types |
| Shell completion | ✅ Implemented | `scope completions bash\|zsh\|fish` via `clap_complete` |
| Command grouping by task | ✅ Implemented | Commands ordered: entity lookup → token → compliance → data → config |
| Command map / decision tree | ✅ Implemented | In README.md and QUICKSTART.md |
| Aliases for common commands | ✅ Implemented | addr, tx, token, port, mon, health, disc, shell, config, insight |
| Docstring consistency (bca→scope) | ✅ Fixed | All module docs and runtime strings updated |
| Next-step hints after setup | ✅ Implemented | Setup wizard shows insights, monitor, completions suggestions |
| Global options in subcommand help | ⚠️ Partial | Global flags still repeat in subcommand help (clap limitation) |
| Option grouping (common vs advanced) | 🔲 Not yet | Planned for flag-heavy commands (crawl, market summary) |
| Flatten single-subcommand nesting | 🔲 Not yet | `market summary` and `report batch` still require subcommand |
| `NO_COLOR` env var support | ⚠️ Partial | `--no-color` flag works; `NO_COLOR` env var not yet checked |

---

## Philosophy

### Human-first design

CLIs are used primarily by humans. Design for humans first, machines second. When convention would compromise usability, consider breaking it—with intention and clarity.

### Composability

Scope will be used in pipelines and scripts. Design for composability: stdout for primary output, stderr for logs and errors, predictable exit codes. Plain text and structured formats (JSON) should pipe cleanly.

### Conversation as the norm

Users run commands, get errors, fix, and retry. This is a conversation. Make it a helpful one: suggest corrections, clarify state, confirm before destructive actions.

---

## Core Rules (Always Follow)

### Exit codes

- Return 0 on success, non-zero on failure.
- Map non-zero codes to the most important failure modes when meaningful.

### Output streams

- **stdout:** Primary output only. Machine-readable output (JSON, CSV) goes here.
- **stderr:** Logs, progress messages, errors, prompts. Never pollute stdout with diagnostic output when piped.

### Help

- Display full help for `-h`, `--help`, and `help` subcommand.
- Subcommands must support `scope <subcommand> --help`.
- When a command requires arguments and is run with none, display concise help (what it does, 1–2 examples, instruction to use `--help` for more).

### Progress

- Never leave users staring at a blank cursor during operations lasting more than ~1 second.
- Use spinners for short sequential tasks (a few seconds).
- Use "X of Y" or progress bars when progress is measurable.
- When done, clear progress UI and show a clear completion state (e.g. checkmarks).

### Errors

- Every error must be human-readable.
- Include a suggested fix or next step when possible (e.g. "Run `scope setup` to configure.").
- Avoid raw stack traces unless `--verbose` or debug mode.
- Never assume invalid input is always a typo—suggestions are helpful; auto-correcting can be dangerous.

---

## Discovery and Help

### Usage display structure

Follow a clear order (Better CLI, man pages):

1. **Name, description, version** — What the program does in one line
2. **Usage and examples** — Synopsis plus 1–3 example invocations
3. **Commands and options** — Reference listing
4. **Configuration** — Env vars, config files when relevant

### Lead with examples

Users learn from examples first. Include 2–3 example invocations in help text for main commands and complex subcommands. Place them in an **Examples** section (before or after options).

### Error-case usage

When a required argument is missing, show the usage line plus a concrete example (e.g. `Example: scope address 0x742d35Cc...`). Avoid leaving users with only `Usage: scope address <ADDRESS>` and no hint of valid input format.

### Global options in subcommand help

When global options (e.g. `--config`, `-v`, `--no-color`) are repeated in every subcommand's help, output becomes long and repetitive. Prefer hiding them in subcommand help or moving them to a collapsible "Global options" section, while keeping them in top-level help.

### Option grouping in help

For commands with 5+ flags, group options (e.g. "Common options" vs "Advanced") so users can scan the most relevant ones first. Use your framework's argument groups or help headings.

### Link to documentation

When subcommands have deeper documentation (e.g. QUICKSTART, architecture docs), link to them in help text.

### Typo suggestions

When the user enters an invalid subcommand or flag, suggest the closest match when confident ("Did you mean: monitor?"). Do not auto-run the suggestion without explicit user confirmation.

---

## Flags and Arguments

### Prefer flags over positional args

Labels give context and avoid memorizing order. Provide short names for common flags (e.g. `-c` for `--chain`).

### Sensible defaults

Provide defaults for options so users can run common workflows with minimal typing. Document defaults in help.

### Prompt for missing required info

When interactive, prompt for required options instead of failing immediately—when it makes sense and is not scripted.

---

## Output and Formatting

### Human-readable by default

Assume output is read by a human unless `--format json`, `--ai`, or piping suggests otherwise. Use formatting (tables, colors) to improve scannability.

### Machine-readable option

Provide `--format json` (or equivalent) for scripting. When human formatting breaks line-based tools (grep, awk), provide `--plain` or document the appropriate format flag.

### Respect NO_COLOR and --no-color

Honor `NO_COLOR` and `--no-color` to disable ANSI colors. Essential for accessibility and scripted use.

### Tense consistency

During an operation: "Downloading...", "Fetching...". After completion: "Downloaded.", "Fetched." Do not leave "ing" verbs in the log after the action finishes.

---

## Progress Patterns

| Pattern      | When to use                                                |
|-------------|-------------------------------------------------------------|
| **Spinner** | Single or few sequential steps, completes in a few seconds |
| **X of Y**  | Multi-step process with known total (e.g. batch of N items) |
| **Progress bar** | Multiple parallel or lengthy processes; avoid when a single bar would suffice |

### Avoid the silent treatment

Do not run long operations with no feedback. Even a simple "Fetching..." line is better than a blank screen.

### Clean up when done

Clear spinners and progress bars when the action completes. Leave a readable log that tells the story of what ran.

---

## Interactive Flows

### Easy way out

For interactive commands, remind users they can exit (e.g. Ctrl+C, Quit). Make exit pathways obvious in the footer or help.

### Reaction for every action

After each user action, provide clear feedback (e.g. "Logged out.", "Report saved to report.md.").

---

## Command Hierarchy and Complexity

### Depth and structure

- Prefer flat commands when one level suffices. Avoid `scope X Y` when `scope X` could do the job (e.g. a single logical action).
- When using subcommands, ensure each level adds real value: either multiple distinct actions (e.g. portfolio add/remove/list/summary) or clear conceptual grouping.
- Avoid single-subcommand nesting: if `scope market` has only `summary`, consider promoting `scope market` to do that by default.

### Consistent grammar

- Use a consistent pattern for subcommands within a command. Either all verbs (add, remove, list) or all noun-phrases (risk, trace, analyze).
- Avoid mixing styles. Document the chosen pattern so future additions follow it.
- Prefer `[noun] [verb]` or `[verb] [object]` patterns users can predict.

### Grouping by task

- In help output, group related commands by user task (e.g. "Entity lookup", "Token analysis", "Compliance", "Data export").
- Order commands by frequency of use or logical flow, not purely alphabetically, when it improves discoverability.
- When commands overlap (e.g. crawl vs token-health vs insights), document clearly when to use each. Add a "Command map" or decision tree if helpful.

### Flag complexity

- For commands with 5+ flags, group "Common" vs "Advanced" in help. Use clap's argument groups or headings.
- Provide sensible defaults so the most common workflow requires few flags.
- Consider `--help` for common options and `--help-all` or collapsible sections for power users.

### Overlap and discoverability

- When adding a new command, check for overlap with existing ones. Prefer extending an existing command over adding a sibling that does something similar.
- If overlap is intentional (e.g. insights as a catch-all vs specific commands), document the relationship and when to use each.

---

## Consistency

### Follow established conventions

Use `-h`/`--help`, `-v`/`--verbose`, `--version` as users expect. Align flag naming with common tools (e.g. `--output`/`-o` for output path).

### Aliases for common commands

Provide short aliases for frequently used subcommands (e.g. `addr`, `mon`, `health`).

---

## Checklist for New Commands

Before shipping a new command or major CLI change:

- [ ] Help text includes description and examples
- [ ] Long-running steps show progress (spinner, X-of-Y, or equivalent)
- [ ] Errors include a suggested fix when possible
- [ ] stdout vs stderr used correctly
- [ ] Exit codes are correct
- [ ] `--no-color` / NO_COLOR respected
- [ ] Sensible defaults for optional args
- [ ] Short alias considered for the subcommand
- [ ] Machine-readable output (`--format json`) if output is structured
- [ ] **Command hierarchy:** Depth justified; grammar consistent with sibling commands; no unnecessary nesting
- [ ] **Overlap:** If similar to existing command, documented when to use each; consider extending existing rather than adding new
- [ ] **Usage display:** Help includes 1–2 examples; error case (missing required arg) shows example; options grouped if 5+

---

## References

- [clig.dev — Command Line Interface Guidelines](https://clig.dev)
- [Atlassian: 10 design principles for delightful CLIs](https://www.atlassian.com/blog/it-teams/10-design-principles-for-delightful-clis)
- [Evil Martians: CLI UX best practices — progress displays](https://evilmartians.com/chronicles/cli-ux-best-practices-3-patterns-for-improving-progress-displays)
- [Better CLI — CLI Design Guide](https://bettercli.org/)
- [Julian Dunn: Designing Great Command-Line User Experiences](https://www.juliandunn.net/2016/08/09/designing-great-command-line-user-experiences/) — Command grammar, noun-verb patterns, cognitive load
