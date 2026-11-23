use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Serialize;
use serde_json::{json, Value};
use walkdir::WalkDir;

mod hunks;

const EXTENSIONS: &[&str] = &[
    "cpp", "cc", "cxx", "C", "h", "hpp", "hh", "hxx", "ipp", "tpp",
];

const GIT_INVENTORY: &[&str] = &[
    "ls-files", "--cached", "--others", "--exclude-standard", "-t", "-z", "--",
];

pub(crate) fn display_path(path: &Path) -> Cow<'_, str> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let escape = |byte: u8| byte == b'%' || byte.is_ascii_control();
    if let Some(text) = path.to_str().filter(|text| !text.bytes().any(escape)) {
        return Cow::Borrowed(text);
    }

    // Escaping the marker as well keeps literal spellings distinct from raw bytes.
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut text = String::with_capacity(bytes.len());
    let push_byte = |text: &mut String, byte: u8| {
        text.push('%');
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 15)]));
    };
    for chunk in bytes.utf8_chunks() {
        // Copy whole valid UTF-8 runs, escaping only ASCII controls and '%'.
        let valid = chunk.valid();
        let mut start = 0;
        for (index, byte) in valid.bytes().enumerate().filter(|&(_, byte)| escape(byte)) {
            text.push_str(&valid[start..index]);
            push_byte(&mut text, byte);
            start = index + 1;
        }
        text.push_str(&valid[start..]);
        for &byte in chunk.invalid() {
            push_byte(&mut text, byte);
        }
    }
    Cow::Owned(text)
}

pub(crate) struct Selection {
    pub root: PathBuf,
    pub files: Vec<PathBuf>, // Relative to root, sorted, deduplicated by identity outside hunk mode.
    pub include_dirs: Vec<PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    mode: &'static str,
    base: Option<String>,
    excludes: Vec<String>,
    git_ignores: bool,
    git_inventory: Option<Vec<u8>>,
    identities: HashMap<FileIdentity, usize>,
    added_lines: Option<Vec<AddedLines>>, // Indexed in the same order as files.
}

#[derive(Clone, Copy, Serialize)]
pub(crate) struct LineRange {
    pub start: usize,
    pub end: usize,
}

pub(crate) enum AddedLines {
    All,
    Ranges(Vec<LineRange>),
}

pub(crate) struct Diagnostic {
    pub file: PathBuf,
    pub message: String,
}

impl Selection {
    pub(crate) fn is_file(&self) -> bool {
        self.mode == "file"
    }

    pub(crate) fn is_changed(&self) -> bool {
        self.base.is_some()
    }

    pub(crate) fn added_lines(&self, path: &Path) -> Option<&AddedLines> {
        let lines = self.added_lines.as_ref()?;
        let index = self.files.binary_search_by(|file| file.as_path().cmp(path)).ok()?;
        Some(&lines[index])
    }

