use std::path::{Path, PathBuf};

#[path = "dist.rs"]
pub(crate) mod dist;
#[path = "dump.rs"]
pub(crate) mod dump;
#[path = "guide.rs"]
pub(crate) mod guide;
#[path = "map.rs"]
pub(crate) mod map;
#[path = "summary/mod.rs"]
pub(crate) mod summary;

use crate::{circuits, complexity, datapaths, deps, flow, graph, loc, redundancy};

pub enum Command {
    Summary,
    Loc { top: Option<usize> },
    Branching,
    Signature,
    Span,
    Deps { top: Option<usize> },
    Map,
    Flow,
    Graph { call_only: bool, dep_only: bool, top: usize },
    Datapaths { walks: usize, from: Option<String> },
    Dist { metric: Option<String>, bins: usize },
    Circuits { min_size: usize },
    Redundancy,
    Dump,
    Guide { topic: Option<String> },
}

fn canon(project_path: &Path) -> PathBuf {
    project_path
        .canonicalize()
        .unwrap_or_else(|_| project_path.to_path_buf())
}

/// Run one of two closures depending on the `json` flag.
fn run_json_or_human<F1, F2>(json: bool, json_fn: F1, human_fn: F2)
where
    F1: FnOnce(),
    F2: FnOnce(),
{
    if json {
        json_fn();
    } else {
        human_fn();
    }
}

pub fn dispatch(command: Command, rs_files: &[PathBuf], project_path: &Path, json: bool, verbose: bool) {
    let p = canon(project_path);
    match command {
        Command::Summary => run_json_or_human(json,
            || summary::render_summary_json(rs_files, &p),
            || summary::render_summary(rs_files, &p, verbose),
        ),
        Command::Loc { top } => run_json_or_human(json,
            || loc::render_loc_json(rs_files, &p),
            || loc::render_loc(rs_files, &p, top, verbose),
        ),
        Command::Branching => complexity::run_branching(rs_files, &p, json, verbose),
        Command::Signature => complexity::run_signature(rs_files, &p, json, verbose),
        Command::Span => complexity::run_span(rs_files, &p, json, verbose),
        Command::Deps { top } => run_json_or_human(json,
            || deps::render_deps_json(rs_files, &p),
            || deps::render_deps(rs_files, &p, top, verbose),
        ),
        Command::Map => run_json_or_human(json,
            || map::render_map_json(rs_files, &p),
            || map::render_map(rs_files, &p, verbose),
        ),
        Command::Flow => run_json_or_human(json,
            || flow::render_flow_json(rs_files, &p),
            || flow::render_flow(rs_files, &p, verbose),
        ),
        Command::Graph { call_only, dep_only, top } => run_json_or_human(json,
            || graph::render_graph_json(rs_files, &p, call_only, dep_only),
            || graph::render_graph(rs_files, &p, call_only, dep_only, top, verbose),
        ),
        Command::Datapaths { walks, from } => run_json_or_human(json,
            || datapaths::render_datapaths_json(rs_files, &p, walks, from.as_deref()),
            || datapaths::render_datapaths(rs_files, &p, walks, from.as_deref(), verbose),
        ),
        Command::Dist { metric, bins } => run_json_or_human(json,
            || dist::render_dist_json(rs_files, &p, metric.as_deref(), bins),
            || dist::render_dist(rs_files, &p, metric.as_deref(), bins, verbose),
        ),
        Command::Circuits { min_size } => run_json_or_human(json,
            || circuits::render_circuits_json(rs_files, &p, min_size),
            || circuits::render_circuits(rs_files, &p, min_size, verbose),
        ),
        Command::Redundancy => run_json_or_human(json,
            || redundancy::render_redundancy_json(rs_files, &p),
            || redundancy::render_redundancy(rs_files, &p, verbose),
        ),
        Command::Dump => {
            colored::control::set_override(false);
            dump::render_dump(rs_files, &p);
        }
        Command::Guide { topic } => match topic {
            Some(t) => guide::render_guide_topic(&t, json),
            None => guide::render_guide_toc(json),
        },
    }
}
