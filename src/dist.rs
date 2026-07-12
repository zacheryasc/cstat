use colored::Colorize;
use serde::Serialize;
use std::path::Path;

use crate::complexity::{self, FunctionComplexity};
use crate::render;

// ── Metric extraction ──────────────────────────────────────────────

const METRIC_NAMES: &[&str] = &["loc", "cyclomatic", "cognitive", "nesting", "params"];

struct MetricSet {
    /// One vector of values per metric, in METRIC_NAMES order.
    columns: Vec<Vec<f64>>,
}

fn extract_metrics(fcs: &[FunctionComplexity]) -> MetricSet {
    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); METRIC_NAMES.len()];
    for fc in fcs {
        columns[0].push(fc.line_count as f64);
        columns[1].push(fc.cyclomatic as f64);
        columns[2].push(fc.cognitive as f64);
        columns[3].push(fc.nesting_depth as f64);
        columns[4].push(fc.param_count as f64);
    }
    MetricSet { columns }
}

// ── Statistics helpers ─────────────────────────────────────────────

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64;
    var.sqrt()
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len();
    if n < 2 {
        return 0.0;
    }
    let mx = mean(x);
    let my = mean(y);
    let mut num = 0.0;
    let mut dx2 = 0.0;
    let mut dy2 = 0.0;
    for i in 0..n {
        let dx = x[i] - mx;
        let dy = y[i] - my;
        num += dx * dy;
        dx2 += dx * dx;
        dy2 += dy * dy;
    }
    let denom = (dx2 * dy2).sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        num / denom
    }
}

fn skewness(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 3 {
        return 0.0;
    }
    let m = mean(v);
    let s = std_dev(v);
    if s < 1e-12 {
        return 0.0;
    }
    let m3 = v.iter().map(|x| ((x - m) / s).powi(3)).sum::<f64>();
    m3 / n as f64
}

fn kurtosis(v: &[f64]) -> f64 {
    let n = v.len();
    if n < 4 {
        return 0.0;
    }
    let m = mean(v);
    let s = std_dev(v);
    if s < 1e-12 {
        return 0.0;
    }
    let m4 = v.iter().map(|x| ((x - m) / s).powi(4)).sum::<f64>();
    m4 / n as f64 - 3.0 // excess kurtosis
}

// ── Histogram ──────────────────────────────────────────────────────

fn render_histogram(values: &[f64], label: &str, num_bins: usize, verbose: bool) {
    if values.is_empty() {
        return;
    }

    let min_v = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_v = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // Cap bins to the actual range for integer-valued data
    let range = max_v - min_v;
    let num_bins = if range.abs() < 1e-12 {
        1
    } else {
        let int_range = range.ceil() as usize;
        num_bins.min(int_range.max(1))
    };
    let bin_width = if num_bins == 1 {
        1.0
    } else {
        range / num_bins as f64
    };

    let mut bins = vec![0usize; num_bins];
    for &v in values {
        let idx = if num_bins == 1 {
            0
        } else {
            ((v - min_v) / bin_width).floor() as usize
        };
        let idx = idx.min(num_bins - 1);
        bins[idx] += 1;
    }

    let max_count = *bins.iter().max().unwrap_or(&1).max(&1);

    let term_w = render::terminal_width();
    let label_w = 12; // "  [xxx, yyy)"
    let count_w = format!("{}", max_count).len() + 1;
    let bar_budget = term_w.saturating_sub(label_w + count_w + 4).max(10);

    println!(
        "\n  {} (n={})",
        label.bright_cyan().bold(),
        format!("{}", values.len()).bold()
    );

    if verbose {
        render::verbose_block(&[
            "Histogram: frequency distribution of values. Each row is a bin range.",
            "Bar length proportional to count. Color: red = most frequent, green = least.",
            "Stats below: skew > 0 = right-tailed (few very high values),",
            "  excess kurtosis > 0 = heavy tails (more outliers than a normal distribution).",
        ]);
        render::guide_ref("complexity");
    }

    for i in 0..num_bins {
        let lo = min_v + i as f64 * bin_width;
        let hi = lo + bin_width;
        let range_label = if num_bins == 1 {
            format!("[{:.0}]", lo)
        } else if i == num_bins - 1 {
            format!("[{:.0},{:.0}]", lo, hi)
        } else {
            format!("[{:.0},{:.0})", lo, hi)
        };

        let bar_len = if max_count > 0 {
            (bins[i] as f64 / max_count as f64 * bar_budget as f64).ceil() as usize
        } else {
            0
        }
        .max(if bins[i] > 0 { 1 } else { 0 });

        let ratio = if max_count > 0 {
            1.0 - (bins[i] as f64 / max_count as f64)
        } else {
            1.0
        };
        let bar = render::bar_color(&"█".repeat(bar_len), ratio);

        println!(
            "  {:>10} │ {}{} {}",
            range_label.dimmed(),
            bar,
            " ".repeat(bar_budget.saturating_sub(bar_len)),
            format!("{}", bins[i]).bold(),
        );
    }

    // Distribution shape stats
    let m = mean(values);
    let sd = std_dev(values);
    let sk = skewness(values);
    let ku = kurtosis(values);
    println!(
        "  {} mean={:.1} σ={:.1} skew={:.2} kurt={:.2}",
        "↳".dimmed(),
        m,
        sd,
        sk,
        ku
    );
}