    pub(crate) fn rules_json(&self) -> Value {
        json!({
            "root": display_path(&self.root),
            "mode": self.mode,
            "base": self.base,
            "extensions": EXTENSIONS,
            "git_ignores": self.git_ignores,
            "excludes": self.excludes,
            "include_dirs": self.include_dirs.iter().map(|p| display_path(p)).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn print_rules(&self) {
        println!("Root: {}", display_path(&self.root));
        print!("Selection: {}", self.mode);
        if let Some(base) = &self.base {
            print!(" (Git base: {base})");
        }
        println!();
        println!("Extensions: {}", EXTENSIONS.join(", "));
        println!(
            "Git ignores: {}",
            if self.git_ignores {
                "applied"
            } else {
                "outside a Git worktree"
            }
        );
        for pattern in &self.excludes {
            println!("Exclude: {pattern}");
        }
        for dir in &self.include_dirs {
            println!("Include root: {}", display_path(dir));
        }
    }
}

pub(crate) fn select(
    path: &Path,
    base: Option<&str>,
    excludes: &[String],
    include_dirs: &[PathBuf],
    diff_hunks: bool,
) -> Result<Selection, String> {
    if diff_hunks && base.is_none() {
        return Err("--diff-hunks requires --changed".into());
    }
    let resolve_error = |error| format!("Cannot resolve path '{}': {error}", display_path(path));
    let resolve = |location: &Path| fs::canonicalize(location).map_err(resolve_error);
    let meta = fs::metadata(path).map_err(resolve_error)?;
    let location = if meta.is_dir() {
        resolve(path)?
    } else if meta.is_file() && is_cpp_source(path) {
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
        resolve(parent.unwrap_or(Path::new(".")))?
    } else {
        return Err(format!(
            "--path must be a directory or a C++ source file: {}",
            display_path(path)
        ));
    };
    // Match an explicit file's requested name, not the target of a leaf
    // symlink. Discovery later validates the target and deduplicates by identity.
    let worktree = git_root(&location)?;
    if base.is_some() && worktree.is_none() {
        return Err("--changed requires a Git worktree".into());
    }
    let explicit = meta.is_file().then(|| {
        let parent = worktree.as_ref().map_or(Path::new(""), |root| {
            location.strip_prefix(root).expect("file is inside analysis root")
        });
        parent.join(path.file_name().unwrap())
    });
    let root = match (explicit.is_some(), worktree.as_ref()) {
        (true, Some(root)) => root.clone(),
        _ => location,
    };
    let explicit = explicit.as_deref();
    let patterns = Excludes::new(excludes)?;
    let mut resolved_dirs = Vec::with_capacity(include_dirs.len());
    for dir in include_dirs {
        let path = root.join(dir);
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Cannot inspect include directory '{}': {error}",
                display_path(&path)
            )
        })?;
        if !metadata.is_dir() {
            return Err(format!(
                "--include-dir is not a directory: {}",
                display_path(&path)
            ));
        }
        // Keep the spelling: a directory alias and its destination can have
        // different eligibility, and `..` must follow the filesystem traversal.
        resolved_dirs.push(path);
    }
    resolved_dirs.sort_unstable();
    resolved_dirs.dedup();

