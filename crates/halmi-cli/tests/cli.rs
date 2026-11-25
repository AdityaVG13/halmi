//! End-to-end tests running the halmi binary on fixture trees.

use std::process::Command;

use serde_json::Value;

fn halmi() -> Command {
    Command::new(env!("CARGO_BIN_EXE_halmi"))
}

fn fixture(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        r"/// Adds things.
pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

/// A busy function that should rank high.
pub fn busy(a: u32, b: u32, c: u32, d: u32) -> u32 {
    let mut acc = a * b + c - d;
    if acc > 10 {
        acc += busy(acc, b, c, d) * 2;
    } else if acc < 0 {
        acc = 0;
    } else {
        acc -= 1;
    }
    match acc % 4 {
        0 => acc + 1,
        1 => acc * 3,
        2 => acc - 5,
        _ => acc / 7,
    }
}

fn trivial() {}
",
    )
    .unwrap();
}

#[test]
fn json_output_shape_and_ranking() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--json", "--top", "10", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();

    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["summary"]["files"], 1);
    assert_eq!(json["summary"]["functions"], 3);
    assert!(json["summary"]["median_effort"].as_f64().unwrap() > 0.0);

    let results = json["results"].as_array().unwrap();
    assert_eq!(results.len(), 3);

    assert_eq!(results[0]["name"], "busy");
    assert!(results[0]["priority"].as_f64().unwrap() > 0.0);
    assert_ne!(results[0]["reason"].as_str().unwrap(), "");

    for r in results {
        for field in ["priority", "difficulty", "effort", "volume", "mi_vs"] {
            let v = r[field].as_f64().unwrap();
            let scaled = v * 1000.0;
            assert!(
                (scaled - scaled.round()).abs() < 1e-6,
                "{field} = {v} has more than 3 decimals"
            );
        }
    }

    assert_eq!(results[0]["start_line"], 7);

    let last = results.last().unwrap();
    assert_eq!(last["name"], "trivial");
    assert_eq!(last["priority"], 0.0);
    assert_eq!(last["mi_vs"], 100.0);

    for r in results {
        for field in [
            "path",
            "name",
            "start_line",
            "end_line",
            "difficulty",
            "effort",
            "volume",
            "estimated_bugs",
            "purity_ratio",
            "mi_vs",
            "mi_original",
            "mi_band",
            "cyclomatic",
            "loc",
            "nesting",
            "priority",
            "reason",
        ] {
            assert!(r.get(field).is_some(), "missing field {field} in {r}");
        }
    }

    assert_eq!(results[0]["language"], "rust");
    assert_eq!(json["summary"]["languages"]["rust"], 3);
    assert_eq!(json["summary"]["languages"]["python"], 0);

    assert!(json["summary"]["mi_median"].as_f64().unwrap() > 0.0);
    let bands = &json["summary"]["mi_bands"];
    assert_eq!(bands["high"], 1);
    assert_eq!(bands["moderate"], 0);
    assert_eq!(bands["low"], 1);
    assert_eq!(results[0]["mi_band"], "low");
    assert_eq!(results[2]["mi_band"], "high");
}

#[test]
fn table_output() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--format", "table", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("PRIORITY"));
    assert!(stdout.contains("busy"));
}

#[test]
fn gates_set_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());

    let out = halmi()
        .args([
            "--min-mi",
            "99.5",
            "--max-difficulty",
            "0.01",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("violated"), "stderr: {stderr}");
}