// ── Scatter plot ───────────────────────────────────────────────────

fn render_scatter(x: &[f64], y: &[f64], x_label: &str, y_label: &str, verbose: bool) {
    if x.is_empty() {
        return;
    }

    let r = pearson(x, y);

    println!(
        "\n{}",
        format!("── {} vs {} (r={:.3}) ", x_label, y_label, r)
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Scatter plot: each ● represents one or more functions at that (x, y) position.",
            "  Green ● = 1 function, Yellow ● = 2-3, Red ● = 4+ (overlapping).",
            "  · = empty cell. X-axis = LoC, Y-axis = composite complexity.",
            "Pearson r measures linear correlation: r > 0.7 = strong positive,",
            "  r ≈ 0 = no linear relationship, r < -0.7 = strong negative.",
        ]);
    }

    let plot_w: usize = render::terminal_width().min(72).saturating_sub(8);
    let plot_h: usize = 20;

    let x_min = x.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_max = x.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let y_min = y.iter().cloned().fold(f64::INFINITY, f64::min);
    let y_max = y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let x_range = if (x_max - x_min).abs() < 1e-12 {
        1.0
    } else {
        x_max - x_min
    };
    let y_range = if (y_max - y_min).abs() < 1e-12 {
        1.0
    } else {
        y_max - y_min
    };

    // Build grid with counts
    let mut grid = vec![vec![0u32; plot_w]; plot_h];
    for i in 0..x.len() {
        let col = ((x[i] - x_min) / x_range * (plot_w - 1) as f64).round() as usize;
        let row = ((y[i] - y_min) / y_range * (plot_h - 1) as f64).round() as usize;
        let col = col.min(plot_w - 1);
        let row = row.min(plot_h - 1);
        grid[row][col] += 1;
    }

    // Render top to bottom (high y first)
    let y_label_w = 6;
    for row in (0..plot_h).rev() {
        let y_val = y_min + (row as f64 / (plot_h - 1).max(1) as f64) * y_range;
        let label = if row == plot_h - 1 || row == 0 || row == plot_h / 2 {
            format!("{:>5.0}", y_val)
        } else {
            "     ".to_string()
        };

        let mut line = String::new();
        for col in 0..plot_w {
            let count = grid[row][col];
            if count == 0 {
                line.push('·');
            } else if count == 1 {
                line.push_str(&"●".green().to_string());
            } else if count < 4 {
                line.push_str(&"●".yellow().to_string());
            } else {
                line.push_str(&"●".red().bold().to_string());
            }
        }

        let border = if row == 0 { "└" } else { "│" };
        println!("{} {}{}", label.dimmed(), border, line);
    }

    // X axis
    let x_min_s = format!("{:.0}", x_min);
    let x_max_s = format!("{:.0}", x_max);
    let mid_x = (x_min + x_max) / 2.0;
    let x_mid_s = format!("{:.0}", mid_x);
    let axis_padding = plot_w.saturating_sub(x_min_s.len() + x_max_s.len() + x_mid_s.len()) / 2;
    println!(
        "{} {}{}{}{}{}",
        " ".repeat(y_label_w),
        x_min_s.dimmed(),
        " ".repeat(axis_padding),
        x_mid_s.dimmed(),
        " ".repeat(axis_padding),
        x_max_s.dimmed(),
    );
    println!(
        "{}  {} →  {}   {} ↑",
        " ".repeat(y_label_w),
        x_label.dimmed(),
        y_label.dimmed(),
        format!("r={:.3}", r).bold(),
    );
}

