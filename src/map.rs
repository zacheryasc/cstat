use colored::Colorize;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::ast_parser::{self, ProjectSymbols};
use crate::complexity;
use crate::deps;
use crate::loc;
use crate::render;

/// Information about a module for the map view.
struct ModuleBox {
    name: String,
    structs: Vec<String>,
    enums: Vec<String>,
    traits: Vec<String>,
    functions: Vec<String>,
    pub_count: usize,
    total_count: usize,
    code_lines: usize,
    /// Per-function complexity scores (for heatbar).
    func_scores: Vec<f64>,
    /// Total connectivity (in-degree + out-degree).
    connectivity: usize,
}

/// An inter-tier connection.
struct TierEdge {
    from_tier: usize,
    to_tier: usize,
    from_mod: String,
    to_mod: String,
}

/// Render the bird's eye module map.
pub fn render_map(rs_files: &[PathBuf], project_path: &Path, verbose: bool) {
    let term_w = render::terminal_width();
    let symbols = ast_parser::parse_project(rs_files);
    let dep_analysis = deps::analyze_deps(rs_files, project_path);

    println!(
        "\n{}",
        "── Architectural Module Map ───────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Bird's-eye module map arranged by dependency depth (BFS from roots).",
            "Tier 0 = Entry Points (zero in-degree — nothing depends on them).",
            "Deepest tier = Leaf Modules (only depended upon, never import others).",
            "Inside each box: S = structs, E = enums, T = traits, f = functions.",
            "  Numbers after the module name: LoC count and pub/total item ratio.",
            "  ● (white) in the legend = public items (pub fn, pub struct, etc.).",
            "Heatbar (▁▃▅▇) = per-function complexity distribution within the module:",
            "  green (▁) = low complexity, yellow (▅) = medium, red (▇) = high.",
            "Border style: ╔ (double-line, bold) = hub module (connectivity > mean+1σ).",
            "  ┌ (single-line, dimmed) = regular or leaf module.",
            "Arrows (───▶) show cross-tier dependency flow between modules.",
            "  Arrow weight in parentheses = number of dependency edges along that path.",
        ]);
        render::guide_ref("architecture");
    }

    if symbols.files.is_empty() {
        println!("  {}", "No modules found.".dimmed());
        return;
    }

    // Get LoC data per file
    let (file_stats, _) = loc::analyze_files(rs_files);
    let mut loc_by_mod: HashMap<String, usize> = HashMap::new();
    for stat in &file_stats {
        let mod_name = render::module_name_from_path(&stat.path, project_path);
        loc_by_mod.insert(mod_name, stat.code_lines);
    }

    // Get complexity data
    let func_complexities = complexity::compute_all(&symbols, project_path);
    let mut complexity_by_mod: HashMap<String, Vec<f64>> = HashMap::new();
    for fc in &func_complexities {
        let mod_name = file_to_module_name(&fc.file);
        complexity_by_mod
            .entry(mod_name)
            .or_default()
            .push(fc.composite_score);
    }

    // Build module boxes
    let module_boxes = build_module_boxes(
        &symbols,
        project_path,
        &loc_by_mod,
        &complexity_by_mod,
        &dep_analysis,
    );

    if module_boxes.is_empty() {
        println!("  {}", "No modules found.".dimmed());
        return;
    }

    // Compute tier assignments via dependency depth (BFS from roots)
    let tiers = compute_tiers(&module_boxes, &dep_analysis);

    // Group modules by tier
    let max_tier = tiers.values().copied().max().unwrap_or(0);
    let mut tier_groups: Vec<Vec<String>> = vec![Vec::new(); max_tier + 1];
    for (name, &tier) in &tiers {
        tier_groups[tier].push(name.clone());
    }
    // Sort modules within each tier by LoC descending
    for group in &mut tier_groups {
        group.sort_by(|a, b| {
            let a_loc = module_boxes.get(a.as_str()).map(|m| m.code_lines).unwrap_or(0);
            let b_loc = module_boxes.get(b.as_str()).map(|m| m.code_lines).unwrap_or(0);
            b_loc.cmp(&a_loc).then(a.cmp(b))
        });
    }

    // Collect inter-tier edges
    let mut tier_edges: Vec<TierEdge> = Vec::new();
    for edge in &dep_analysis.edges {
        let from_tier = tiers.get(edge.from.as_str()).copied().unwrap_or(0);
        let to_tier = tiers.get(edge.to.as_str()).copied().unwrap_or(0);
        tier_edges.push(TierEdge {
            from_tier,
            to_tier,
            from_mod: edge.from.clone(),
            to_mod: edge.to.clone(),
        });
    }

    // Find max LoC for proportional sizing
    let max_loc = module_boxes
        .values()
        .map(|m| m.code_lines)
        .max()
        .unwrap_or(1)
        .max(1);

    // Compute connectivity threshold for hub detection
    let connectivities: Vec<usize> = module_boxes.values().map(|m| m.connectivity).collect();
    let mean_conn = if connectivities.is_empty() {
        0.0
    } else {
        connectivities.iter().sum::<usize>() as f64 / connectivities.len() as f64
    };
    let hub_threshold = (mean_conn + 1.0) as usize;

    // Render tier by tier (top = tier 0 = entry points, bottom = leaves)
    for tier_idx in 0..=max_tier {
        let group = &tier_groups[tier_idx];
        if group.is_empty() {
            continue;
        }

        // Tier header
        let tier_label = match tier_idx {
            0 => "Entry Points".to_string(),
            t if t == max_tier && max_tier > 0 => "Leaf Modules".to_string(),
            t => format!("Layer {}", t),
        };
        let separator = format!(
            "╶╶╶ {} {}",
            tier_label,
            "╶".repeat(term_w.saturating_sub(tier_label.len()).saturating_sub(6))
        );
        println!("\n{}", separator.dimmed());

        // Decide layout: side-by-side if terminal is wide enough
        let box_min_width = 30;
        let max_side_by_side = (term_w / box_min_width).max(1);

        // Render modules in rows of max_side_by_side
        let mut i = 0;
        while i < group.len() {
            let row_end = (i + max_side_by_side).min(group.len());
            let row = &group[i..row_end];
            let col_count = row.len();
            let col_width = if col_count > 1 {
                (term_w / col_count).min(term_w).max(box_min_width)
            } else {
                term_w.min(80).max(box_min_width)
            };

            // Render boxes side-by-side
            let mut box_lines: Vec<Vec<String>> = Vec::new();
            for mod_name in row {
                if let Some(mbox) = module_boxes.get(mod_name.as_str()) {
                    let is_hub = mbox.connectivity >= hub_threshold && hub_threshold > 0;
                    let height = proportional_height(mbox.code_lines, max_loc);
                    let lines =
                        render_module_box(mbox, col_width.saturating_sub(1), is_hub, height);
                    box_lines.push(lines);
                }
            }

            let max_lines = box_lines.iter().map(|b| b.len()).max().unwrap_or(0);

            for line_idx in 0..max_lines {
                let mut row_str = String::new();
                for (col_idx, lines) in box_lines.iter().enumerate() {
                    let line = if line_idx < lines.len() {
                        &lines[line_idx]
                    } else {
                        ""
                    };
                    let vis_len = render::visible_len(line);
                    let pad = col_width.saturating_sub(1).saturating_sub(vis_len);
                    row_str.push_str(line);
                    row_str.push_str(&" ".repeat(pad));
                    if col_idx < box_lines.len() - 1 {
                        row_str.push(' ');
                    }
                }
                println!("{}", row_str);
            }

            i = row_end;
        }

        // Render weighted arrows from this tier to deeper tiers
        let arrows_from_tier: Vec<&TierEdge> = tier_edges
            .iter()
            .filter(|e| e.from_tier == tier_idx && e.to_tier > tier_idx)
            .collect();

        if !arrows_from_tier.is_empty() {
            let mut arrow_groups: HashMap<(&str, &str), usize> = HashMap::new();
            for e in &arrows_from_tier {
                *arrow_groups
                    .entry((e.from_mod.as_str(), e.to_mod.as_str()))
                    .or_insert(0) += 1;
            }
            let mut arrow_list: Vec<((&str, &str), usize)> =
                arrow_groups.into_iter().collect();
            arrow_list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0 .0.cmp(b.0 .0)));

            let show_count = arrow_list.len().min(6);
            let mut arrow_strs: Vec<String> = Vec::new();
            for ((from, to), count) in arrow_list.iter().take(show_count) {
                let weight_str = if *count > 1 {
                    format!("({})", count)
                } else {
                    String::new()
                };
                arrow_strs.push(format!(
                    "{}→{}{}",
                    from.bright_blue(),
                    to.bright_blue(),
                    weight_str.dimmed()
                ));
            }
            if arrow_list.len() > show_count {
                arrow_strs.push(
                    format!("+{} more", arrow_list.len() - show_count)
                        .dimmed()
                        .to_string(),
                );
            }
            println!("    {} {}", "───▶".dimmed(), arrow_strs.join("  "));
        }
    }

    // Legend
    println!();
    println!(
        "  {} struct  {} enum  {} trait  {} fn  {} pub  {} hub  {} leaf",
        "S".bright_green().bold(),
        "E".bright_yellow().bold(),
        "T".bright_magenta().bold(),
        "f".bright_blue(),
        "●".white().bold(),
        "╔".bold(),
        "┌".dimmed(),
    );
    println!(
        "  {} complexity heatbar: {} low {} med {} high",
        "▁▃▅▇".dimmed(),
        "▁".green(),
        "▅".yellow(),
        "▇".red(),
    );
    println!();
}