#[test]
fn gates_pass_when_clean() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--min-mi", "1.0", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn baseline_changed_only_mode() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let baseline_path = dir.path().join("baseline.json");

    let first = halmi()
        .args(["--top", "50", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    std::fs::write(&baseline_path, &first.stdout).unwrap();

    let lib = dir.path().join("src/lib.rs");
    let mut src = std::fs::read_to_string(&lib).unwrap();
    src = src.replace(
        "    let mut acc = a * b + c - d;",
        "    let mut acc = a * b + c - d + a * b - c * d + (a + b) * (c - d) * (a - b);",
    );
    std::fs::write(&lib, src).unwrap();

    let second = halmi()
        .args([
            "--baseline",
            baseline_path.to_str().unwrap(),
            "--changed-only",
            "--top",
            "50",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        second.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let json: Value = serde_json::from_slice(&second.stdout).unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(
        !results.is_empty(),
        "expected at least the worsened function"
    );
    assert_eq!(results[0]["name"], "busy");
    assert_eq!(results[0]["delta_status"], "worsened");
    let delta = &results[0]["delta"];
    assert!(delta["priority"].as_f64().unwrap() > 0.0);
    assert!(delta["effort"].as_f64().unwrap() > 0.0);
    assert_eq!(json["summary"]["delta"]["worsened"], 1);
}

#[test]
fn parse_errors_do_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/bad.rs"), "fn broken( {").unwrap();
    std::fs::write(dir.path().join("src/good.rs"), "fn ok() -> u32 { 1 }").unwrap();
    let out = halmi()
        .args(["--top", "10", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_ne!(
        json["summary"]["errors"].as_array().unwrap().as_slice(),
        &[] as &[Value]
    );
    assert_eq!(json["summary"]["files"], 2);
}

#[test]
fn help_and_version() {
    let help = halmi().arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("USAGE"));
    let version = halmi().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).starts_with("halmi "));
}

#[test]
fn bad_flag_exits_two() {
    let out = halmi().arg("--nope").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn defaults_scan_the_current_directory() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi().current_dir(dir.path()).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["root"], ".");
    assert_eq!(json["summary"]["functions"], 3);
}

#[test]
fn top_limits_results_in_both_flag_forms() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for form in [["--top", "1"].as_slice(), ["--top=1"].as_slice()] {
        let out = halmi()
            .args(form)
            .arg(dir.path().to_str().unwrap())
            .output()
            .unwrap();
        assert!(out.status.success());
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["results"].as_array().unwrap().len(), 1, "{form:?}");
    }
}

#[test]
fn json_alias_matches_explicit_format() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--json", "--format", "json", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["schema_version"], 2);
}

#[test]
fn changed_only_without_baseline_is_rejected() {
    let out = halmi().arg("--changed-only").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--changed-only requires --baseline"),
        "{stderr}"
    );
}

#[test]
fn invalid_gate_value_exits_two() {
    let out = halmi().arg("--min-mi").arg("abc").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--min-mi expects a number"), "{stderr}");
}

#[test]
fn single_file_argument_is_analyzed_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lonely.rs");
    std::fs::write(&file, "fn solo() -> u32 { 1 }").unwrap();
    let out = halmi().arg(file.to_str().unwrap()).output().unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["files"], 1);
    assert_eq!(json["results"][0]["name"], "solo");
    assert_eq!(json["results"][0]["path"], file.to_str().unwrap());
}

#[test]
fn table_truncates_long_unicode_names_without_panicking() {
    let name = "aaaaaaaaaaaaaaaaaaaaaaaaaa\u{fc}_name_that_is_long";
    let src = format!("fn {name}() -> u32 {{ 1 }}");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.rs"), src).unwrap();
    let out = halmi()
        .args(["--format", "table", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("aaaaaaaaaaaaaaaaaaaaaaaaaa"), "{stdout}");
    assert!(
        !stdout.contains("\u{fc}_name"),
        "cut must land before the multibyte char"
    );
}

#[test]
fn sarif_output_shape() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--format",
            "sarif",
            "--top",
            "10",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sarif: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        sarif["$schema"],
        "https://json.schemastore.org/sarif-2.1.0.json"
    );
    assert_eq!(sarif["version"], "2.1.0");
    let run = &sarif["runs"][0];
    assert_eq!(run["tool"]["driver"]["name"], "halmi");
    let results = run["results"].as_array().unwrap();

    assert_eq!(results.len(), 2, "{results:?}");
    let busy = results
        .iter()
        .find(|r| r["message"]["text"].as_str().unwrap().contains("busy"))
        .expect("busy present");
    assert_eq!(busy["ruleId"], "HALMI001");
    assert_eq!(busy["level"], "note");
    let loc = &busy["locations"][0]["physicalLocation"];
    assert!(
        loc["artifactLocation"]["uri"]
            .as_str()
            .unwrap()
            .ends_with("src/lib.rs")
    );
    assert_eq!(loc["region"]["startLine"], 7);
    assert_eq!(loc["region"]["endLine"], 22);

    let text = busy["message"]["text"].as_str().unwrap();
    assert!(text.contains("priority"), "{text}");
    assert!(text.contains("MI"), "{text}");
}