// ── Outlier detection ──────────────────────────────────────────────

fn render_outliers(fcs: &[FunctionComplexity], metrics: &MetricSet, verbose: bool) {
    println!(
        "\n{}",
        "── Outliers (z-score > 2.0) ───────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Outlier detection: functions with a z-score > 2.0 in any metric.",
            "z-score = (value - mean) / standard deviation. z > 2.0 means the value is",
            "more than 2 standard deviations above the mean — statistically unusual.",
            "These are candidates for refactoring or closer inspection.",
            "Grouped by metric, showing up to 5 outliers per metric.",
        ]);
    }

    let mut any_outlier = false;

    for (mi, metric_name) in METRIC_NAMES.iter().enumerate() {
        let vals = &metrics.columns[mi];
        let m = mean(vals);
        let s = std_dev(vals);
        if s < 1e-12 {
            continue;
        }

        let mut outliers: Vec<(usize, f64)> = Vec::new();
        for (i, &v) in vals.iter().enumerate() {
            let z = (v - m) / s;
            if z > 2.0 {
                outliers.push((i, z));
            }
        }
        outliers.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        if !outliers.is_empty() {
            any_outlier = true;
            println!(
                "\n  {} ({} outlier{})",
                metric_name.yellow().bold(),
                outliers.len(),
                if outliers.len() == 1 { "" } else { "s" }
            );

            for (i, z) in outliers.iter().take(5) {
                let name = if fcs[*i].name.len() > 45 {
                    format!("{}...", &fcs[*i].name[..42])
                } else {
                    fcs[*i].name.clone()
                };
                println!(
                    "    {} {} (z={:.2}, val={:.0})",
                    "▸".red(),
                    name.dimmed(),
                    z,
                    vals[*i],
                );
            }
        }
    }

    if !any_outlier {
        println!("  {}", "No outliers detected.".dimmed());
    }
}

// ── Correlation matrix ─────────────────────────────────────────────

fn render_correlation_matrix(metrics: &MetricSet, verbose: bool) {
    let n = METRIC_NAMES.len();

    println!(
        "\n{}",
        "── Correlation Matrix ─────────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Pairwise Pearson correlation coefficients (r) between all metrics.",
            "r ranges from -1.0 (perfect negative) to +1.0 (perfect positive).",
            "Color: red/bold = strong positive (r > 0.7), blue/bold = strong negative (r < -0.7),",
            "  yellow = moderate positive (r > 0.4), cyan = moderate negative (r < -0.4),",
            "  dim = weak correlation (|r| ≤ 0.4).",
            "Diagonal is always 1.000 (a metric perfectly correlates with itself).",
            "High correlation between two metrics suggests redundancy or a shared underlying factor.",
        ]);
    }

    // Header row
    print!("  {:>12}", "");
    for name in METRIC_NAMES {
        print!(" {:>10}", name.bold());
    }
    println!();

    for i in 0..n {
        print!("  {:>12}", METRIC_NAMES[i].bold());
        for j in 0..n {
            let r = pearson(&metrics.columns[i], &metrics.columns[j]);
            let cell = format!("{:>7.3}", r);
            let colored = color_correlation(&cell, r);
            print!("   {}", colored);
        }
        println!();
    }
}

