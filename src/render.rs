use colored::Colorize;
use std::path::Path;

// ── Module name extraction ──────────────────────────────────────────

/// Extract module name from a file path relative to the project.
/// e.g. `src/foo/bar.rs` → `foo::bar`, `src/baz/mod.rs` → `baz`
pub fn module_name_from_path(path: &Path, project_path: &Path) -> String {
    let rel = path.strip_prefix(project_path).unwrap_or(path);
    let s = rel.to_string_lossy().to_string();
    let s = s.strip_suffix(".rs").unwrap_or(&s);
    let s = s.replace('/', "::");
    let s = s.strip_prefix("src::").unwrap_or(s.as_str());
    if s.ends_with("::mod") {
        s.strip_suffix("::mod").unwrap_or(s).to_string()
    } else {
        s.to_string()
    }
}

// ── Terminal width ──────────────────────────────────────────────────

/// Get terminal width, default to 80 if unavailable.
pub fn terminal_width() -> usize {
    if let Some((w, _)) = term_size() {
        w
    } else {
        80
    }
}

fn term_size() -> Option<(usize, usize)> {
    #[cfg(unix)]
    {
        use std::mem::zeroed;
        unsafe {
            let mut ws: libc_winsize = zeroed();
            if libc_ioctl(1, TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return Some((ws.ws_col as usize, ws.ws_row as usize));
            }
        }
    }
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(w) = cols.parse::<usize>() {
            if w > 0 {
                return Some((w, 24));
            }
        }
    }
    None
}

#[cfg(unix)]
const TIOCGWINSZ: libc_ulong = 0x5413;

#[cfg(unix)]
#[allow(non_camel_case_types)]
type libc_ulong = std::os::raw::c_ulong;

#[cfg(unix)]
#[repr(C)]
struct libc_winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

#[cfg(unix)]
extern "C" {
    fn ioctl(fd: i32, request: libc_ulong, ...) -> i32;
}

#[cfg(unix)]
unsafe fn libc_ioctl(fd: i32, request: libc_ulong, ws: &mut libc_winsize) -> i32 {
    unsafe { ioctl(fd, request, ws as *mut libc_winsize) }
}

// ── ANSI-aware string utilities ─────────────────────────────────────

/// Get the visible character length (ignoring ANSI escape codes).
pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if ch.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

/// Strip ANSI escape sequences, returning plain text.
#[allow(dead_code)]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\x1b' {
            in_escape = true;
        } else if in_escape && ch == 'm' {
            in_escape = false;
        } else if !in_escape {
            out.push(ch);
        }
    }
    out
}

// ── Color helpers ───────────────────────────────────────────────────

/// Color text by rank ratio: 0.0 = top (red), 1.0 = bottom (green).
pub fn bar_color(text: &str, ratio: f64) -> String {
    if ratio < 0.33 {
        text.red().bold().to_string()
    } else if ratio < 0.66 {
        text.yellow().to_string()
    } else {
        text.green().to_string()
    }
}

// ── Stat row helpers ────────────────────────────────────────────────

/// Print a stat row in a `│ label │ value │` table with 11-char label, 12-char value.
pub fn print_stat_row(label: &str, value: &str) {
    println!("│ {:<11} │ {:>12} │", label.cyan(), value.bold());
}

/// Print a verbose explanation line with dimmed cyan ℹ prefix.
pub fn verbose_line(text: &str) {
    println!("  {}", format!("ℹ {}", text).dimmed());
}

/// Print a block of verbose explanation lines followed by a blank line.
pub fn verbose_block(lines: &[&str]) {
    for line in lines {
        verbose_line(line);
    }
    println!();
}

/// Print a guide cross-reference hint.
pub fn guide_ref(topic: &str) {
    use colored::Colorize;
    println!("  {}", format!("ℹ See: cstat guide {}", topic).dimmed());
    println!();
}

/// Print a section header with a decorative line.
pub fn section_header(title: &str) {
    use colored::Colorize;
    println!(
        "\n┌─ {} {}",
        title.bright_cyan().bold(),
        "─"
            .repeat(50usize.saturating_sub(title.len() + 3))
            .bright_cyan(),
    );
}
