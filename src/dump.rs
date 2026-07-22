use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::ast_parser;
use crate::complexity;
use crate::deps;
use crate::diagnostics;
use crate::loc;

// ── Top-level dump output ────────────────────────────────────────

#[derive(Serialize)]
struct DumpOutput {
    cstat_version: String,
    project: ProjectInfo,
    scores: diagnostics::HealthScores,
    diagnostic_counts: DiagnosticCounts,
    worst_items: Vec<diagnostics::Diagnostic>,
    suggested_drilldowns: Vec<Drilldown>,
}

#[derive(Serialize)]
struct ProjectInfo {
    files: usize,
    total_loc: usize,
    functions: usize,
    modules: usize,
}

// ── Diagnostic counts ───────────────────────────────────────────

#[derive(Serialize)]
struct DiagnosticCounts {
    total: usize,
    alerts: usize,
    warns: usize,
    by_category: BTreeMap<String, CategoryCounts>,
}

#[derive(Serialize)]
struct CategoryCounts {
    alerts: usize,
    warns: usize,
}

// ── Drilldowns ──────────────────────────────────────────────────

#[derive(Serialize)]
struct Drilldown {
    command: String,
    reason: String,
}

// ── Public entry point ───────────────────────────────────────────

pub fn render_dump(rs_files: &[PathBuf], project_path: &Path) {
    let version = env!("CARGO_PKG_VERSION").to_string();

    let (file_stats, agg_loc) = loc::analyze_files(rs_files);
    let symbols = ast_parser::parse_project(rs_files);
    let func_complexities = complexity::compute_all(&symbols, project_path);
    let dep_analysis = deps::analyze_deps(rs_files, project_path);

    let report = diagnostics::run_diagnostics(rs_files, project_path);

    let project = ProjectInfo {
        files: file_stats.len(),
        total_loc: agg_loc.as_ref().map(|a| a.total_loc).unwrap_or(0),
        functions: func_complexities.len(),
        modules: dep_analysis.modules.len(),
    };

    let diagnostic_counts = build_diagnostic_counts(&report.diagnostics);
    let worst_items = build_worst_items(&report.diagnostics);
    let suggested_drilldowns = build_drilldowns(&report.diagnostics);

    let output = DumpOutput {
        cstat_version: version,
        project,
        scores: report.scores,
        diagnostic_counts,
        worst_items,
        suggested_drilldowns,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}

// ── Diagnostic count builder ────────────────────────────────────

fn build_diagnostic_counts(diagnostics: &[diagnostics::Diagnostic]) -> DiagnosticCounts {
    let mut total_alerts = 0usize;
    let mut total_warns = 0usize;
    let mut by_category: BTreeMap<String, CategoryCounts> = BTreeMap::new();

    // Initialize all categories so they always appear
    let all_categories = [
        "bloated_function",
        "redundant_code",
        "high_cognitive",
        "high_complexity",
        "deep_nesting",
        "dead_code",
        "bloated_file",
        "low_cohesion",
        "too_many_params",
        "god_module",
        "high_coupling",
        "cyclic_deps",
    ];
    for cat in &all_categories {
        by_category.insert(
            cat.to_string(),
            CategoryCounts {
                alerts: 0,
                warns: 0,
            },
        );
    }

    for d in diagnostics {
        let cat_name = category_name(&d.category);
        let entry = by_category.entry(cat_name).or_insert(CategoryCounts {
            alerts: 0,
            warns: 0,
        });
        match d.severity {
            diagnostics::Severity::Alert => {
                total_alerts += 1;
                entry.alerts += 1;
            }
            diagnostics::Severity::Warn => {
                total_warns += 1;
                entry.warns += 1;
            }
        }
    }

    DiagnosticCounts {
        total: diagnostics.len(),
        alerts: total_alerts,
        warns: total_warns,
        by_category,
    }
}

// ── Worst items (top 5) ─────────────────────────────────────────

fn build_worst_items(diagnostics: &[diagnostics::Diagnostic]) -> Vec<diagnostics::Diagnostic> {
    // Sort: alerts first, then by metric magnitude descending.
    let mut sorted: Vec<_> = diagnostics.to_vec();
    sorted.sort_by(|a, b| {
        let sev_order = |s: &diagnostics::Severity| match s {
            diagnostics::Severity::Alert => 0,
            diagnostics::Severity::Warn => 1,
        };
        sev_order(&a.severity)
            .cmp(&sev_order(&b.severity))
            .then_with(|| {
                b.metric
                    .partial_cmp(&a.metric)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    sorted.into_iter().take(5).collect()
}

// ── Drilldown suggestions ───────────────────────────────────────

fn build_drilldowns(diagnostics: &[diagnostics::Diagnostic]) -> Vec<Drilldown> {
    // Count alerts per category
    let mut alert_counts: BTreeMap<String, usize> = BTreeMap::new();
    for d in diagnostics {
        if d.severity == diagnostics::Severity::Alert {
            *alert_counts.entry(category_name(&d.category)).or_insert(0) += 1;
        }
    }

    // Map categories to subcommands
    let category_to_command: &[(&[&str], &str)] = &[
        (
            &["high_complexity", "high_cognitive", "deep_nesting"],
            "cstat branching --json",
        ),
        (&["too_many_params"], "cstat signature --json"),
        (&["bloated_function"], "cstat span --json"),
        (&["redundant_code"], "cstat redundancy --json"),
        (&["bloated_file"], "cstat loc --json"),
        (
            &["low_cohesion", "high_coupling", "god_module"],
            "cstat deps --json",
        ),
        (&["dead_code"], "cstat test-reachability --json"),
        (&["cyclic_deps"], "cstat graph --json"),
    ];

    let mut drilldowns: Vec<(usize, String, Vec<String>)> = Vec::new();

    for (categories, command) in category_to_command {
        let mut total_alerts = 0usize;
        let mut reasons: Vec<String> = Vec::new();

        for cat in *categories {
            if let Some(&count) = alert_counts.get(*cat) {
                total_alerts += count;
                let display = cat.replace('_', " ");
                reasons.push(format!("{} alert-level {}", count, display));
            }
        }

        if total_alerts > 0 {
            drilldowns.push((total_alerts, command.to_string(), reasons));
        }
    }

    // Sort by total alert count descending
    drilldowns.sort_by(|a, b| b.0.cmp(&a.0));

    drilldowns
        .into_iter()
        .map(|(_, command, reasons)| Drilldown {
            command,
            reason: reasons.join(", "),
        })
        .collect()
}

// ── Helpers ─────────────────────────────────────────────────────

fn category_name(cat: &diagnostics::Category) -> String {
    // Match the serde snake_case serialization
    match cat {
        diagnostics::Category::BloatedFile => "bloated_file",
        diagnostics::Category::BloatedFunction => "bloated_function",
        diagnostics::Category::HighComplexity => "high_complexity",
        diagnostics::Category::HighCognitive => "high_cognitive",
        diagnostics::Category::DeepNesting => "deep_nesting",
        diagnostics::Category::HighCoupling => "high_coupling",
        diagnostics::Category::GodModule => "god_module",
        diagnostics::Category::DeadCode => "dead_code",
        diagnostics::Category::LowCohesion => "low_cohesion",
        diagnostics::Category::RedundantCode => "redundant_code",
        diagnostics::Category::TooManyParams => "too_many_params",
        diagnostics::Category::CyclicDeps => "cyclic_deps",
    }
    .to_string()
}