    let mut result = Selection {
        root,
        files: Vec::new(),
        include_dirs: resolved_dirs,
        diagnostics: Vec::new(),
        mode: if diff_hunks {
            "diff_hunks"
        } else if base.is_some() {
            "changed"
        } else if explicit.is_some() {
            "file"
        } else {
            "directory"
        },
        base: base.map(str::to_owned),
        excludes: excludes.to_vec(),
        git_ignores: worktree.is_some(),
        git_inventory: None,
        identities: HashMap::new(),
        added_lines: diff_hunks.then(Vec::new),
    };
    let mut candidates = Vec::new();
    let changed;
    let mut hunk_ranges = None;
    if let Some(worktree) = &worktree {
        let scope;
        let cpp_scopes;
        let scopes = if diff_hunks && explicit.is_none() {
            // Limit the textual patch at the source: unrelated binaries and
            // their Git clean filters must not affect a C++ score.
            cpp_scopes = EXTENSIONS.iter()
                .map(|ext| OsString::from(format!(":(glob)**/*.{ext}")))
                .collect::<Vec<_>>();
            cpp_scopes.as_slice()
        } else {
            scope = explicit.map_or_else(|| OsString::from("."), literal_pathspec);
            std::slice::from_ref(&scope)
        };
        changed = if let Some(base) = base {
            let commit = super::git_output(
                &result.root,
                &[
                    "rev-parse",
                    "--verify",
                    "--end-of-options",
                    &format!("{base}^{{commit}}"),
                ],
            )
            .map_err(|error| format!("Cannot resolve Git base '{base}': {error}"))?;
            let commit = String::from_utf8_lossy(&commit);
            // Git can report D for an inaccessible index entry. Keep every
            // status and let the filesystem distinguish absence from denial.
            let base = commit.trim_end();
            let names = ["diff", "--name-only", "--relative", "--no-renames", "-z", base, "--"];
            let patch = [
                "diff", "--patch-with-raw", "--relative", "--no-renames", "-z", "--unified=0",
                "--inter-hunk-context=0", "--diff-algorithm=myers", "--indent-heuristic",
                "--no-color", "--word-diff=none", "--no-ext-diff", "--no-textconv", "--text",
                "--src-prefix=a/", "--dst-prefix=b/", "--ignore-submodules=none", "--submodule=short",
                base, "--",
            ];
            Some(Selection::git_listing(
                &result.root,
                &mut result.diagnostics,
                if diff_hunks { &patch } else { &names },
                scopes,
                None,
            ))
        } else {
            None
        };
        let changed_names = if diff_hunks {
            hunk_ranges = Some(hunks::parse(changed.as_deref().unwrap_or_default()).unwrap_or_else(|error| {
                Selection::git_diagnostic(
                    &mut result.diagnostics,
                    format!("Malformed Git diff output: {error}"),
                );
                HashMap::new()
            }));
            None
        } else {
            changed.as_deref().map(|listing| {
                listing
                    .split(|&byte| byte == 0)
                    .filter_map(super::git_path)
                    .filter(|path| is_cpp_source(path))
                    .collect::<HashSet<_>>()
            })
        };
        let relative_root = result.root.strip_prefix(worktree).unwrap();
        let inventory = Selection::git_listing(
            &result.root,
            &mut result.diagnostics,
            GIT_INVENTORY,
            scopes,
            Some((relative_root, &patterns)),
        );
        // A path deleted from the index may still exist as an ignored untracked
        // file. Only the current inventory can qualify that path for selection.
        candidates.extend(
            git_entries(&inventory)
                .filter(|(untracked, path)| {
                    is_cpp_source(path)
                        && (*untracked || match &hunk_ranges {
                            Some(ranges) => ranges.contains_key(path),
                            None => changed_names.as_ref().is_none_or(|set| set.contains(path)),
                        })
                })
                .map(|(untracked, path)| (path.to_path_buf(), untracked)),
        );
        if explicit.is_none() {
            result.git_inventory = Some(inventory);
        }
    } else if let Some(file) = explicit {
        candidates.push((file.to_path_buf(), false));
    } else {
        for entry in WalkDir::new(&result.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                entry.depth() == 0
                    || !patterns.matches(
                        entry.path().strip_prefix(&result.root).unwrap(),
                        entry.file_type().is_dir(),
                    )
            })
        {
            match entry {
                Ok(entry) if is_cpp_source(entry.path()) && !entry.file_type().is_dir() => {
                    let path = entry.path().strip_prefix(&result.root).unwrap();
                    candidates.push((path.to_path_buf(), false));
                }
                Ok(_) => {}
                Err(error) => {
                    let file = error
                        .path()
                        .and_then(|p| p.strip_prefix(&result.root).ok())
                        .filter(|p| !p.as_os_str().is_empty());
                    result.diagnostics.push(Diagnostic {
                        file: file.unwrap_or(Path::new(".")).to_path_buf(),
                        message: error.to_string(),
                    });
                }
            }
        }
    }

    // If Git reports both statuses, prefer the untracked inventory entry.
    candidates.sort_unstable_by(|(a, a_untracked), (b, b_untracked)| a.cmp(b).then(b_untracked.cmp(a_untracked)));
    candidates.dedup_by(|a, b| a.0 == b.0);
    let mut safe_dirs = HashMap::new();
    let mut worktree_dirs = HashMap::new();
    for (file, untracked) in candidates {
        if (result.git_ignores || explicit.is_some()) && patterns.excludes_file(&file)
            || explicit.is_some_and(|only| only != file)
        {
            continue;
        }
        let identity = (|| {
            if result.git_ignores {
                let parent = file.parent().unwrap();
                if !ordinary_directory(&result.root, parent, &mut safe_dirs)?
                    || !same_worktree(&result.root, parent, &mut worktree_dirs)?
                {
                    return Ok(None);
                }
            }
            file_identity(
                &result.root,
                &file,
                &patterns,
                result.git_ignores.then_some(&mut worktree_dirs),
                diff_hunks,
            )
        })();
        match identity {
            Ok(Some(identity)) => {
                let index = result.files.len();
                let first = *result.identities.entry(identity).or_insert(index);
                if !diff_hunks && first != index {
                    continue;
                }
                if let Some(added) = &mut result.added_lines {
                    // A nonignored untracked path has no current tracked base,
                    // even when the same spelling was deleted from the index.
                    added.push(if untracked {
                        AddedLines::All
                    } else {
                        AddedLines::Ranges(
                            hunk_ranges.as_mut().unwrap().remove(file.as_path()).flatten().unwrap(),
                        )
                    });
                }
                // Each hardlink spelling has its own Git diff and ranges.
                result.files.push(file);
            }
            Err(error) => result.diagnostics.push(Diagnostic {
                file,
                message: error.to_string(),
            }),
            _ => {}
        }
    }
    result
        .diagnostics
        .sort_unstable_by(|a, b| (&a.file, &a.message).cmp(&(&b.file, &b.message)));
    Ok(result)
}