#[test]
fn sarif_marks_gate_violations_as_warning() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--format",
            "sarif",
            "--min-mi",
            "99.5",
            "--max-difficulty",
            "0.01",
            "--max-effort",
            "0.001",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let sarif: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let busy = results
        .iter()
        .find(|r| r["message"]["text"].as_str().unwrap().contains("busy"))
        .expect("busy present");
    assert_eq!(busy["level"], "warning");
}

#[test]
fn min_band_gate_sets_exit_code_and_is_named_in_stderr() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());

    let out = halmi()
        .args(["--min-band", "high", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("min-band=high"), "{stderr}");
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["gates"]["min_band"], "high");
    assert_eq!(json["summary"]["gates"]["violations"], 1);

    let out = halmi()
        .args(["--min-band", "low", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));

    let out = halmi().arg("--min-band").arg("ultra").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("high, moderate, or low"));
}

#[test]
fn method_chain_is_reported_for_iterator_spines() {
    let dir = tempfile::tempdir().unwrap();
    let src = "fn chainy(v: Vec<u32>) -> u32 {\n    v.iter().map(|x| x + 1).filter(|x| *x > 2).take(3).sum()\n}\n";
    std::fs::write(dir.path().join("c.rs"), src).unwrap();
    let out = halmi().arg(dir.path().to_str().unwrap()).output().unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["results"][0]["method_chain"], 5);
}

#[test]
fn gates_stderr_names_every_enabled_gate() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--min-mi",
            "99.5",
            "--max-difficulty",
            "0.01",
            "--max-effort",
            "0.001",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("min-mi"), "{stderr}");
    assert!(stderr.contains("max-difficulty"), "{stderr}");
    assert!(stderr.contains("max-effort"), "{stderr}");
}

#[test]
fn gate_stderr_has_exact_single_labels() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--min-mi",
            "99.5",
            "--max-difficulty",
            "0.01",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("(min-mi=99.5 max-difficulty=0.01 max-effort=- min-band=-)"),
        "exact gate summary malformed: {stderr}"
    );
}

#[test]
fn nonfinite_gate_values_are_rejected() {
    for bad in [
        vec!["--min-mi", "nan"],
        vec!["--min-mi", "inf"],
        vec!["--max-difficulty", "nan"],
        vec!["--max-difficulty=inf"],
        vec!["--max-effort", "inf"],
    ] {
        let out = halmi().args(&bad).arg(".").output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{bad:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("finite"), "{bad:?}: {stderr}");
    }
}

#[test]
fn baseline_mismatch_warns_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let base = dir.path().join("base.json");
    let first = halmi()
        .args(["--top", "2", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    std::fs::write(&base, &first.stdout).unwrap();
    let second = halmi()
        .args([
            "--baseline",
            base.to_str().unwrap(),
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains(
            "warning: 1 of 3 functions are new vs the baseline (baseline holds 2 functions)"
        ),
        "{stderr}"
    );
}

#[test]
#[cfg(unix)]
fn symlinked_directories_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("a.rs"), "fn once() -> u32 { 1 }").unwrap();
    std::os::unix::fs::symlink(&sub, dir.path().join("link")).unwrap();
    let out = halmi().arg(dir.path().to_str().unwrap()).output().unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["summary"]["functions"], 1,
        "symlink must not duplicate"
    );
}

#[test]
fn gates_name_their_violators() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--min-mi",
            "99.5",
            "--top",
            "1",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let violators = json["summary"]["gates"]["violators"].as_array().unwrap();
    let joined: Vec<&str> = violators.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(
        joined
            .iter()
            .any(|v| v.starts_with("busy@") && v.ends_with(":7")),
        "{joined:?}"
    );
    assert!(
        !joined.iter().any(|v| v.starts_with("trivial@")),
        "{joined:?}"
    );
}

#[test]
fn baseline_directory_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = halmi()
        .arg("--baseline")
        .arg(dir.path().to_str().unwrap())
        .arg(".")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("baseline path is a directory"));
}

