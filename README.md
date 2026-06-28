# CodeGraph

A single static binary that turns **any codebase** into a queryable code-knowledge graph for AI agents — over **MCP** (Claude Code, Cursor, …) and from a **standalone CLI**. Project-agnostic, local-only, no API key required. An optional local LLM (LM Studio / MLX / Ollama, or a cloud key) adds natural-language Q&A and semantic search — everything degrades gracefully with no model running.

> **v1.1** — 12 languages, cross-file call resolution, incremental indexing, Louvain communities + betweenness, inheritance/hyperedges, full-text + semantic search, doc ingestion, an MCP server with 9 tools, and an optional LLM layer. 34 tests, zero clippy warnings.

## Features

- **12 languages** — Rust, Python, JavaScript, TypeScript, Go, **Swift**, Java, C, C++, Ruby, C#, Bash. One grammar-driven parser (a grammar + a label map per language).
- **The graph** — `File / Function / Method / Class / Enum / Interface / Type / Module / Document` nodes joined by `DEFINES`, `CALLS`, `INHERITS`, `IMPLEMENTS` edges, plus **IMPLEMENTS hyperedges** (an interface + all its implementers).
- **Honest, cross-file resolution** — calls resolve within a file first, then to a project-wide **unique** name. Ambiguous names stay unlinked — no phantom edges; no phantom cross-language calls.
- **Incremental** — sha-256 manifest skips unchanged files, re-parses edits, prunes deletions, and rebuilds edges from the full graph so cross-file links stay correct.
- **Graph intelligence** — `trace` (shortest dependency path), `impact` (blast-radius), `callers` / `callees`, `implementers`, `important` (PageRank), `communities` (Louvain). Betweenness + PageRank + community are computed at index time and stored on every node.
- **Search** — full-text (`search`) and **semantic** vector search (`semantic`, with optional `--hyde`), plus `ask` (NL answer over real source snippets).
- **Ingest** — `ingest` pulls PDFs, text/markdown, and web pages in as searchable `Document` nodes.
- **MCP server** — `search, semantic_search, get_node, callers, callees, trace_path, blast_radius, important, stats` over stdio.
- **Local-first** — works fully offline with zero deps; LLM/embedding layers are optional enrichment.

## Install

Rust 1.89+:

```bash
git clone git@github.com:sina-parsania/FlowCrafter.git codegraph && cd codegraph
./install.sh                         # release build → ~/.local/bin + Claude Code MCP hint
# or
cargo install --path crates/codegraph-cli
```

Prebuilt binaries (macOS arm64/x64, Linux x64/arm64, Windows x64) are built by CI on every `v*` tag and attached to the GitHub release.

## Usage

```bash
codegraph index .                    # incremental index → .codegraph/graph.db  (--full to force)
codegraph search UserService         # full-text symbol search
codegraph semantic "retry with backoff" --hyde   # vector search by meaning
codegraph ask "how does auth work?"  # NL answer via a local LLM over source snippets
codegraph important --limit 15       # most central symbols (PageRank)
codegraph communities                # detected code clusters (Louvain)
codegraph impact processPayment      # what breaks if this changes (blast-radius)
codegraph callers handleLogin        # who calls it
codegraph callees parseFile          # what it calls
codegraph implementers Repository    # who implements/extends it
codegraph trace router handler       # shortest dependency path
codegraph ingest ./docs/guide.pdf    # ingest a PDF/markdown/URL as Document nodes
codegraph doctor                     # languages, schema, local-LLM availability
codegraph install                    # wire into Claude Code (~/.claude.json)
codegraph mcp                        # run the MCP server over stdio
```

### Use from Claude Code

`codegraph install` writes the MCP server into `~/.claude.json` (backed up first), or add it manually:

```json
{
  "mcpServers": {
    "codegraph": {
      "command": "codegraph",
      "args": ["mcp", "--path", "/path/to/repo"]
    }
  }
}
```

Then: _"use codegraph to find what depends on `processPayment`"_ → the agent calls `blast_radius`.

## Optional LLM (local-first, no key)

Auto-detected, first reachable wins: **LM Studio** (`:1234`, MLX on Apple Silicon) → **mlx-lm/mlx-vlm** (`:8080`) → **Ollama** (`:11434`) → **OpenAI/Gemini** (opt-in `OPENAI_API_KEY` / `GEMINI_API_KEY`). One `LlmClient` over a unified OpenAI-compatible backend — adding a provider is one config entry, never a code change. Override with `CODEGRAPH_LLM_PROVIDER` / `_BASE_URL` / `_MODEL` / `CODEGRAPH_EMBED_MODEL`.

> Semantic search uses `/v1/embeddings`; works with any compliant endpoint (e.g. Ollama `nomic-embed-text`). LM Studio: an embedding model must be _loaded_ (not just downloaded) with its embeddings server enabled.

## Architecture

Cargo workspace: `codegraph-core` (types, config, LLM traits, cosine) · `codegraph-parse` (grammar-driven tree-sitter, 12 languages) · `codegraph-graph` (edge build, cross-file resolution, Louvain, betweenness, hyperedges) · `codegraph-store` (SQLite + FTS5 + vectors + zst artifact) · `codegraph-llm` (OpenAI-compatible provider registry) · `codegraph-ingest` (PDF/web/text) · `codegraph-mcp` (rmcp server) · `codegraph-cli`.

Pipeline: **walk → parse → resolve & build graph → analytics (community/PageRank/betweenness) → persist → search / traverse / serve (MCP) → optional LLM enrichment.**

## Roadmap (gated; the core never depends on these)

These remain deliberately out of v1.1 — each needs an external dependency the core shouldn't assume:

- **Media ingestion** (audio/video transcription, image/figure vision) — behind the `media` feature; needs ffmpeg / whisper / tesseract. Text ingest (PDF/web/markdown) ships today.
- **SCIP import** (compiler-grade resolution) — needs a generated `.scip` from an external indexer + symbol mapping. The `ResolutionTier::Scip` seam exists; today's resolver is tree-sitter scoped + unique-name.
- **Cross-service links** (HTTP route ↔ client) — held back to avoid heuristic phantom edges.
- **LLM reranking** of search results — marginal over hybrid + HyDE.

## License

Dual-licensed under MIT or Apache-2.0.