/// Convert a file path like "src/foo.rs" to module name "foo".
fn file_to_module_name(file: &str) -> String {
    let s = file.strip_suffix(".rs").unwrap_or(file);
    let s = s.replace('/', "::");
    let s = s.strip_prefix("src::").unwrap_or(&s);
    if s.ends_with("::mod") {
        s.strip_suffix("::mod").unwrap_or(s).to_string()
    } else {
        s.to_string()
    }
}

/// Build module boxes with LoC, complexity, and connectivity data.
fn build_module_boxes(
    symbols: &ProjectSymbols,
    project_path: &Path,
    loc_by_mod: &HashMap<String, usize>,
    complexity_by_mod: &HashMap<String, Vec<f64>>,
    dep_analysis: &deps::DepAnalysis,
) -> HashMap<String, ModuleBox> {
    let mut boxes: HashMap<String, ModuleBox> = HashMap::new();

    for file_syms in &symbols.files {
        let mod_name = render::module_name_from_path(&file_syms.path, project_path);

        let structs: Vec<String> = file_syms.structs.iter().map(|s| s.name.clone()).collect();
        let enums: Vec<String> = file_syms.enums.iter().map(|e| e.name.clone()).collect();
        let traits: Vec<String> = file_syms.traits.iter().map(|t| t.name.clone()).collect();
        let functions: Vec<String> = file_syms
            .functions
            .iter()
            .filter(|f| !f.name.contains("::"))
            .map(|f| f.name.clone())
            .collect();

        let total = structs.len() + enums.len() + traits.len() + functions.len();
        let pub_count = count_pub_items(&file_syms.path);
        let code_lines = loc_by_mod.get(&mod_name).copied().unwrap_or(0);

        let func_scores = complexity_by_mod
            .get(&mod_name)
            .cloned()
            .unwrap_or_default();

        let in_d = dep_analysis
            .in_degree
            .get(&mod_name)
            .copied()
            .unwrap_or(0);
        let out_d = dep_analysis
            .out_degree
            .get(&mod_name)
            .copied()
            .unwrap_or(0);
        let connectivity = in_d + out_d;

        boxes.insert(
            mod_name.clone(),
            ModuleBox {
                name: mod_name,
                structs,
                enums,
                traits,
                functions,
                pub_count,
                total_count: total,
                code_lines,
                func_scores,
                connectivity,
            },
        );
    }

    boxes
}

