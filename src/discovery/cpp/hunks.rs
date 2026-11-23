use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Component, Path};

use super::LineRange;

// With --patch-with-raw -z Git emits the raw, NUL-delimited records first,
// followed by one more NUL and the ordinary line-delimited patch. The raw
// records establish the exact path spellings and include mode-only changes.
pub(super) fn parse(output: &[u8]) -> Result<HashMap<&Path, Option<Vec<LineRange>>>, &'static str> {
    let mut paths = HashMap::new();
    if output.is_empty() {
        return Ok(paths);
    }
    let mut rest = output;
    while rest.starts_with(b":") {
        let header = take_field(&mut rest).ok_or("unterminated raw record")?;
        if !raw_header_valid(&header[1..]) {
            return Err("invalid raw record header");
        }
        let name = take_field(&mut rest).ok_or("unterminated raw path")?;
        let path = super::super::git_path(name)
            .filter(|path| !name.is_empty() && path.components().all(|p| matches!(p, Component::Normal(_))))
            .ok_or("invalid raw path")?;
        if paths.insert(path, None).is_some() {
            return Err("duplicate raw path");
        }
    }
    let patch = rest.strip_prefix(b"\0").ok_or("missing raw/patch separator")?;
    if paths.is_empty() {
        return Err("patch has no raw records");
    }
    let mut lines = patch.split_inclusive(|&byte| byte == b'\n').peekable();
    while let Some(line) = lines.next() {
        let header = line
            .strip_suffix(b"\n")
            .and_then(|line| line.strip_prefix(b"diff --git "))
            .ok_or("expected patch section header")?;
        let name = patch_name(header).ok_or("invalid patch path")?;
        let path = super::super::git_path(&name).ok_or("invalid patch path")?;
        let ranges = paths
            .get_mut(path)
            .ok_or("patch path not in raw records")?
            .get_or_insert_with(Vec::new);
        let mut remaining = (0, 0);
        let mut saw_hunk = false;
        while let Some(line) = lines.next_if(|line| !line.starts_with(b"diff --git ")) {
            let line = line.strip_suffix(b"\n").ok_or("unterminated patch line")?;
            if saw_hunk && line == b"\\ No newline at end of file" {
                continue;
            }
            if remaining != (0, 0) {
                match line.first() {
                    Some(b'-') if remaining.0 > 0 => remaining.0 -= 1,
                    Some(b'+') if remaining.1 > 0 => remaining.1 -= 1,
                    _ => return Err("invalid zero-context hunk body"),
                }
            } else if line.starts_with(b"@@") {
                let (old_len, new_len, range) = parse_hunk(line).ok_or("invalid hunk header")?;
                if let Some(range) = range {
                    if ranges.last().is_some_and(|last: &LineRange| last.end >= range.start) {
                        return Err("overlapping or unordered new-side ranges");
                    }
                    ranges.push(range);
                }
                remaining = (old_len, new_len);
                saw_hunk = true;
            } else if saw_hunk || !is_patch_metadata(line) {
                return Err("unexpected patch line");
            }
        }
        if remaining != (0, 0) {
            return Err("truncated hunk body");
        }
    }
    if paths.values().any(Option::is_none) {
        return Err("raw path has no patch section");
    }
    Ok(paths)
}

fn take_field<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    let end = rest.iter().position(|&byte| byte == 0)?;
    let (field, tail) = rest.split_at(end);
    *rest = &tail[1..];
    Some(field)
}

fn raw_header_valid(header: &[u8]) -> bool {
    let mut fields = header.split(|&byte| byte == b' ');
    let mode = |field: &[u8]| field.len() == 6 && field.iter().all(|byte| matches!(byte, b'0'..=b'7'));
    let object = |field: &[u8]| !field.is_empty() && field.iter().all(u8::is_ascii_hexdigit);
    fields.next().is_some_and(mode)
        && fields.next().is_some_and(mode)
        && fields.next().is_some_and(object)
        && fields.next().is_some_and(object)
        && matches!(fields.next(), Some(b"A" | b"D" | b"M" | b"T" | b"U"))
        && fields.next().is_none()
}

fn is_patch_metadata(line: &[u8]) -> bool {
    [b"old mode ".as_slice(), b"new mode ", b"deleted file mode ", b"new file mode ", b"index ", b"--- ", b"+++ "]
        .iter().any(|&prefix| line.starts_with(prefix))
}

fn patch_name(header: &[u8]) -> Option<Cow<'_, [u8]>> {
    // Both sides use the same path and equal-length prefixes (renames and
    // copies are off). Even an unquoted name containing " b/" is unambiguous.
    let (old, new) = header.split_at(header.len() / 2);
    let new = new.strip_prefix(b" ")?;
    if let Some(old) = old.strip_prefix(b"a/") {
        let new = new.strip_prefix(b"b/")?;
        return (old == new).then_some(Cow::Borrowed(new));
    }
    let old = old.strip_prefix(b"\"a/")?.strip_suffix(b"\"")?;
    let new = new.strip_prefix(b"\"b/")?.strip_suffix(b"\"")?;
    if old != new {
        return None;
    }
    unquote(new).map(Cow::Owned)
}

fn unquote(mut input: &[u8]) -> Option<Vec<u8>> {
    let mut name = Vec::with_capacity(input.len());
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        if byte != b'\\' {
            name.push(byte);
            continue;
        }
        let (&escape, rest) = input.split_first()?;
        input = rest;
        name.push(match escape {
            b'\\' | b'"' => escape,
            b'a' => 7,
            b'b' => 8,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 11,
            b'f' => 12,
            b'r' => b'\r',
            b'0'..=b'3' => {
                let [second @ b'0'..=b'7', third @ b'0'..=b'7', rest @ ..] = input else {
                    return None;
                };
                input = rest;
                (escape - b'0') * 64 + (second - b'0') * 8 + (third - b'0')
            }
            _ => return None,
        });
    }
    Some(name)
}

fn parse_hunk(header: &[u8]) -> Option<(usize, usize, Option<LineRange>)> {
    let mut fields = header.strip_prefix(b"@@ -")?.splitn(3, |&byte| byte == b' ');
    let (old_len, _) = parse_span(fields.next()?)?;
    let (new_len, range) = parse_span(fields.next()?.strip_prefix(b"+")?)?;
    let trailer = fields.next()?;
    (trailer == b"@@" || trailer.starts_with(b"@@ "))
        .then_some((old_len, new_len, range))
        .filter(|_| old_len != 0 || new_len != 0)
}

fn parse_span(span: &[u8]) -> Option<(usize, Option<LineRange>)> {
    let mut fields = span.splitn(2, |&byte| byte == b',');
    let start = decimal(fields.next()?)?;
    let len = match fields.next() {
        Some(bytes) => decimal(bytes)?,
        None => 1,
    };
    let range = if len == 0 {
        None
    } else {
        if start == 0 {
            return None;
        }
        Some(LineRange { start, end: start.checked_add(len - 1)? })
    };
    Some((len, range))
}

fn decimal(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0_usize, |value, &byte| {
        let digit = byte.checked_sub(b'0').filter(|&digit| digit < 10)?;
        value.checked_mul(10)?.checked_add(usize::from(digit))
    })
}
