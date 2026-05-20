//! Single-pass project walk shared by every Group-A level.
//!
//! `cargo` build artefacts (`target/`), VCS state (`.git/` and any other
//! dotted directory), and editor scratch (e.g. `node_modules/`) are skipped.
//! `.rs` files are slurped once; everything else is recorded as size +
//! depth only. The same inventory is reused across all filesystem and
//! source-text levels so we touch each file at most once per run.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub size_bytes: u64,
    pub depth: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct DirEntry {
    pub depth: usize,
}

#[derive(Debug)]
pub(crate) struct RustFile {
    pub relative_path: PathBuf,
    pub bytes: Vec<u8>,
    /// `None` when the file is not valid UTF-8; callers must handle that
    /// case rather than panicking on `.unwrap()`.
    pub text: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct FileInventory {
    pub files: Vec<FileEntry>,
    pub directories: Vec<DirEntry>,
    pub rust_files: Vec<RustFile>,
}

impl FileInventory {
    pub fn scan(project_root: &Path) -> Self {
        let mut inv = FileInventory::default();
        let walker = WalkDir::new(project_root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(is_descendable);

        for entry in walker.flatten() {
            let depth = entry.depth();

            if entry.file_type().is_dir() {
                if depth == 0 {
                    continue; // skip the root itself in dir-count
                }
                inv.directories.push(DirEntry { depth });
                continue;
            }

            if !entry.file_type().is_file() {
                continue; // symlinks already filtered out by follow_links(false)
            }

            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            inv.files.push(FileEntry { size_bytes, depth });

            let relative = entry
                .path()
                .strip_prefix(project_root)
                .unwrap_or(entry.path())
                .to_path_buf();
            if relative.extension().is_some_and(|e| e == "rs") {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    let text = std::str::from_utf8(&bytes).ok().map(str::to_owned);
                    inv.rust_files.push(RustFile {
                        relative_path: relative,
                        bytes,
                        text,
                    });
                }
            }
        }

        inv
    }
}

fn is_descendable(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    if !entry.file_type().is_dir() {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    if name == "target" || name == "node_modules" {
        return false;
    }
    if name.starts_with('.') {
        return false; // .git, .cargo, .loop, etc.
    }
    true
}

/// Cheap shareable handle around a lazy inventory scan.
///
/// All filesystem / source-text levels hold a clone of the same handle, so
/// the very first one whose `extract` runs pays the scan cost; the rest
/// borrow the cached value. Cloning the handle is `Arc`-cheap and does not
/// duplicate the inventory.
#[derive(Clone, Default)]
pub(crate) struct SharedInventory(Arc<OnceLock<FileInventory>>);

impl SharedInventory {
    pub fn new() -> Self {
        Self(Arc::new(OnceLock::new()))
    }

    pub fn get(&self, project_root: &Path) -> &FileInventory {
        self.0.get_or_init(|| FileInventory::scan(project_root))
    }
}