impl Selection {
    fn git_listing(
        root: &Path,
        diagnostics: &mut Vec<Diagnostic>,
        args: &[&str],
        scopes: &[OsString],
        excludes: Option<(&Path, &Excludes)>,
    ) -> Vec<u8> {
        let mut command = git(root);
        if args.contains(&"--patch-with-raw") {
            // Git gives this legacy environment variable precedence over -U.
            command.env_remove("GIT_DIFF_OPTS");
        }
        command.args(args).args(scopes);
        let mut output = command.output();
        if let (Some((root, rules)), Some(initial)) = (
            excludes,
            output.as_ref().ok().filter(|out| out.status.success() && !out.stderr.is_empty()),
        ) {
            let mut excluded_dirs = HashSet::new();
            let mut other_warnings = Vec::new();
            for line in git_warning_lines(&initial.stderr) {
                if let Some(dir) = excluded_git_warning_dir(line, root, rules) {
                    excluded_dirs.insert(dir);
                } else {
                    other_warnings.push(line);
                }
            }
            if !excluded_dirs.is_empty() {
                for warning in other_warnings {
                    Self::git_diagnostic(diagnostics, String::from_utf8_lossy(warning).into_owned());
                }
                // Let Git confirm the warning disappears when the directory is
                // pruned. A global exclude file can produce the same warning
                // as a per-directory .gitignore and must not be suppressed.
                command.args(excluded_dirs.into_iter().map(|dir| pathspec(":(top,exclude,literal)", dir)));
                output = command.output();
            }
        }
        match output {
            Ok(output) if output.status.success() => {
                for line in git_warning_lines(&output.stderr) {
                    Self::git_diagnostic(diagnostics, String::from_utf8_lossy(line).into_owned());
                }
                return output.stdout;
            }
            Ok(output) => Self::git_diagnostic(diagnostics, git_failure(args[0], &output)),
            Err(error) => Self::git_diagnostic(diagnostics, format!("Cannot run Git: {error}")),
        }
        Vec::new()
    }

    fn git_diagnostic(diagnostics: &mut Vec<Diagnostic>, message: String) {
        diagnostics.push(Diagnostic {
            file: PathBuf::from("."),
            message,
        });
    }
}

fn git_entries(listing: &[u8]) -> impl Iterator<Item = (bool, &Path)> {
    listing.split(|&byte| byte == 0).filter_map(|entry| match entry {
        [status, b' ', name @ ..] => super::git_path(name).map(|path| (*status == b'?', path)),
        _ => None,
    })
}

fn git_warning_lines(stderr: &[u8]) -> impl Iterator<Item = &[u8]> {
    stderr.split(|&byte| byte == b'\n').filter(|line| !line.is_empty())
}

fn literal_pathspec(path: &Path) -> OsString {
    pathspec(":(literal)", path)
}

fn pathspec(magic: &str, path: &Path) -> OsString {
    let mut spec = OsString::from(magic);
    spec.push(path);
    spec
}

