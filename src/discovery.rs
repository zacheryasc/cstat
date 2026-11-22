use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub enum TargetKind {
    Project,
    File { file_path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct AnalysisTarget {
    pub project_path: PathBuf,
    pub kind: TargetKind,
    pub project_rs_files: Vec<PathBuf>,
    pub target_rs_files: Vec<PathBuf>,
}

pub fn selected_file(target: &AnalysisTarget) -> Option<&Path> {
    match &target.kind {
        TargetKind::Project => None,
        TargetKind::File { file_path } => Some(file_path.as_path()),
    }
}

pub fn files_for_project(target: &AnalysisTarget) -> &[PathBuf] {
    &target.project_rs_files
}

pub fn files_for_target(target: &AnalysisTarget) -> &[PathBuf] {
    &target.target_rs_files
}

pub fn resolve_target(path: &Path) -> Result<AnalysisTarget, String> {
    let canonical_path = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{}': {}", path.display(), e))?;

    if canonical_path.is_dir() {
        let rs_files = discover_rs_files(&canonical_path)?;
        return Ok(AnalysisTarget {
            project_path: canonical_path,
            kind: TargetKind::Project,
            project_rs_files: rs_files.clone(),
            target_rs_files: rs_files,
        });
    }

    if canonical_path.is_file() {
        let project_path = crate_for_source(&canonical_path)?;

        let project_rs_files = discover_rs_files(&project_path)?;
        if !project_rs_files.contains(&canonical_path) {
            return Err(format!(
                "file is not under this crate's src, tests, or benches: {}",
                canonical_path.display()
            ));
        }

        return Ok(AnalysisTarget {
            project_path,
            kind: TargetKind::File {
                file_path: canonical_path.clone(),
            },
            project_rs_files,
            target_rs_files: vec![canonical_path],
        });
    }

    Err(format!(
        "Cannot resolve path '{}': not a file or directory",
        path.display()
    ))
}

fn crate_for_source(path: &Path) -> Result<PathBuf, String> {
    if path.extension().is_none_or(|ext| ext != "rs") {
        return Err(format!(
            "--path file must be a Rust source file: {}",
            path.display()
        ));
    }
    path.ancestors()
        .skip(1)
        .find(|dir| dir.join("Cargo.toml").exists())
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("No Cargo.toml found above '{}'", path.display()))
}

pub(crate) fn resolve_changed_target(
    path: &Path,
    base: &str,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{}': {}", path.display(), e))?;
    let (project_path, selected_file) = if path.is_dir() {
        if !path.join("Cargo.toml").exists() {
            return Err(format!("No Cargo.toml found at '{}'", path.display()));
        }
        (path, None)
    } else if path.is_file() {
        (crate_for_source(&path)?, Some(path))
    } else {
        return Err(format!(
            "Cannot resolve path '{}': not a file or directory",
            path.display()
        ));
    };
    let commit = git_output(
        &project_path,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )
    .map_err(|error| format!("Cannot resolve Git base '{base}': {error}"))?;
    let commit = String::from_utf8_lossy(&commit);
    let tracked = git_output(
        &project_path,
        &[
            "diff",
            "--name-only",
            "--relative",
            "--no-renames",
            "--diff-filter=d",
            "-z",
            commit.trim_end(),
            "--",
            "*.rs",
        ],
    )?;
    let untracked = git_output(
        &project_path,
        &["ls-files", "--others", "--exclude-standard", "-z", "--", "*.rs"],
    )?;
    let selected_relative = selected_file
        .as_deref()
        .map(|file| file.strip_prefix(&project_path).unwrap());
    let mut files: Vec<_> = [&tracked, &untracked]
        .into_iter()
        .flat_map(|output| output.split(|&byte| byte == 0))
        .filter(|path| !path.is_empty())
        .filter_map(git_path)
        .filter(|path| selected_relative.is_none_or(|selected| *path == selected))
        .map(|path| project_path.join(path))
        .filter(|path| path.is_file())
        .collect();
    files.sort_unstable();
    files.dedup();
    Ok((project_path, files))
}

fn git_output(project_path: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .current_dir(project_path)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .map_err(|error| format!("Cannot run Git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("git {} failed: {}", args[0], stderr.trim()))
    }
}

fn git_path(bytes: &[u8]) -> Option<&Path> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Some(Path::new(std::ffi::OsStr::from_bytes(bytes)))
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(Path::new)
    }
}