/// Count `pub fn`, `pub struct`, `pub enum`, `pub trait` in source file.
fn count_pub_items(path: &Path) -> usize {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let ast = match syn::parse_file(&content) {
        Ok(f) => f,
        Err(_) => return 0,
    };
    let mut count = 0;
    for item in &ast.items {
        match item {
            syn::Item::Fn(f) => {
                if matches!(f.vis, syn::Visibility::Public(_)) {
                    count += 1;
                }
            }
            syn::Item::Struct(s) => {
                if matches!(s.vis, syn::Visibility::Public(_)) {
                    count += 1;
                }
            }
            syn::Item::Enum(e) => {
                if matches!(e.vis, syn::Visibility::Public(_)) {
                    count += 1;
                }
            }
            syn::Item::Trait(t) => {
                if matches!(t.vis, syn::Visibility::Public(_)) {
                    count += 1;
                }
            }
            _ => {}
        }
    }
    count
}

/// Compute dependency depth tiers via BFS from root modules (zero in-degree).
fn compute_tiers(
    module_boxes: &HashMap<String, ModuleBox>,
    dep_analysis: &deps::DepAnalysis,
) -> HashMap<String, usize> {
    let mut tiers: HashMap<String, usize> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    // Build adjacency list: from -> [to] (follows dependency direction)
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &dep_analysis.edges {
        adj.entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
    }

    // Start from modules with zero in-degree (entry points / roots)
    for name in module_boxes.keys() {
        let in_d = dep_analysis.in_degree.get(name).copied().unwrap_or(0);
        if in_d == 0 {
            tiers.insert(name.clone(), 0);
            queue.push_back(name.clone());
        }
    }

    // If no roots found (all cycles), start everyone at tier 0
    if queue.is_empty() {
        for name in module_boxes.keys() {
            tiers.insert(name.clone(), 0);
        }
        return tiers;
    }

    // BFS: propagate max depth. Limit iterations to avoid infinite loops on cycles.
    let max_iterations = module_boxes.len() * module_boxes.len() + 1;
    let mut iterations = 0;
    while let Some(current) = queue.pop_front() {
        iterations += 1;
        if iterations > max_iterations {
            break;
        }
        let current_tier = tiers[&current];
        if let Some(neighbors) = adj.get(current.as_str()) {
            for &neighbor in neighbors {
                let new_tier = current_tier + 1;
                let existing = tiers.get(neighbor).copied();
                if existing.is_none() || new_tier > existing.unwrap() {
                    // Cap tier depth to prevent runaway on cycles
                    if new_tier <= module_boxes.len() {
                        tiers.insert(neighbor.to_string(), new_tier);
                        queue.push_back(neighbor.to_string());
                    }
                }
            }
        }
    }

    // Ensure all modules have a tier
    for name in module_boxes.keys() {
        tiers.entry(name.clone()).or_insert(0);
    }

    tiers
}

