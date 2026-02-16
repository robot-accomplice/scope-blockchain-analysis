# Scope Architecture Documentation

Architecture and dataflow diagrams for the Scope blockchain analysis tool.

## Diagram Index

### C4 Model

| Document | Description |
|----------|--------------|
| [c4-context.md](c4-context.md) | **Level 1** — System context: Scope and external systems (Etherscan, DexScreener, RPCs, Biconomy) |
| [c4-containers.md](c4-containers.md) | **Level 2** — Containers: CLI, Chains, Compliance, Market, Display, Config, Tokens |

### Dataflow Diagrams

| Document | Command / Area | Description |
|----------|----------------|--------------|
| [dataflow-main.md](dataflow-main.md) | Entry point | CLI parse, config load, command dispatch |
| [dataflow-address.md](dataflow-address.md) | `scope address` | Balance, transactions, tokens, report |
| [dataflow-tx.md](dataflow-tx.md) | `scope tx` | Transaction lookup by hash |
| [dataflow-crawl.md](dataflow-crawl.md) | `scope crawl` | Token analytics from DEX + explorer |
| [dataflow-discover.md](dataflow-discover.md) | `scope discover` | Trending/boosted tokens from DexScreener |
| [dataflow-compliance.md](dataflow-compliance.md) | `scope compliance` | Risk, analyze, trace, compliance-report |
| [dataflow-market.md](dataflow-market.md) | `scope market summary` | Peg and order book health |
| [dataflow-token-health.md](dataflow-token-health.md) | `scope token-health` / `scope health` | DEX + optional market composite |
| [dataflow-portfolio.md](dataflow-portfolio.md) | `scope portfolio` | Add, remove, list, summary |
| [dataflow-export.md](dataflow-export.md) | `scope export` | Address history, portfolio data |
| [dataflow-report.md](dataflow-report.md) | Report generation | All `--report` / `--output` flows |
| [dataflow-monitor.md](dataflow-monitor.md) | `scope monitor` | Live TUI dashboard |
| [dataflow-interactive.md](dataflow-interactive.md) | `scope interactive` | REPL with context |
| [dataflow-data-sources.md](dataflow-data-sources.md) | Data sources | ChainClientFactory, DexScreener, Etherscan, Biconomy |
| [dataflow-web.md](dataflow-web.md) | `scope web` / `scope serve` | Browser mode: REST API, WebSocket monitor, daemon (feature-gated) |

| [contract-analysis.md](contract-analysis.md) | `scope contract` | Contract analysis pipeline: source, proxy, access, vulnerability, DeFi, external intel |

**Note:** `scope insights` dispatches to address, tx, and crawl handlers (see dataflow-main). `scope contract` has its own dataflow in [contract-analysis.md](contract-analysis.md). `scope completions` generates shell scripts to stdout (no dataflow diagram needed).

### Guidelines

| Document | Description |
|----------|-------------|
| [cli-usability-guidelines.md](cli-usability-guidelines.md) | **CLI UX** — Usability philosophy and framework principles for feature development |

### Other Architecture Docs

| Document | Description |
|----------|-------------|
| [monitor-layout-architecture.md](../monitor-layout-architecture.md) | Monitor TUI layout presets and widgets |
| [monitor-config-schema.yaml](../monitor-config-schema.yaml) | Monitor config schema |
| [test-coverage-improvement-plan.md](test-coverage-improvement-plan.md) | Testing roadmap |

## Rendering Diagrams

Diagrams use [Mermaid](https://mermaid.js.org/) syntax. They render in:

- GitHub (native Mermaid support in markdown)
- VS Code (Mermaid extension)
- Mermaid Live Editor: https://mermaid.live/

## C4 Diagram Note

C4 diagrams (`C4Context`, `C4Container`) are experimental in Mermaid. If they do not render, use the [C4-PlantUML](https://github.com/plantuml-stdlib/C4-PlantUML) syntax reference to convert to PlantUML.
