# Live 3-Way Benchmark — codegraph v1.21.0 vs codebase-memory-mcp (cbm) v0.8.1 vs code-review-graph (crg)

Machine: M-series macOS, 12 workers. All runs live, sequential, 2026-07-02. cbm isolated via fake `$HOME`, codegraph via `CODEGRAPH_CACHE_DIR`, crg via in-repo `.code-review-graph/` (deleted after). All artifacts cleaned up.

| Metric (median unless noted) | codegraph | cbm | crg |
|---|---|---|---|
| **Cold index — knowledge-rag** (~190 files) | **0.24 s** (1,177 n / 1,613 e) | 0.79 s (2,176 n / 7,169 e) | 1.38 s (794 n / 7,107 e) |
| **Cold index — backend-app** (~2,840 files) | **1.50 s** (14,050 n / 22,802 e) | 2.83 s (23,545 n / 76,944 e) | 11.10 s (17,979 n / 152,551 e) |
| **Warm no-change reindex — knowledge-rag** | **0.015 s** | 0.017 s | 0.190 s |
| **Warm no-change reindex — backend-app** | **0.061 s** | 0.062 s | 0.582 s |
| **Symbol lookup `getUserProfile`, backend-app (5×, one-shot proc)** | **11 ms** | 16 ms | ~970 ms one-shot (10 ms warm in-session + 0.5–2.6 s Python/MCP startup) |
| **Callers `getUserProfile` (5×, one-shot proc)** | 14 ms | **13 ms** | ~970 ms one-shot (2 ms warm in-session) |
| **Callers `getUserProfile` — answer** | 13 direct callers + **✓ "all 18 call sites resolved — complete"** | 53 rows mixing hop 1–3 (only 11 direct); no coverage signal | 14 callers incl. `it:`-test nodes; no coverage signal |
| **Callers `create` (ambiguous) — answer** | 155 rows + **⚠ "184/323 call sites resolved, 139 dropped — may be INCOMPLETE, fall back to grep"** | Silently traced ONE definition (`BaseRepository.create`, inferable only from its callee); 19 direct + 50 transitive; no ambiguity/coverage signal | **Refused**: "'create' is a common builtin — skipped to avoid noise" → 0 results, no override |
| **Disk — backend-app index** | 83 MB (`graph.db` 86.9 MB) | **74 MB** | 224 MB |
| **Disk — knowledge-rag index** | **4.6 MB** | 8.9 MB | 9.1 MB |

## Behavioral differences observed

- **Coverage honesty is codegraph-only.** codegraph was the only tool emitting a per-query confidence signal (✓ complete on `getUserProfile`, ⚠ resolved/dropped counts + grep-fallback advice on `create`). cbm and crg present partial answers as if complete.
- **Ambiguity handling, three philosophies:** codegraph = answer per-name across definitions with an explicit incompleteness warning; cbm = silently binds a bare name to one definition (dangerous — a reviewer asking "who calls create" gets one repository's slice with zero indication that dozens of other `create` methods exist); crg = hard blocklist on common names, returns nothing (safe but useless, no `--force`).
- **cbm `trace_path` direction footgun:** `direction:"callers"`, `"callees"`, `"upstream"`, `"incoming"`, even `"bogus"` all return an empty echo `{"function":...,"direction":...}` with rc=0 — only `"both"` produces data. Silent-empty on an invalid enum is easy to misread as "no callers."
- **cbm mixes transitive hops into "callers"** (hop 1–3 in one flat list: 53 rows for a function with 11 direct callers), inflating apparent answer size; codegraph/crg return direct callers.
- **cbm symbol search is noisy on generic terms:** `create` → 709 substring matches, 200 returned, `.cursor/*.md` docs and migration *files* ranked first. On exact names it's rich (per-node complexity/cognitive/fingerprint metadata — the most detailed single-node payload of the three).
- **crg is server-first:** no CLI query commands at all (queries only via MCP tools), so every one-shot query pays ~0.5–2.6 s Python startup; warm in-session calls are the fastest measured (2 ms callers). Its 152k edges include low-confidence edges (schema has explicit edge-confidence columns), and it has the broadest analysis surface (flows, communities, wiki, hub/bridge nodes).
- **crg failed inside the macOS sandbox** (`ProcessPoolExecutor` → `PermissionError: SC_SEM_NSEMS_MAX`); had to run unsandboxed. codegraph and cbm ran fully sandboxed.
- **Index placement:** crg writes 224 MB *inside the repo working tree* (adds its own .gitignore); cbm uses a global `~/.cache/codebase-memory-mcp/` store keyed by absolute path; codegraph uses a central cache relocatable via env var — cleanest for CI/isolation.
- **Warm-path parity:** cbm's no-change reindex is exactly as fast as codegraph's (~60 ms on 2,840 files) — its change detection is excellent. crg's `update` re-parsed "8–44 files" on every no-change run (~0.6 s).
- **Footprint:** cbm ships a 269 MB binary; codegraph is a small single Rust binary; crg is a uv-managed Python tool (needs interpreter + optional igraph, which was missing — it fell back to file-based community detection).

Caveats: node/edge counts aren't comparable 1:1 (different node taxonomies — cbm counts more node kinds, crg keeps unresolved/low-confidence edges, codegraph drops unresolved edges by design). Latency medians include process startup for codegraph/cbm CLI (that is how agents invoke them); crg's warm in-session numbers are given separately since it has no query CLI.