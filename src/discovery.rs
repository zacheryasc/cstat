use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Discover all `.rs` source files in a Rust project.
///
/// Given a project root directory, this function:
/// 1. Verifies a `Cargo.toml` exists at the root.
/// 2. Checks for a `[workspace]` section to find workspace members.
/// 3. Walks `src/` directories of the root crate and any workspace members.
/// 4. Collects all `.rs` files found.
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
        // Single crate — walk `src/` from the root
        let src_dir = project_path.join("src");
        if src_dir.is_dir() {
            source_dirs.push(src_dir);
        }
    } else {
        // Workspace — walk each member's `src/`
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
                                    let src = p.join("src");
                                    if src.is_dir() {
                                        source_dirs.push(src);
                                    }
                                }
                            }
                        }
                    }
                }
            } else if member_path.is_dir() {
                let src = member_path.join("src");
                if src.is_dir() {
                    source_dirs.push(src);
                }
            }
        }
        // Also include root src/ if it exists (some workspaces have a root crate)
        let root_src = project_path.join("src");
        if root_src.is_dir() {
            source_dirs.push(root_src);
        }
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