fn excluded_git_warning_dir<'a>(line: &'a [u8], root: &Path, excludes: &Excludes) -> Option<&'a Path> {
    // These LC_ALL=C warnings name worktree-relative paths. Leave unknown or
    // ambiguous stderr alone rather than guessing what Git did not inspect.
    let line = line.strip_suffix(b"': Permission denied")?;
    let dir = if let Some(path) = line.strip_prefix(b"warning: could not open directory '") {
        super::git_path(path.strip_suffix(b"/")?)?
    } else {
        let path = super::git_path(line.strip_prefix(b"warning: unable to access '")?)?;
        if path.file_name()? != ".gitignore" {
            return None;
        }
        path.parent()?
    };
    if !dir.components().all(|part| matches!(part, Component::Normal(_))) {
        return None;
    }
    excludes.excludes_directory(dir.strip_prefix(root).ok()?).then_some(dir)
}

fn is_cpp_source(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| EXTENSIONS.contains(&ext))
}

struct Excludes {
    paths: GlobSet,
    trees: GlobSet,
}

impl Excludes {
    fn new(patterns: &[String]) -> Result<Self, String> {
        let mut paths = GlobSetBuilder::new();
        let mut trees = GlobSetBuilder::new();
        for pattern in patterns {
            let normalized = pattern
                .strip_prefix("./")
                .unwrap_or(pattern)
                .trim_end_matches('/');
            if normalized.is_empty()
                || Path::new(normalized).is_absolute()
                || normalized.split('/').any(|part| part == "..")
            {
                return Err(format!("--exclude glob must be root-relative: {pattern:?}"));
            }
            let compile = |value: &str| {
                GlobBuilder::new(value)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| format!("Invalid --exclude glob {pattern:?}: {error}"))
            };
            paths.add(compile(normalized)?);
            if let Some(tree) = normalized.strip_suffix("/**") {
                trees.add(compile(tree)?);
            }
        }
        Ok(Self {
            paths: paths.build().map_err(|error| error.to_string())?,
            trees: trees.build().map_err(|error| error.to_string())?,
        })
    }

    fn matches(&self, path: &Path, directory: bool) -> bool {
        self.paths.is_match(path) || (directory && self.trees.is_match(path))
    }

    fn excludes_file(&self, path: &Path) -> bool {
        self.matches(path, false)
            || path.parent().is_some_and(|parent| self.excludes_directory(parent))
    }

    fn excludes_directory(&self, path: &Path) -> bool {
        path.ancestors()
            .take_while(|dir| !dir.as_os_str().is_empty())
            .any(|dir| self.matches(dir, true))
    }
}

fn ordinary_directory(
    root: &Path,
    path: &Path,
    cache: &mut HashMap<PathBuf, bool>,
) -> io::Result<bool> {
    if path.as_os_str().is_empty() {
        return Ok(true);
    }
    if let Some(&allowed) = cache.get(path) {
        return Ok(allowed);
    }
    let allowed = ordinary_directory(root, path.parent().unwrap(), cache)?
        && optional_meta(fs::symlink_metadata(root.join(path)))?.is_some_and(|meta| meta.is_dir());
    cache.insert(path.to_path_buf(), allowed);
    Ok(allowed)
}

#[cfg(unix)]
pub(crate) type FileIdentity = (u64, u64);
#[cfg(not(unix))]
pub(crate) type FileIdentity = PathBuf;

fn file_identity(
    root: &Path,
    file: &Path,
    excludes: &Excludes,
    git_dirs: Option<&mut HashMap<PathBuf, bool>>,
    skip_symlinks: bool,
) -> io::Result<Option<FileIdentity>> {
    let mut path = root.join(file);
    let Some(mut meta) = optional_meta(fs::symlink_metadata(&path))? else {
        return Ok(None);
    };
    if meta.file_type().is_symlink() {
        if skip_symlinks {
            return Ok(None);
        }
        match fs::canonicalize(&path) {
            Ok(target) if target.starts_with(root) => path = target,
            Ok(_) => return Ok(None),
            Err(error) if absent(&error) => return Ok(None),
            Err(error) => return Err(error),
        }
        let target = path.strip_prefix(root).unwrap();
        if excludes.excludes_file(target) {
            return Ok(None);
        }
        if let Some(dirs) = git_dirs {
            if !same_worktree(root, target.parent().unwrap(), dirs)? {
                return Ok(None);
            }
        }
        let Some(target) = optional_meta(fs::metadata(&path))? else {
            return Ok(None);
        };
        meta = target;
    }
    if !meta.is_file() {
        return Ok(None);
    }
    Ok(Some(physical_identity(&path, &meta)))
}