/// Compute proportional height (content lines) based on LoC.
/// Minimum 1, maximum 6.
fn proportional_height(code_lines: usize, max_loc: usize) -> usize {
    if max_loc == 0 || code_lines == 0 {
        return 1;
    }
    let ratio = code_lines as f64 / max_loc as f64;
    let height = (ratio * 5.0).ceil() as usize + 1;
    height.clamp(1, 6)
}

/// Render a single module box, returning lines.
fn render_module_box(
    mbox: &ModuleBox,
    box_width: usize,
    is_hub: bool,
    prop_height: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    let box_width = box_width.max(20);
    let inner_w = box_width.saturating_sub(4); // "│ " + " │" = 4 chars

    // Border characters: hubs get bold double-line borders
    let (tl, tr, bl, br, h, v) = if is_hub {
        ("╔", "╗", "╚", "╝", "═", "║")
    } else {
        ("┌", "┐", "└", "┘", "─", "│")
    };

    // Header line
    let header = format!(" {} ", mbox.name);
    let pub_hint = format!("{}/{} pub", mbox.pub_count, mbox.total_count);
    let loc_hint = format!("{} loc", mbox.code_lines);
    let meta = format!("{} {}", loc_hint, pub_hint);
    let fill_len = box_width
        .saturating_sub(2)
        .saturating_sub(header.len())
        .saturating_sub(meta.len())
        .saturating_sub(2);
    let fill = h.repeat(fill_len);

    if is_hub {
        lines.push(format!(
            "{}{}{}{}{}{}",
            tl.bold(),
            h.repeat(1).bold(),
            header.white().bold(),
            fill.bold(),
            format!(" {} ", meta).dimmed(),
            tr.bold(),
        ));
    } else {
        lines.push(format!(
            "{}{}{}{}{}{}",
            tl.dimmed(),
            h.repeat(1).dimmed(),
            header.white().bold(),
            fill.dimmed(),
            format!(" {} ", meta).dimmed(),
            tr.dimmed(),
        ));
    }

    // Content lines
    let mut content: Vec<String> = Vec::new();

    // Complexity heatbar
    let heatbar = build_heatbar(&mbox.func_scores, inner_w);
    if !heatbar.is_empty() {
        content.push(heatbar);
    }

    // Structs
    if !mbox.structs.is_empty() {
        let items = abbreviate_list(&mbox.structs, inner_w.saturating_sub(2));
        content.push(format!("{} {}", "S".bright_green().bold(), items));
    }

    // Enums
    if !mbox.enums.is_empty() {
        let items = abbreviate_list(&mbox.enums, inner_w.saturating_sub(2));
        content.push(format!("{} {}", "E".bright_yellow().bold(), items));
    }

    // Traits
    if !mbox.traits.is_empty() {
        let items = abbreviate_list(&mbox.traits, inner_w.saturating_sub(2));
        content.push(format!("{} {}", "T".bright_magenta().bold(), items));
    }

    // Functions
    if !mbox.functions.is_empty() {
        let items = abbreviate_list(&mbox.functions, inner_w.saturating_sub(2));
        content.push(format!("{} {}", "f".bright_blue(), items));
    }

    if content.is_empty() {
        content.push(format!("{}", "(empty)".dimmed()));
    }

    // Pad or trim to proportional height
    while content.len() < prop_height {
        content.push(String::new());
    }
    if content.len() > prop_height && prop_height > 0 {
        content.truncate(prop_height);
    }

    // Render content with borders
    for line in &content {
        let vis_len = render::visible_len(line);
        let pad = inner_w.saturating_sub(vis_len);
        if is_hub {
            lines.push(format!(
                "{} {}{} {}",
                v.bold(),
                line,
                " ".repeat(pad),
                v.bold()
            ));
        } else {
            lines.push(format!(
                "{} {}{} {}",
                v.dimmed(),
                line,
                " ".repeat(pad),
                v.dimmed()
            ));
        }
    }

    // Footer
    let footer_fill = box_width.saturating_sub(2);
    if is_hub {
        lines.push(format!(
            "{}{}{}",
            bl.bold(),
            h.repeat(footer_fill).bold(),
            br.bold()
        ));
    } else {
        lines.push(format!(
            "{}{}{}",
            bl.dimmed(),
            h.repeat(footer_fill).dimmed(),
            br.dimmed()
        ));
    }

    lines
}

