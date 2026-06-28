//! Incremental repo indexing: walk → sha256 → (re)parse changed → persist →
//! rebuild edges from the full persisted graph (so cross-file edges stay correct).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use codegraph_graph::{build, LoadedGraph};
use codegraph_parse::parse_file;
use codegraph_core::{Edge, EdgeRelation, Node};
use codegraph_store::Store;
use sha2::{Digest, Sha256};
use ignore::WalkBuilder;
use rayon::prelude::*;

const EXTS: &[&str] = &[
    "rs", "py", "pyi", "js", "jsx", "mjs", "cjs", "ts", "mts", "cts", "tsx", "go", "swift", "java",
    "c", "h", "cpp", "cc", "cxx", "hpp", "hh", "hxx", "rb", "cs", "sh", "bash", "kt", "kts",
];

/// Directories never indexed (dependencies, build output, caches, VCS).
const EXCLUDE_DIRS: &[&str] = &[
    "target", "node_modules", ".venv", "venv", "env", "Pods", "build", "dist", ".git", ".gradle",
    ".next", ".nuxt", "__pycache__", ".cache", "DerivedData", "vendor", ".idea", ".vscode", "out",
    ".dart_tool", ".mypy_cache", ".pytest_cache", ".tox", "bin", "obj", ".svn", ".hg", ".terraform",
    "coverage", ".codegraph", "Carthage", ".bundle", "bower_components", ".yarn", ".pnp",
];

/// Skip files larger than this (minified bundles, generated blobs) to keep
/// parsing fast and avoid pathological tree-sitter inputs.
const MAX_FILE_BYTES: u64 = 1_500_000;

pub struct IndexStats {
    pub files: usize,
    pub changed: usize,
    pub pruned: usize,
    pub nodes: usize,
    pub edges: usize,
    pub scip_edges: usize,
}

pub fn db_path(root: &Path) -> std::path::PathBuf {
    root.join(".codegraph").join("graph.db")
}

pub fn index_dir(root: &Path, db: &Path, full: bool, scip: Option<&Path>) -> Result<IndexStats> {
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let store = Store::open(db)?;
    store.begin()?;
    let project = project_name(root);
    let mut seen: HashSet<String> = HashSet::new();
    let mut files = 0usize;

    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .add_custom_ignore_filename(".codegraphignore")
        .filter_entry(|e| {
            !e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                || !EXCLUDE_DIRS.contains(&e.file_name().to_str().unwrap_or(""))
        })
        .build();

    // Phase 1: walk + read; decide which files actually need (re)parsing.
    let mut to_parse: Vec<(String, String, String)> = Vec::new();
    for entry in walker.filter_map(|e| e.ok()) {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(true) {
            continue;
        }
        let path = entry.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !EXTS.contains(&ext) {
            continue;
        }
        if entry.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(false) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(path) else { continue };
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        files += 1;
        seen.insert(rel.clone());
        let sha = sha256(&source);
        if !full {
            if let Some(m) = store.manifest_for(&rel)? {
                if m.sha256 == sha {
                    continue; // unchanged
                }
            }
        }
        to_parse.push((rel, source, sha));
    }

    // Phase 2: parse changed files in parallel (CPU-bound, no shared state).
    let parsed: Vec<(String, String, codegraph_parse::ParsedFile)> = to_parse
        .par_iter()
        .map(|(rel, source, sha)| (rel.clone(), sha.clone(), parse_file(&project, rel, source)))
        .collect();
    let changed = parsed.len();

    // Phase 3: persist sequentially (SQLite writes are serial).
    let mut changed_nodes: Vec<Node> = Vec::new();
    for (rel, sha, pf) in parsed {
        store.delete_file_data(&rel)?;
        store.save_calls(&rel, &pf.calls)?;
        store.save_inherits(&rel, &pf.inherits)?;
        store.save_manifest(&rel, &sha, 0)?;
        changed_nodes.extend(pf.nodes);
    }
    store.bulk_upsert_nodes(&changed_nodes)?;

    // Prune files that vanished since last index.
    let mut pruned = 0usize;
    for mf in store.manifest_files()? {
        if !seen.contains(&mf) {
            store.delete_file_data(&mf)?;
            store.delete_manifest(&mf)?;
            pruned += 1;
        }
    }

    // Rebuild ALL edges from the full persisted node + call set (keeps
    // cross-file CALLS correct after a partial update).
    // Nothing changed and not a forced full rebuild: the graph is already current.
    if changed == 0 && pruned == 0 && !full && scip_path(root, scip).is_none() {
        store.commit()?;
        return Ok(IndexStats {
            files,
            changed,
            pruned,
            nodes: store.node_count()? as usize,
            edges: store.edge_count()? as usize,
            scip_edges: 0,
        });
    }

    let nodes = store.all_nodes()?;
    let calls = store.all_calls()?;
    let inherits = store.all_inherits()?;
    let built = build(&nodes, &calls, &inherits);
    let mut edges = built.edges;
    let scip_edges = merge_scip_edges(root, scip, &nodes, &mut edges);
    store.clear_edges()?;
    store.bulk_upsert_edges(&edges)?;
    store.clear_hyperedges()?;
    for (h, members) in &built.hyperedges {
        store.upsert_hyperedge(h, members)?;
    }

    // Community + centrality over the full graph, persisted onto each node.
    let lg = LoadedGraph::load(&nodes, &edges);
    let analytics = lg.analyze();
    let mut nodes = nodes;
    for nd in nodes.iter_mut() {
        if let Some(&(c, pr, bw)) = analytics.get(&nd.id) {
            nd.community = Some(c);
            nd.pagerank = pr;
            nd.betweenness = bw;
        }
    }
    store.bulk_upsert_nodes(&nodes)?;
    store.rebuild_fts()?;
    store.commit()?;

    Ok(IndexStats { files, changed, pruned, nodes: nodes.len(), edges: edges.len(), scip_edges })
}

/// Locate a `.scip` index: explicit path, else `index.scip`, else any `*.scip` at root.
fn scip_path(root: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return p.exists().then(|| p.to_path_buf());
    }
    let cand = root.join("index.scip");
    if cand.exists() {
        return Some(cand);
    }
    std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "scip"))
}

/// Merge compiler-grade SCIP edges in: they supersede the tree-sitter edge for
/// the same (src, dst, relation) and add precise edges tree-sitter missed.
fn merge_scip_edges(root: &Path, explicit: Option<&Path>, nodes: &[Node], edges: &mut Vec<Edge>) -> usize {
    let Some(path) = scip_path(root, explicit) else { return 0 };
    let Ok(bytes) = std::fs::read(&path) else { return 0 };
    let Ok(scip) = codegraph_resolve::import_scip(&bytes, nodes) else { return 0 };
    if scip.is_empty() {
        return 0;
    }
    let superseded: HashSet<(String, String, EdgeRelation)> =
        scip.iter().map(|e| (e.src.clone(), e.dst.clone(), e.relation)).collect();
    edges.retain(|e| !superseded.contains(&(e.src.clone(), e.dst.clone(), e.relation)));
    let n = scip.len();
    edges.extend(scip);
    n
}

fn sha256(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

fn project_name(root: &Path) -> String {
    root.canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| "project".to_string())
}