/// Discover all `.rs` source files in a Rust project.
///
/// Given a project root directory, this function:
/// 1. Verifies a `Cargo.toml` exists at the root.
/// 2. Checks for a `[workspace]` section to find workspace members.
/// 3. Walks `src/`, `tests/`, and `benches/` directories of the root crate
///    and any workspace members.
pub fn discover_rs_files(project_path: &Path) -> Result<Vec<PathBuf>, String> {
    let project_path = project_path
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path '{}': {}", project_path.display(), e))?;

    let cargo_toml = project_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Err(format!(
            "No Cargo.toml found at '{}'",
            project_path.display()
        ));
    }

    let mut source_dirs: Vec<PathBuf> = Vec::new();

    // Check for workspace members
    let cargo_content = std::fs::read_to_string(&cargo_toml)
        .map_err(|e| format!("Cannot read Cargo.toml: {}", e))?;

    let workspace_members = parse_workspace_members(&cargo_content);
    if workspace_members.is_empty() {
        // Single crate — walk Rust source, test, and benchmark roots.
        push_standard_rust_dirs(&project_path, &mut source_dirs);
    } else {
        // Workspace — walk each member's Rust source, test, and benchmark roots.
        for member in &workspace_members {
            // Handle glob patterns (e.g., "crates/*")
            let member_path = project_path.join(member);
            if member.contains('*') {
                if let Some(parent) = member_path.parent() {
                    if parent.is_dir() {
                        for entry in std::fs::read_dir(parent).into_iter().flatten() {
                            if let Ok(entry) = entry {
                                let p = entry.path();
                                if p.is_dir() && p.join("Cargo.toml").exists() {
                                    push_standard_rust_dirs(&p, &mut source_dirs);
                                }
                            }
                        }
                    }
                }
            } else if member_path.is_dir() {
                push_standard_rust_dirs(&member_path, &mut source_dirs);
            }
        }
        // Also include root dirs if they exist (some workspaces have a root crate).
        push_standard_rust_dirs(&project_path, &mut source_dirs);
    }

    let mut rs_files: Vec<PathBuf> = Vec::new();
    for dir in &source_dirs {
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "rs") {
                rs_files.push(path.to_path_buf());
            }
        }
    }

    rs_files.sort();
    Ok(rs_files)
}

fn push_standard_rust_dirs(crate_path: &Path, source_dirs: &mut Vec<PathBuf>) {
    for dirname in ["src", "tests", "benches"] {
        let dir = crate_path.join(dirname);
        if dir.is_dir() && !source_dirs.contains(&dir) {
            source_dirs.push(dir);
        }
    }
}

/// Simple parser for workspace members from Cargo.toml content.
/// Looks for `[workspace]` section and `members = [...]`.
fn parse_workspace_members(content: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut in_workspace = false;
    let mut in_members = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == "[workspace]" {
            in_workspace = true;
            continue;
        }

        if in_workspace && trimmed.starts_with('[') && trimmed != "[workspace]" {
            in_workspace = false;
            in_members = false;
            continue;
        }

        if in_workspace {
            if trimmed.starts_with("members") && trimmed.contains('=') {
                in_members = true;
                // Parse inline array if on same line
                if let Some(start) = trimmed.find('[') {
                    let rest = &trimmed[start..];
                    members.extend(parse_string_array(rest));
                    if rest.contains(']') {
                        in_members = false;
                    }
                }
            } else if in_members {
                members.extend(parse_string_array(trimmed));
                if trimmed.contains(']') {
                    in_members = false;
                }
            }
        }
    }

    members
}

/// Extract quoted strings from a fragment like `["foo", "bar/*"]` or `"baz",`
fn parse_string_array(s: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut in_quote = false;
    let mut current = String::new();

    for ch in s.chars() {
        if ch == '"' {
            if in_quote {
                results.push(current.clone());
                current.clear();
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            current.push(ch);
        }
    }

    results
}