/// Build a complexity heatbar from function scores.
fn build_heatbar(scores: &[f64], max_width: usize) -> String {
    if scores.is_empty() {
        return String::new();
    }

    let blocks = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max_score = scores.iter().cloned().fold(0.0f64, f64::max).max(1.0);

    let bar_width = max_width.min(scores.len());
    let mut result = String::new();

    if scores.len() <= bar_width {
        let mut sorted_scores = scores.to_vec();
        sorted_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for &score in &sorted_scores {
            let ratio = score / max_score;
            let idx = (ratio * (blocks.len() - 1) as f64).round() as usize;
            let idx = idx.min(blocks.len() - 1);
            let ch = blocks[idx].to_string();
            let colored = if ratio < 0.33 {
                ch.green().to_string()
            } else if ratio < 0.66 {
                ch.yellow().to_string()
            } else {
                ch.red().to_string()
            };
            result.push_str(&colored);
        }
    } else {
        let bucket_size = (scores.len() as f64 / bar_width as f64).ceil() as usize;
        let mut sorted_scores = scores.to_vec();
        sorted_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for chunk in sorted_scores.chunks(bucket_size) {
            let bucket_max = chunk.iter().cloned().fold(0.0f64, f64::max);
            let ratio = bucket_max / max_score;
            let idx = (ratio * (blocks.len() - 1) as f64).round() as usize;
            let idx = idx.min(blocks.len() - 1);
            let ch = blocks[idx].to_string();
            let colored = if ratio < 0.33 {
                ch.green().to_string()
            } else if ratio < 0.66 {
                ch.yellow().to_string()
            } else {
                ch.red().to_string()
            };
            result.push_str(&colored);
        }
    }

    result
}

