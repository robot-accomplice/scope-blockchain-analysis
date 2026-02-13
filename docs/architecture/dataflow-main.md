# Main Application Dataflow

High-level flow from CLI entry to command execution.

```mermaid
flowchart TB
    subgraph Entry
        A[User: scope &lt;cmd&gt;] --> B[main.rs]
        B --> C[Clap parse]
        C --> D[Config.load]
        D --> E{Config exists?}
        E -->|No| F[Setup wizard]
        F --> D
        E -->|Yes| G[DefaultClientFactory.new]
    end

    subgraph "Dispatch (task-ordered)"
        G --> H[Commands match]

        H --> I1["<b>Entity lookup</b><br/>address::run<br/>tx::run<br/>insights::run"]
        H --> I2["<b>Token analysis</b><br/>crawl::run<br/>token_health::run<br/>discover::run<br/>monitor::run_direct<br/>market::run"]
        H --> I3["<b>Compliance</b><br/>compliance::handle_*"]
        H --> I4["<b>Data &amp; export</b><br/>portfolio::run<br/>export::run<br/>report::run"]
        H --> I5["<b>Config</b><br/>interactive::run<br/>setup::run<br/>completions (stdout)"]
    end

    subgraph "Error Handling"
        I1 & I2 & I3 & I4 --> ERR{Error?}
        ERR -->|Yes| DISP["display_error()"]
        DISP --> HINT["error_suggestion() → hint"]
        HINT --> EXIT["exit(1)"]
        ERR -->|No| OK["exit(0)"]
    end

    subgraph "Progress (indicatif)"
        I1 -.-> SP["Spinner / StepProgress"]
        I2 -.-> SP
        I3 -.-> SP
        I4 -.-> SP
        SP -.-> TTY{"stderr is TTY?"}
        TTY -->|Yes| ANIM[Animated spinner]
        TTY -->|No| HIDDEN["Hidden (clean pipe)"]
    end

    subgraph Factory
        G -.-> J[create_chain_client]
        G -.-> K[create_dex_client]
    end

    style A fill:#e1f5fe
    style H fill:#fff3e0
    style HINT fill:#fff9c4
```

## Flow Summary

1. **Parse** — CLI args → `Commands` enum (with typo suggestions and `after_help` examples)
2. **Config** — Load `~/.config/scope/config.yaml`, merge with env
3. **Factory** — Build `DefaultClientFactory` with chain config
4. **Dispatch** — Match command (ordered by task group), call handler with config + factory
5. **Progress** — Each handler creates `Spinner` or `StepProgress` for long operations; auto-hides in pipes
6. **Handler** — Uses factory for chain/DEX clients, config for output preferences
7. **Error handling** — `display_error()` prints error + `error_suggestion()` hint (invalid address, missing config, network, auth)
8. **Completions** — `scope completions <shell>` generates shell script to stdout (no factory needed)