#[test]
fn top_rejects_plus_sign_and_dash_dash_ends_flags() {
    let out = halmi().arg("--top").arg("+5").arg(".").output().unwrap();
    assert_eq!(out.status.code(), Some(2));

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("--dashy.rs"), "fn dashed() -> u32 { 1 }").unwrap();
    let out = halmi()
        .current_dir(dir.path())
        .args(["--", "--dashy.rs"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["functions"], 1);
}

#[test]
fn exclude_patterns_filter_files() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::create_dir(dir.path().join("vendor")).unwrap();
    std::fs::write(dir.path().join("vendor/v.rs"), "fn vendored() -> u32 { 1 }").unwrap();
    let out = halmi()
        .args(["--exclude", "*/vendor/*", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["files"], 1);
    let out = halmi()
        .args(["--exclude", "*/src/lib.rs", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["files"], 1, "full-path suffix exclude");
}

#[test]
fn sort_keys_order_the_results() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--sort", "effort", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let efforts: Vec<f64> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["effort"].as_f64().unwrap())
        .collect();
    let mut sorted = efforts.clone();
    sorted.sort_by(|a, b| b.total_cmp(a));
    assert_eq!(efforts, sorted, "effort must be descending");

    let out = halmi()
        .args(["--sort", "mi", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let mis: Vec<f64> = json["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["mi_vs"].as_f64().unwrap())
        .collect();
    let mut sorted = mis.clone();
    sorted.sort_by(f64::total_cmp);
    assert_eq!(mis, sorted, "mi must be ascending (worst first)");

    let empty_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(empty_dir.path().join("src")).unwrap();
    std::fs::write(empty_dir.path().join("src/lib.py"), "# no functions\n").unwrap();
    for by_file in [false, true] {
        let mut command = halmi();
        command.args(["--sort", "effort"]);
        if by_file {
            command.arg("--by-file");
        }
        let out = command.arg(empty_dir.path()).output().unwrap();
        assert!(
            out.status.success(),
            "empty result must not panic (by_file={by_file}): {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(json["summary"]["functions"], 0);
        if by_file {
            assert_eq!(json["files"].as_array().unwrap().len(), 0);
        } else {
            assert_eq!(json["results"].as_array().unwrap().len(), 0);
        }
    }

    let out = halmi().arg("--sort").arg("prio").arg(".").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn min_priority_floors_the_results() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--min-priority", "0.01", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_ne!(results.as_slice(), &[] as &[Value]);
    assert!(
        results
            .iter()
            .all(|r| r["priority"].as_f64().unwrap() >= 0.01)
    );
    assert_eq!(json["summary"]["functions"], 3, "summary counts everything");

    let out = halmi()
        .arg("--min-priority")
        .arg("999999")
        .arg(dir.path())
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["results"].as_array().unwrap().len(), 0);
}

#[test]
fn by_file_rollup_shape_and_rejections() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args(["--by-file", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    let f = &files[0];
    assert!(f["path"].as_str().unwrap().ends_with("src/lib.rs"));
    assert_eq!(f["functions"], 3);
    assert!(f["total_loc"].as_u64().unwrap() > 0);
    assert!(f["total_effort"].as_f64().unwrap() > 0.0);
    assert!(f["worst"].as_str().unwrap().starts_with("busy@"));
    assert_eq!(json["results"].as_array().unwrap().len(), 0);

    let out = halmi()
        .args([
            "--by-file",
            "--format",
            "table",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let table = String::from_utf8_lossy(&out.stdout);
    assert!(table.contains("MAXPRI"));
    assert!(table.contains("MAX_D"));
    assert!(table.contains("MED_MI"));

    let sorted_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sorted_dir.path().join("src")).unwrap();
    let add = r"pub fn add(a: u32, b: u32) -> u32 {
    a + b
}
";
    let busy = r"pub fn busy(a: u32, b: u32, c: u32, d: u32) -> u32 {
    a * b + c - d
}
";
    std::fs::write(sorted_dir.path().join("src/add.rs"), add).unwrap();
    std::fs::write(
        sorted_dir.path().join("src/busy.rs"),
        format!("{add}{busy}"),
    )
    .unwrap();
    let out = halmi()
        .args([
            "--by-file",
            "--sort",
            "difficulty",
            sorted_dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let files = json["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0]["path"].as_str().unwrap().ends_with("busy.rs"));
    let add_row = files
        .iter()
        .find(|row| row["path"].as_str().unwrap().ends_with("add.rs"))
        .unwrap();
    let busy_row = files
        .iter()
        .find(|row| row["path"].as_str().unwrap().ends_with("busy.rs"))
        .unwrap();
    assert_eq!(add_row["max_difficulty"], 1.0);
    assert_eq!(busy_row["max_difficulty"], 2.0);
    assert_eq!(add_row["median_mi_vs"], 93.542);
    assert!((busy_row["median_mi_vs"].as_f64().unwrap() - 91.8715).abs() < 1e-9);

    let out = halmi()
        .args(["--by-file", "--format", "sarif", "."])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("json and table"));

    let out = halmi()
        .arg("--by-file")
        .arg("--baseline")
        .arg("x")
        .arg(".")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn color_modes_control_table_ansi() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--format",
            "table",
            "--color",
            "always",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains('\u{1b}'),
        "always must emit ANSI: {stdout:?}"
    );

    let out = halmi()
        .args([
            "--format",
            "table",
            "--color",
            "never",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains('\u{1b}'), "never must not emit ANSI");

    let out = halmi()
        .arg("--color")
        .arg("sometimes")
        .arg(".")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn halstead_display_floors_filter_results() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = halmi()
        .args([
            "--min-effort",
            "100",
            "--min-difficulty",
            "1",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    let results = json["results"].as_array().unwrap();
    assert_ne!(results.as_slice(), &[] as &[Value]);
    assert!(results.iter().all(
        |r| r["effort"].as_f64().unwrap() >= 100.0 && r["difficulty"].as_f64().unwrap() >= 1.0
    ));
    assert_eq!(json["summary"]["functions"], 3, "floors are display-only");

    let out = halmi()
        .args(["--min-volume", "999999", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["results"].as_array().unwrap().len(), 0);

    let out = halmi().args(["--min-effort", "nan", "."]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn config_file_is_loaded_and_cli_wins() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("halmi.toml"),
        "top = 1\nsort = \"effort\"\nexclude = [\"*/vendor/*\"]\n\n[gates]\nmin_mi = 99.5\n",
    )
    .unwrap();
    let out = halmi().current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "config gate must fire");
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["results"].as_array().unwrap().len(),
        1,
        "config top = 1"
    );
    assert_eq!(json["summary"]["gates"]["min_mi"], 99.5);
    let first_effort = json["results"][0]["effort"].as_f64().unwrap();

    let out = halmi()
        .current_dir(dir.path())
        .args(["--top", "2"])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["results"].as_array().unwrap().len(),
        2,
        "CLI top beats config"
    );
    assert!(
        json["results"][0]["effort"].as_f64().unwrap() >= first_effort,
        "config sort holds"
    );

    std::fs::write(
        dir.path().join("halmi.toml"),
        "exclude = [\"*/vendor/*\", \"*\"]\n",
    )
    .unwrap();
    let out = halmi().current_dir(dir.path()).output().unwrap();
    assert!(out.stdout.is_empty(), "every array element must apply");
    assert!(String::from_utf8_lossy(&out.stderr).contains("no source files found"));
}

#[test]
fn config_errors_are_strict() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("halmi.toml"), "nope = 1\n").unwrap();
    let out = halmi().current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown key `nope`"));

    std::fs::write(dir.path().join("halmi.toml"), "[unknown]\n").unwrap();
    let out = halmi().current_dir(dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown section"));
}

#[test]
fn path_containing_equals_is_not_mangled() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a=b.rs");
    std::fs::write(&file, "fn eq() -> u32 { 1 }").unwrap();
    let out = halmi().arg(file.to_str().unwrap()).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["summary"]["functions"], 1);
}

#[test]
fn explicit_missing_config_is_an_error() {
    let out = halmi()
        .arg("--config")
        .arg("/nonexistent/halmi.toml")
        .arg(".")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("config file not found"));
}

#[test]
fn config_by_file_with_baseline_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(dir.path().join("halmi.toml"), "by_file = true\n").unwrap();
    let base = dir.path().join("base.json");
    let first = halmi().current_dir(dir.path()).output().unwrap();
    std::fs::write(&base, &first.stdout).unwrap();
    let out = halmi()
        .current_dir(dir.path())
        .args(["--baseline", base.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--by-file does not support --baseline"));
}
