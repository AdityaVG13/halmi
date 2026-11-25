//! Deterministic source discovery: walk, prune, glob exclusions.

use std::path::{Path, PathBuf};

pub fn glob_match(pattern: &str, path: &str) -> bool {
    fn inner(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        match p[0] {
            b'*' => {
                let rest = &p[1..];
                (0..=s.len()).any(|i| inner(rest, &s[i..]))
            }
            b'?' => !s.is_empty() && inner(&p[1..], &s[1..]),
            c => !s.is_empty() && s[0] == c && inner(&p[1..], &s[1..]),
        }
    }
    inner(pattern.as_bytes(), path.as_bytes())
}

const SOURCE_EXTENSIONS: [&str; 31] = [
    "rs", "py", "go", "swift", "zig", "c", "h", "cpp", "cc", "cxx", "hpp", "hxx", "hh", "ipp",
    "tcc", "inl", "java", "cs", "kt", "kts", "php", "rb", "lua", "js", "jsx", "ts", "tsx", "mjs",
    "cjs", "mts", "cts",
];
const SKIPPED_DIRS: [&str; 5] = ["target", "node_modules", "venv", ".venv", "site-packages"];

fn has_source_extension(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => SOURCE_EXTENSIONS.contains(&ext),
        None => false,
    }
}

pub fn collect_source_files(paths: &[String], excludes: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            walk_dir(path, &mut out);
        } else if has_source_extension(&path.to_string_lossy()) {
            out.push(path.to_path_buf());
        } else {
            eprintln!("halmi: skipping {p} (not a source file or directory)");
        }
    }
    out.sort_by_key(|f| display_path(f));
    out.dedup();
    if !excludes.is_empty() {
        out.retain(|f| {
            let display = display_path(f);
            !excludes.iter().any(|pat| glob_match(pat, &display))
        });
    }
    out
}

pub fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("halmi: cannot read {}", dir.display());
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk_dir(&path, out);
        } else if has_source_extension(&name) && !name.starts_with('.') {
            out.push(path);
        }
    }
}

/// Report path string: `/`-separated on every platform so JSON, SARIF URIs,
/// exclude globs, and baselines match across macOS, Linux, and Windows.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::display_path;
    use std::path::Path;

    #[test]
    fn display_path_uses_forward_slashes() {
        assert_eq!(
            display_path(Path::new(".\\crates\\halmi-cli\\src\\walk.rs")),
            "./crates/halmi-cli/src/walk.rs"
        );
        assert_eq!(
            display_path(Path::new("crates/halmi-core/src/lib.rs")),
            "crates/halmi-core/src/lib.rs"
        );
    }
}
