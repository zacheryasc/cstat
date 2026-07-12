use colored::Colorize;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{Expr, File as SynFile, ImplItem, Item, ReturnType, Type};

use crate::render;

/// Information about a function sufficient for redundancy comparison.
#[derive(Debug, Clone)]
struct FunctionSig {
    name: String,
    file: String,
    line_start: usize,
    param_types: Vec<String>,
    return_type: String,
}

/// A pair of functions flagged as potentially redundant.
#[derive(Debug, Clone)]
pub struct RedundantPair {
    pub func_a: String,
    pub file_a: String,
    pub line_a: usize,
    pub func_b: String,
    pub file_b: String,
    pub line_b: usize,
    pub similarity: f64,
    pub kind: SimilarityKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SimilarityKind {
    Signature,
    Structural,
    Both,
}

impl std::fmt::Display for SimilarityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimilarityKind::Signature => write!(f, "signature"),
            SimilarityKind::Structural => write!(f, "structural"),
            SimilarityKind::Both => write!(f, "both"),
        }
    }
}

// ── Signature extraction ─────────────────────────────────────────────

/// Extract function signatures from all project files.
fn extract_signatures(rs_files: &[PathBuf], project_path: &Path) -> Vec<FunctionSig> {
    let mut sigs = Vec::new();

    for file in rs_files {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let ast: SynFile = match syn::parse_file(&content) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let rel_path = file
            .strip_prefix(project_path)
            .unwrap_or(file)
            .display()
            .to_string();

        for item in &ast.items {
            match item {
                Item::Fn(func) => {
                    let sig = extract_fn_sig(&func.sig, &func.sig.ident.to_string(), &rel_path);
                    sigs.push(sig);
                }
                Item::Impl(imp) => {
                    let type_name = type_to_short(&imp.self_ty);
                    for impl_item in &imp.items {
                        if let ImplItem::Fn(method) = impl_item {
                            let qualified = format!("{}::{}", type_name, method.sig.ident);
                            let sig = extract_fn_sig(&method.sig, &qualified, &rel_path);
                            sigs.push(sig);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    sigs
}

fn extract_fn_sig(sig: &syn::Signature, name: &str, file: &str) -> FunctionSig {
    let param_types: Vec<String> = sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            syn::FnArg::Typed(pat_type) => Some(type_to_short(&pat_type.ty)),
            syn::FnArg::Receiver(_) => Some("self".to_string()),
        })
        .collect();

    let return_type = match &sig.output {
        ReturnType::Default => "()".to_string(),
        ReturnType::Type(_, ty) => type_to_short(ty),
    };

    let line_start = sig.fn_token.span.start().line;

    FunctionSig {
        name: name.to_string(),
        file: file.to_string(),
        line_start,
        param_types,
        return_type,
    }
}

fn type_to_short(ty: &Type) -> String {
    match ty {
        Type::Path(tp) => tp
            .path
            .segments
            .iter()
            .map(|s| {
                let ident = s.ident.to_string();
                if let syn::PathArguments::AngleBracketed(args) = &s.arguments {
                    let inner: Vec<String> = args
                        .args
                        .iter()
                        .filter_map(|a| {
                            if let syn::GenericArgument::Type(t) = a {
                                Some(type_to_short(t))
                            } else {
                                None
                            }
                        })
                        .collect();
                    if inner.is_empty() {
                        ident
                    } else {
                        format!("{}<{}>", ident, inner.join(", "))
                    }
                } else {
                    ident
                }
            })
            .collect::<Vec<_>>()
            .join("::"),
        Type::Reference(r) => {
            let mutability = if r.mutability.is_some() { "&mut " } else { "&" };
            format!("{}{}", mutability, type_to_short(&r.elem))
        }
        Type::Tuple(t) => {
            let inner: Vec<String> = t.elems.iter().map(type_to_short).collect();
            format!("({})", inner.join(", "))
        }
        Type::Slice(s) => format!("[{}]", type_to_short(&s.elem)),
        Type::Array(a) => format!("[{}; _]", type_to_short(&a.elem)),
        Type::ImplTrait(_) => "impl Trait".to_string(),
        Type::TraitObject(_) => "dyn Trait".to_string(),
        _ => "?".to_string(),
    }
}

// ── Pass 1: Signature similarity ─────────────────────────────────────

/// Compute normalized Levenshtein distance between two strings (0.0 = identical, 1.0 = completely different).
fn levenshtein_distance(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    if m == 0 && n == 0 {
        return 0.0;
    }
    if m == 0 || n == 0 {
        return 1.0;
    }

    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];

    for j in 0..=n {
        prev[j] = j;
    }

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    let max_len = m.max(n);
    prev[n] as f64 / max_len as f64
}

/// Name similarity: 1.0 - normalized_levenshtein. Also checks substring containment.
fn name_similarity(a: &str, b: &str) -> f64 {
    // Use the "bare" name (after last ::) for comparison
    let a_bare = a.rsplit("::").next().unwrap_or(a);
    let b_bare = b.rsplit("::").next().unwrap_or(b);

    let lev_sim = 1.0 - levenshtein_distance(a_bare, b_bare);

    // Substring containment bonus
    let substr_sim = if a_bare.len() >= 3 && b_bare.len() >= 3 {
        if a_bare.contains(b_bare) || b_bare.contains(a_bare) {
            let shorter = a_bare.len().min(b_bare.len()) as f64;
            let longer = a_bare.len().max(b_bare.len()) as f64;
            shorter / longer
        } else {
            0.0
        }
    } else {
        0.0
    };

    lev_sim.max(substr_sim)
}

/// Jaccard index on parameter type sets.
fn param_type_jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();

    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;

    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Compute signature similarity between two functions. Returns 0.0..1.0.
fn signature_similarity(a: &FunctionSig, b: &FunctionSig) -> f64 {
    let name_sim = name_similarity(&a.name, &b.name);
    let param_sim = param_type_jaccard(&a.param_types, &b.param_types);
    let ret_sim = if a.return_type == b.return_type {
        1.0
    } else {
        0.0
    };

    // Weighted combination
    0.4 * name_sim + 0.4 * param_sim + 0.2 * ret_sim
}

/// Find signature-similar pairs above a threshold.
fn find_signature_pairs(sigs: &[FunctionSig], threshold: f64) -> Vec<(usize, usize, f64)> {
    let mut pairs = Vec::new();

    for i in 0..sigs.len() {
        for j in (i + 1)..sigs.len() {
            // Skip trivially tiny functions (getters, etc.)
            if sigs[i].param_types.is_empty()
                && sigs[j].param_types.is_empty()
                && sigs[i].return_type == "()"
                && sigs[j].return_type == "()"
            {
                continue;
            }

            let sim = signature_similarity(&sigs[i], &sigs[j]);
            if sim >= threshold {
                pairs.push((i, j, sim));
            }
        }
    }

    pairs
}

// ── Pass 2: Structural similarity ────────────────────────────────────

/// Normalized AST node kind (stripping identifiers and literals).
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum NormNode {
    Block(usize), // number of statements
    If,
    Match(usize), // number of arms
    For,
    While,
    Loop,
    Let,
    Return,
    Call(usize), // number of arguments
    MethodCall(usize),
    Binary,
    Unary,
    Closure(usize), // number of params
    Index,
    Field,
    Assign,
    Break,
    Continue,
    Literal,
    Path,
    Other,
}

/// Extract a structural hash from a function body by normalizing the AST.
fn structural_hash_from_file(
    file_path: &Path,
    _project_path: &Path,
    func_name: &str,
) -> Option<u64> {
    let content = std::fs::read_to_string(file_path).ok()?;
    let ast = syn::parse_file(&content).ok()?;

    for item in &ast.items {
        match item {
            Item::Fn(func) => {
                if func.sig.ident == func_name || func_name == func.sig.ident.to_string() {
                    return Some(hash_block(&func.block));
                }
            }
            Item::Impl(imp) => {
                let type_name = type_to_short(&imp.self_ty);
                for impl_item in &imp.items {
                    if let ImplItem::Fn(method) = impl_item {
                        let qualified = format!("{}::{}", type_name, method.sig.ident);
                        if qualified == func_name {
                            return Some(hash_block(&method.block));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Hash a block by normalizing its structure.
fn hash_block(block: &syn::Block) -> u64 {
    let mut nodes = Vec::new();
    let mut visitor = StructuralVisitor { nodes: &mut nodes };
    visitor.visit_block(block);

    let mut hasher = DefaultHasher::new();
    nodes.hash(&mut hasher);
    hasher.finish()
}

struct StructuralVisitor<'a> {
    nodes: &'a mut Vec<NormNode>,
}

impl<'a, 'ast> Visit<'ast> for StructuralVisitor<'a> {
    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.nodes.push(NormNode::Block(block.stmts.len()));
        syn::visit::visit_block(self, block);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::If(_) => self.nodes.push(NormNode::If),
            Expr::Match(m) => self.nodes.push(NormNode::Match(m.arms.len())),
            Expr::ForLoop(_) => self.nodes.push(NormNode::For),
            Expr::While(_) => self.nodes.push(NormNode::While),
            Expr::Loop(_) => self.nodes.push(NormNode::Loop),
            Expr::Return(_) => self.nodes.push(NormNode::Return),
            Expr::Call(c) => self.nodes.push(NormNode::Call(c.args.len())),
            Expr::MethodCall(m) => self.nodes.push(NormNode::MethodCall(m.args.len())),
            Expr::Binary(_) => self.nodes.push(NormNode::Binary),
            Expr::Unary(_) => self.nodes.push(NormNode::Unary),
            Expr::Closure(c) => self.nodes.push(NormNode::Closure(c.inputs.len())),
            Expr::Index(_) => self.nodes.push(NormNode::Index),
            Expr::Field(_) => self.nodes.push(NormNode::Field),
            Expr::Assign(_) => self.nodes.push(NormNode::Assign),
            Expr::Break(_) => self.nodes.push(NormNode::Break),
            Expr::Continue(_) => self.nodes.push(NormNode::Continue),
            Expr::Lit(_) => self.nodes.push(NormNode::Literal),
            Expr::Path(_) => self.nodes.push(NormNode::Path),
            _ => self.nodes.push(NormNode::Other),
        }
        syn::visit::visit_expr(self, expr);
    }

    fn visit_local(&mut self, _local: &'ast syn::Local) {
        self.nodes.push(NormNode::Let);
        syn::visit::visit_local(self, _local);
    }
}

// ── Analysis entry point ─────────────────────────────────────────────

const SIG_THRESHOLD: f64 = 0.6;
const REPORT_THRESHOLD: f64 = 0.7;

/// Find structural duplicate pairs from functions sharing identical AST hashes.
fn find_structural_duplicates(
    sigs: &[FunctionSig],
    structural_hashes: &HashMap<usize, u64>,
) -> (Vec<RedundantPair>, HashSet<(usize, usize)>) {
    let mut hash_to_indices: HashMap<u64, Vec<usize>> = HashMap::new();
    for (&idx, &hash) in structural_hashes {
        hash_to_indices.entry(hash).or_default().push(idx);
    }

    let mut results: Vec<RedundantPair> = Vec::new();
    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();

    for indices in hash_to_indices.values() {
        if indices.len() < 2 {
            continue;
        }
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = indices[i].min(indices[j]);
                let b = indices[i].max(indices[j]);
                if seen_pairs.insert((a, b)) {
                    let sig_sim = signature_similarity(&sigs[a], &sigs[b]);
                    let kind = if sig_sim >= SIG_THRESHOLD {
                        SimilarityKind::Both
                    } else {
                        SimilarityKind::Structural
                    };
                    results.push(RedundantPair {
                        func_a: sigs[a].name.clone(),
                        file_a: sigs[a].file.clone(),
                        line_a: sigs[a].line_start,
                        func_b: sigs[b].name.clone(),
                        file_b: sigs[b].file.clone(),
                        line_b: sigs[b].line_start,
                        similarity: 1.0,
                        kind,
                    });
                }
            }
        }
    }

    (results, seen_pairs)
}

/// Merge signature-similar pairs that were not already found as structural duplicates.
fn merge_signature_pairs(
    sig_pairs: &[(usize, usize, f64)],
    sigs: &[FunctionSig],
    structural_hashes: &HashMap<usize, u64>,
    seen_pairs: &mut HashSet<(usize, usize)>,
) -> Vec<RedundantPair> {
    let mut results = Vec::new();

    for (i, j, sim) in sig_pairs {
        let a = (*i).min(*j);
        let b = (*i).max(*j);
        if seen_pairs.contains(&(a, b)) || *sim < REPORT_THRESHOLD {
            continue;
        }

        let struct_match = match (structural_hashes.get(&a), structural_hashes.get(&b)) {
            (Some(ha), Some(hb)) => ha == hb,
            _ => false,
        };

        let (final_sim, kind) = if struct_match {
            (1.0, SimilarityKind::Both)
        } else {
            (*sim, SimilarityKind::Signature)
        };

        seen_pairs.insert((a, b));
        results.push(RedundantPair {
            func_a: sigs[a].name.clone(),
            file_a: sigs[a].file.clone(),
            line_a: sigs[a].line_start,
            func_b: sigs[b].name.clone(),
            file_b: sigs[b].file.clone(),
            line_b: sigs[b].line_start,
            similarity: final_sim,
            kind,
        });
    }

    results
}

/// Run redundancy detection on the project. Returns pairs sorted by similarity descending.
pub fn detect_redundancy(rs_files: &[PathBuf], project_path: &Path) -> Vec<RedundantPair> {
    let sigs = extract_signatures(rs_files, project_path);

    if sigs.len() < 2 {
        return Vec::new();
    }

    // Pass 1: signature similarity
    let sig_pairs = find_signature_pairs(&sigs, SIG_THRESHOLD);

    // Pass 2: structural hashes for all functions
    let mut structural_hashes: HashMap<usize, u64> = HashMap::new();
    for (i, sig) in sigs.iter().enumerate() {
        let full_path = project_path.join(&sig.file);
        if let Some(hash) = structural_hash_from_file(&full_path, project_path, &sig.name) {
            structural_hashes.insert(i, hash);
        }
    }

    // Find structural duplicates
    let (mut results, mut seen_pairs) = find_structural_duplicates(&sigs, &structural_hashes);

    // Merge in signature-similar pairs
    let sig_results = merge_signature_pairs(&sig_pairs, &sigs, &structural_hashes, &mut seen_pairs);
    results.extend(sig_results);

    // Sort by similarity descending
    results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    results
}

// ── Human-readable rendering ─────────────────────────────────────────

pub fn render_redundancy(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let pairs = detect_redundancy(rs_files, project_path);

    println!(
        "\n{}",
        "── Redundancy Detection ──────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Detects suspected duplicate or near-duplicate functions.",
            "Pass 1: Signature similarity — compares function names, parameter types, and return types.",
            "Pass 2: Structural similarity — normalizes ASTs (strips identifiers/literals) and compares hashes.",
            "Similarity 1.00 = structurally identical. >= 0.70 = signature-similar candidates.",
            "Kind: 'signature' = similar signatures, 'structural' = identical AST structure, 'both' = both match.",
        ]);
        render::guide_ref("redundancy");
    }

    if pairs.is_empty() {
        println!("\n  {}", "No suspected duplicates found.".green());
        println!();
        return;
    }

    println!(
        "\n  {} suspected duplicate pair(s) found:\n",
        format!("{}", pairs.len()).bold()
    );

    // Header
    println!(
        "  {:<35} {:<35} {:>5} {}",
        "Function A".bold().underline(),
        "Function B".bold().underline(),
        "Sim".bold().underline(),
        "Kind".bold().underline(),
    );

    for pair in &pairs {
        let name_a = if pair.func_a.len() > 33 {
            format!("{}...", &pair.func_a[..30])
        } else {
            pair.func_a.clone()
        };
        let name_b = if pair.func_b.len() > 33 {
            format!("{}...", &pair.func_b[..30])
        } else {
            pair.func_b.clone()
        };

        let sim_str = format!("{:.2}", pair.similarity);
        let colored_sim = if pair.similarity >= 0.9 {
            sim_str.red().bold().to_string()
        } else {
            sim_str.yellow().to_string()
        };

        let kind_str = match pair.kind {
            SimilarityKind::Both => "both".red().to_string(),
            SimilarityKind::Structural => "structural".yellow().to_string(),
            SimilarityKind::Signature => "signature".dimmed().to_string(),
        };

        println!(
            "  {:<35} {:<35} {:>5} {}",
            name_a.dimmed(),
            name_b.dimmed(),
            colored_sim,
            kind_str,
        );
        println!(
            "  {:<35} {}",
            format!("  {}:{}", pair.file_a, pair.line_a).dimmed(),
            format!("  {}:{}", pair.file_b, pair.line_b).dimmed(),
        );
    }

    println!();
}

// ── JSON rendering ───────────────────────────────────────────────────

#[derive(Serialize)]
struct RedundantPairJson {
    func_a: String,
    file_a: String,
    line_a: usize,
    func_b: String,
    file_b: String,
    line_b: usize,
    similarity: f64,
    kind: String,
}

#[derive(Serialize)]
struct RedundancyJson {
    cstat_version: String,
    redundant_pairs: Vec<RedundantPairJson>,
    total_pairs: usize,
}

pub fn render_redundancy_json(rs_files: &[PathBuf], project_path: &Path) {
    let pairs = detect_redundancy(rs_files, project_path);

    let total = pairs.len();
    let json_pairs: Vec<RedundantPairJson> = pairs
        .into_iter()
        .map(|p| RedundantPairJson {
            func_a: p.func_a,
            file_a: p.file_a,
            line_a: p.line_a,
            func_b: p.func_b,
            file_b: p.file_b,
            line_b: p.line_b,
            similarity: p.similarity,
            kind: p.kind.to_string(),
        })
        .collect();

    let output = RedundancyJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        redundant_pairs: json_pairs,
        total_pairs: total,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
