//! Reading and analyzing files: the parallel pipeline entry points.

use std::path::{Path, PathBuf};

use halmi_core::adapter::{LanguageAdapter, SourceAnalysis};
use halmi_core::report::{FunctionReport, parse_reports};

use halmi_c::CAdapter;
use halmi_cpp::CppAdapter;
use halmi_cs::CsAdapter;
use halmi_go::GoAdapter;
use halmi_java::JavaAdapter;
use halmi_kotlin::KotlinAdapter;
use halmi_lua::LuaAdapter;
use halmi_php::PhpAdapter;
use halmi_python::PythonAdapter;
use halmi_rs::RustAdapter;
use halmi_ruby::RubyAdapter;
use halmi_script::ScriptAdapter;
use halmi_swift::SwiftAdapter;
use halmi_zig::ZigAdapter;

fn adapter_for(path: &Path) -> &'static dyn LanguageAdapter {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py") => &PythonAdapter,
        Some("go") => &GoAdapter,
        Some("swift") => &SwiftAdapter,
        Some("zig") => &ZigAdapter,
        Some("c" | "h") => &CAdapter,
        Some("cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" | "ipp" | "tcc" | "inl") => &CppAdapter,
        Some("java") => &JavaAdapter,
        Some("cs") => &CsAdapter,
        Some("kt" | "kts") => &KotlinAdapter,
        Some("php") => &PhpAdapter,
        Some("rb") => &RubyAdapter,
        Some("lua") => &LuaAdapter,
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts") => &ScriptAdapter,
        _ => &RustAdapter,
    }
}

/// Read one file and run the adapter its extension routes to. IO errors
/// degrade to `error` entries like lex errors do, so a whole-tree scan
/// keeps going.
pub fn read_and_analyze(path: &Path) -> SourceAnalysis {
    let display = crate::walk::display_path(path);
    match std::fs::read_to_string(path) {
        Ok(src) => adapter_for(path).analyze(&display, &src),
        Err(e) => SourceAnalysis {
            path: display,
            functions: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

/// Analyze files in parallel with a bounded std-thread pool. Chunking keeps
/// the merge deterministic: results are concatenated in file order.
pub fn analyze_files(files: &[PathBuf]) -> Vec<SourceAnalysis> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(files.len());
    if workers <= 1 {
        return files.iter().map(|p| read_and_analyze(p)).collect();
    }
    let chunk_size = files.len().div_ceil(workers).max(1);
    std::thread::scope(|s| {
        #[expect(
            clippy::needless_collect,
            reason = "every chunk must be spawned before the first join; a lazy chain would run the chunks one after another"
        )]
        let handles: Vec<_> = files
            .chunks(chunk_size)
            .map(|chunk| {
                s.spawn(move || {
                    chunk
                        .iter()
                        .map(|p| read_and_analyze(p))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("analysis thread panicked"))
            .collect()
    })
}

pub fn load_baseline(path: &Path) -> Result<Vec<FunctionReport>, String> {
    if path.is_dir() {
        return Err(format!("baseline path is a directory: {}", path.display()));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read baseline {}: {e}", path.display()))?;
    parse_reports(&text).map_err(|e| format!("baseline is not valid halmi JSON: {e}"))
}