#[cfg(unix)]
fn physical_identity(_path: &Path, meta: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn physical_identity(path: &Path, _meta: &Metadata) -> FileIdentity {
    path.to_path_buf()
}

pub(crate) struct IncludeHit {
    pub identity: FileIdentity,
    canonical: PathBuf,
}

pub(crate) fn include_hit(path: &Path) -> io::Result<Option<IncludeHit>> {
    let Some(meta) = optional_meta(fs::metadata(path))? else {
        return Ok(None);
    };
    if !meta.is_file() {
        return Ok(None);
    }
    // After a successful stat, even disappearance during canonicalization is
    // an inconsistent result rather than a confirmed miss.
    let canonical = fs::canonicalize(path)?;
    Ok(Some(IncludeHit {
        identity: physical_identity(&canonical, &meta),
        canonical,
    }))
}

pub(crate) struct IncludeUniverse<'a> {
    selection: &'a Selection,
    excludes: Excludes,
    git_paths: Option<HashSet<&'a Path>>,
    git_checks: HashMap<PathBuf, bool>,
    worktree_dirs: HashMap<PathBuf, bool>,
    ordinary_dirs: HashMap<PathBuf, bool>,
}

impl<'a> IncludeUniverse<'a> {
    pub(crate) fn new(selection: &'a Selection) -> Result<Self, String> {
        let git_paths = selection.git_inventory.as_deref()
            .map(|inventory| git_entries(inventory).map(|(_, path)| path).collect());
        Ok(Self {
            selection,
            excludes: Excludes::new(&selection.excludes)?,
            git_paths,
            git_checks: HashMap::new(),
            worktree_dirs: HashMap::new(),
            ordinary_dirs: HashMap::new(),
        })
    }

    pub(crate) fn eligible(
        &mut self,
        spelled: &Path,
        hit: &IncludeHit,
    ) -> io::Result<Option<PathBuf>> {
        let root = &self.selection.root;
        let Ok(canonical) = hit.canonical.strip_prefix(root) else {
            return Ok(None);
        };
        let Some(normalized) = self.include_spelling(spelled)? else {
            return Ok(None);
        };
        let relative = normalized.strip_prefix(root).ok()
            .filter(|path| !path.as_os_str().is_empty());
        if !is_cpp_source(spelled) || self.excludes.excludes_file(canonical)
            || relative.is_some_and(|path| self.excludes.excludes_file(path))
        {
            return Ok(None);
        }

        let candidate = match relative {
            None => canonical.to_path_buf(),
            Some(relative) => {
                let parent = relative.parent().unwrap();
                if ordinary_directory(root, parent, &mut self.ordinary_dirs)? {
                    relative.to_path_buf()
                } else {
                    if !self.allows_directory_alias(parent)? {
                        return Ok(None);
                    }
                    fs::canonicalize(spelled.parent().unwrap())?
                        .strip_prefix(root)
                        .map(|parent| parent.join(spelled.file_name().unwrap()))
                        .unwrap_or_else(|_| canonical.to_path_buf())
                }
            }
        };
        if !is_cpp_source(&candidate)
            || self.excludes.excludes_file(&candidate)
            || (self.selection.git_ignores
                && (!same_worktree(root, candidate.parent().unwrap(), &mut self.worktree_dirs)?
                    || !same_worktree(root, canonical.parent().unwrap(), &mut self.worktree_dirs)?
                    || !self.git_contains(&candidate)?))
        {
            return Ok(None);
        }
        Ok(Some(candidate))
    }