/// Abbreviate a list of names to fit within the given width.
fn abbreviate_list(names: &[String], max_width: usize) -> String {
    let mut result = String::new();
    let mut remaining = names.len();

    for (i, name) in names.iter().enumerate() {
        let separator = if i > 0 { ", " } else { "" };
        let addition = format!("{}{}", separator, name);

        if result.len() + addition.len() > max_width && i > 0 {
            let more = remaining;
            result.push_str(&format!(" +{}", more));
            break;
        }

        result.push_str(&addition);
        remaining -= 1;
    }

    result
}

#[derive(Serialize)]
struct ModuleMetadataJson {
    name: String,
    structs: Vec<String>,
    enums: Vec<String>,
    traits: Vec<String>,
    functions: Vec<String>,
    pub_count: usize,
    total_count: usize,
    code_lines: usize,
    tier: usize,
    is_hub: bool,
    complexity_scores: Vec<f64>,
}

#[derive(Serialize)]
struct MapJson {
    cstat_version: String,
    modules: Vec<ModuleMetadataJson>,
}

/// Render module map as JSON.
pub fn render_map_json(rs_files: &[PathBuf], project_path: &Path) {
    let symbols = ast_parser::parse_project(rs_files);
    let dep_analysis = deps::analyze_deps(rs_files, project_path);

    let (file_stats, _) = loc::analyze_files(rs_files);
    let mut loc_by_mod: HashMap<String, usize> = HashMap::new();
    for stat in &file_stats {
        let mod_name = render::module_name_from_path(&stat.path, project_path);
        loc_by_mod.insert(mod_name, stat.code_lines);
    }

    let func_complexities = complexity::compute_all(&symbols, project_path);
    let mut complexity_by_mod: HashMap<String, Vec<f64>> = HashMap::new();
    for fc in &func_complexities {
        let mod_name = file_to_module_name(&fc.file);
        complexity_by_mod.entry(mod_name).or_default().push(fc.composite_score);
    }

    let module_boxes = build_module_boxes(
        &symbols, project_path, &loc_by_mod, &complexity_by_mod, &dep_analysis,
    );

    let tiers = compute_tiers(&module_boxes, &dep_analysis);

    let connectivities: Vec<usize> = module_boxes.values().map(|m| m.connectivity).collect();
    let mean_conn = if connectivities.is_empty() {
        0.0
    } else {
        connectivities.iter().sum::<usize>() as f64 / connectivities.len() as f64
    };
    let hub_threshold = (mean_conn + 1.0) as usize;

    let mut modules: Vec<ModuleMetadataJson> = module_boxes
        .values()
        .map(|mbox| ModuleMetadataJson {
            name: mbox.name.clone(),
            structs: mbox.structs.clone(),
            enums: mbox.enums.clone(),
            traits: mbox.traits.clone(),
            functions: mbox.functions.clone(),
            pub_count: mbox.pub_count,
            total_count: mbox.total_count,
            code_lines: mbox.code_lines,
            tier: tiers.get(&mbox.name).copied().unwrap_or(0),
            is_hub: mbox.connectivity >= hub_threshold && hub_threshold > 0,
            complexity_scores: mbox.func_scores.clone(),
        })
        .collect();
    modules.sort_by(|a, b| a.tier.cmp(&b.tier).then(b.code_lines.cmp(&a.code_lines)));

    let output = MapJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        modules,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