fn color_correlation(text: &str, r: f64) -> String {
    let abs_r = r.abs();
    if abs_r > 0.7 {
        if r > 0.0 {
            text.red().bold().to_string()
        } else {
            text.blue().bold().to_string()
        }
    } else if abs_r > 0.4 {
        if r > 0.0 {
            text.yellow().to_string()
        } else {
            text.cyan().to_string()
        }
    } else {
        text.dimmed().to_string()
    }
}

// ── Public entry point ─────────────────────────────────────────────

pub fn render_dist(
    rs_files: &[std::path::PathBuf],
    project_path: &Path,
    metric_filter: Option<&str>,
    num_bins: usize,
    verbose: bool,
) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    let fcs = complexity::compute_all(&symbols, project_path);

    if fcs.is_empty() {
        println!("{}", "No functions found to analyze.".yellow());
        return;
    }

    let metrics = extract_metrics(&fcs);

    println!(
        "\n{}",
        "── Distribution Analysis ──────────────────────────────"
            .bright_cyan()
            .bold()
    );

    if verbose {
        render::verbose_block(&[
            "Statistical distribution analysis of complexity metrics across all functions.",
            "Histograms show frequency distributions. Scatter plot shows LoC vs complexity.",
            "Outliers are functions with z-score > 2.0 (more than 2σ above the mean).",
            "Correlation matrix shows pairwise Pearson r between all metrics.",
        ]);
    }

    println!(
        "{} functions across {} files\n",
        format!("{}", fcs.len()).bold(),
        format!("{}", rs_files.len()).bold(),
    );

    // Histograms
    let labels = &[
        "Function LoC",
        "Cyclomatic",
        "Cognitive",
        "Nesting Depth",
        "Param Count",
    ];

    if let Some(filter) = metric_filter {
        // Find the matching metric
        let filter_lower = filter.to_lowercase();
        if let Some(idx) = METRIC_NAMES.iter().position(|&n| n == filter_lower) {
            render_histogram(&metrics.columns[idx], labels[idx], num_bins, verbose);
        } else {
            println!(
                "{} Unknown metric '{}'. Available: {}",
                "Error:".red().bold(),
                filter,
                METRIC_NAMES.join(", "),
            );
            return;
        }
    } else {
        for (i, label) in labels.iter().enumerate() {
            render_histogram(&metrics.columns[i], label, num_bins, verbose);
        }
    }

    // Scatter plot: complexity (composite score) vs LoC
    let composite_scores: Vec<f64> = fcs.iter().map(|f| f.composite_score).collect();
    render_scatter(
        &metrics.columns[0], // LoC
        &composite_scores,
        "Function LoC",
        "Complexity",
        verbose,
    );

    // Outlier detection
    render_outliers(&fcs, &metrics, verbose);

    // Correlation matrix
    render_correlation_matrix(&metrics, verbose);

    println!();
}

#[derive(Serialize)]
struct HistogramBinJson {
    low: f64,
    high: f64,
    count: usize,
}

#[derive(Serialize)]
struct HistogramJson {
    metric: String,
    bins: Vec<HistogramBinJson>,
    mean: f64,
    std_dev: f64,
    skewness: f64,
    kurtosis: f64,
}

#[derive(Serialize)]
struct OutlierJson {
    metric: String,
    function: String,
    z_score: f64,
    value: f64,
}

#[derive(Serialize)]
struct CorrelationEntryJson {
    metric_a: String,
    metric_b: String,
    pearson_r: f64,
}