    fn include_spelling(&mut self, spelled: &Path) -> io::Result<Option<PathBuf>> {
        let root = &self.selection.root;
        let mut path = PathBuf::new();
        for part in spelled.components() {
            match part {
                Component::CurDir => {}
                Component::ParentDir => {
                    // Popping is valid only before encountering a symlink.
                    // Apply rules to the traversed alias, not to a path that
                    // a lexical collapse may invent under an unrelated tree.
                    let link = match path.strip_prefix(root) {
                        Ok(dir) if ordinary_directory(root, dir, &mut self.ordinary_dirs)? => false,
                        Ok(dir) if !self.allows_directory_alias(dir)? => return Ok(None),
                        _ => true,
                    };
                    if link {
                        path = fs::canonicalize(&path)?;
                    }
                    path.pop();
                }
                _ => path.push(part),
            }
        }
        Ok(Some(path))
    }

    fn allows_directory_alias(&mut self, parent: &Path) -> io::Result<bool> {
        let root = &self.selection.root;
        // The parent is known to traverse an alias. Git inventories the first
        // link itself, not the paths seen beneath it.
        for dir in parent.ancestors().take_while(|dir| !dir.as_os_str().is_empty()) {
            if ordinary_directory(root, dir.parent().unwrap(), &mut self.ordinary_dirs)? {
                if self.excludes.excludes_directory(dir) {
                    return Ok(false);
                }
                return if self.selection.git_ignores {
                    Ok(same_worktree(root, dir.parent().unwrap(), &mut self.worktree_dirs)?
                        && self.git_contains(dir)?)
                } else {
                    Ok(true)
                };
            }
        }
        Ok(true)
    }

    fn git_contains(&mut self, path: &Path) -> io::Result<bool> {
        if let Some(paths) = &self.git_paths {
            return Ok(paths.contains(path));
        }
        if let Some(&eligible) = self.git_checks.get(path) {
            return Ok(eligible);
        }
        // File-scoped discovery cannot answer for other includes. Use Git's
        // literal pathspec to query only the candidate or directory alias.
        let output = git(&self.selection.root)
            .args(GIT_INVENTORY)
            .arg(literal_pathspec(path))
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other(git_failure("ls-files", &output)));
        }
        if !output.stderr.is_empty() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        let eligible = git_entries(&output.stdout).any(|(_, candidate)| candidate == path);
        self.git_checks.insert(path.to_path_buf(), eligible);
        Ok(eligible)
    }

    pub(crate) fn selected_path(&self, identity: &FileIdentity) -> Option<&'a Path> {
        self.selection.identities.get(identity).map(|&i| self.selection.files[i].as_path())
    }
}

fn same_worktree(root: &Path, dir: &Path, cache: &mut HashMap<PathBuf, bool>) -> io::Result<bool> {
    if dir.as_os_str().is_empty() {
        return Ok(true);
    }
    if let Some(&allowed) = cache.get(dir) {
        return Ok(allowed);
    }
    let allowed = same_worktree(root, dir.parent().unwrap(), cache)?
        && optional_meta(fs::symlink_metadata(root.join(dir).join(".git")))?.is_none();
    cache.insert(dir.to_path_buf(), allowed);
    Ok(allowed)
}

fn absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

fn optional_meta(result: io::Result<Metadata>) -> io::Result<Option<Metadata>> {
    match result {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if absent(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn git(root: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C");
    command
}

fn git_failure(subcommand: &str, output: &Output) -> String {
    format!(
        "git {subcommand} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn git_root(location: &Path) -> Result<Option<PathBuf>, String> {
    match git(location)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let bytes = output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout);
            let root = super::git_path(bytes)
                .and_then(|path| path.canonicalize().ok())
                .filter(|root| location.starts_with(root))
                .ok_or_else(|| {
                    format!(
                        "Cannot resolve Git worktree root for '{}'",
                        display_path(location)
                    )
                })?;
            Ok(Some(root))
        }
        Ok(output) if String::from_utf8_lossy(&output.stderr).contains("not a git repository") => {
            Ok(None)
        }
        Ok(output) => Err(git_failure("rev-parse", &output)),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound
                && !location.ancestors().any(|dir| dir.join(".git").exists()) =>
        {
            Ok(None)
        }
        Err(error) => Err(format!("Cannot run Git: {error}")),
    }
}