#[derive(Serialize)]
struct DistJson {
    cstat_version: String,
    histograms: Vec<HistogramJson>,
    outliers: Vec<OutlierJson>,
    correlations: Vec<CorrelationEntryJson>,
}

fn compute_histogram_bins(values: &[f64], num_bins: usize) -> Vec<HistogramBinJson> {
    if values.is_empty() {
        return vec![];
    }

    let min_v = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_v = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max_v - min_v;
    let num_bins = if range.abs() < 1e-12 {
        1
    } else {
        let int_range = range.ceil() as usize;
        num_bins.min(int_range.max(1))
    };
    let bin_width = if num_bins == 1 {
        1.0
    } else {
        range / num_bins as f64
    };

    let mut counts = vec![0usize; num_bins];
    for &v in values {
        let idx = if num_bins == 1 {
            0
        } else {
            ((v - min_v) / bin_width).floor() as usize
        };
        let idx = idx.min(num_bins - 1);
        counts[idx] += 1;
    }

    (0..num_bins)
        .map(|i| {
            let lo = min_v + i as f64 * bin_width;
            let hi = lo + bin_width;
            HistogramBinJson {
                low: lo,
                high: hi,
                count: counts[i],
            }
        })
        .collect()
}

/// Render distribution analysis as JSON.
pub fn render_dist_json(
    rs_files: &[std::path::PathBuf],
    project_path: &std::path::Path,
    metric_filter: Option<&str>,
    num_bins: usize,
) {
    let symbols = crate::ast_parser::parse_project(rs_files);
    let fcs = complexity::compute_all(&symbols, project_path);
    let metrics = extract_metrics(&fcs);

    let labels = &["loc", "cyclomatic", "cognitive", "nesting", "params"];

    let indices: Vec<usize> = if let Some(filter) = metric_filter {
        let filter_lower = filter.to_lowercase();
        if let Some(idx) = METRIC_NAMES.iter().position(|&n| n == filter_lower) {
            vec![idx]
        } else {
            vec![]
        }
    } else {
        (0..METRIC_NAMES.len()).collect()
    };

    let histograms: Vec<HistogramJson> = indices
        .iter()
        .map(|&i| {
            let vals = &metrics.columns[i];
            let bins_json = compute_histogram_bins(vals, num_bins);
            HistogramJson {
                metric: labels[i].to_string(),
                bins: bins_json,
                mean: mean(vals),
                std_dev: std_dev(vals),
                skewness: skewness(vals),
                kurtosis: kurtosis(vals),
            }
        })
        .collect();

    let mut outliers_json: Vec<OutlierJson> = Vec::new();
    for (mi, metric_name) in METRIC_NAMES.iter().enumerate() {
        let vals = &metrics.columns[mi];
        let m = mean(vals);
        let s = std_dev(vals);
        if s < 1e-12 {
            continue;
        }
        for (i, &v) in vals.iter().enumerate() {
            let z = (v - m) / s;
            if z > 2.0 {
                outliers_json.push(OutlierJson {
                    metric: metric_name.to_string(),
                    function: fcs[i].name.clone(),
                    z_score: z,
                    value: v,
                });
            }
        }
    }
    outliers_json.sort_by(|a, b| {
        b.z_score
            .partial_cmp(&a.z_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let n = METRIC_NAMES.len();
    let mut correlations: Vec<CorrelationEntryJson> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let r = pearson(&metrics.columns[i], &metrics.columns[j]);
            correlations.push(CorrelationEntryJson {
                metric_a: METRIC_NAMES[i].to_string(),
                metric_b: METRIC_NAMES[j].to_string(),
                pearson_r: r,
            });
        }
    }

    let output = DistJson {
        cstat_version: env!("CARGO_PKG_VERSION").to_string(),
        histograms,
        outliers: outliers_json,
        correlations,
    };

    println!("{}", serde_json::to_string(&output).unwrap());
}
