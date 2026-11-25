"""Span differential between halmi and a tree-sitter oracle over a local corpus.

usage: python3 tools/oracle_diff.py <halmi-binary> <mode> [limit]

The oracle is a private developer tool: it is not in this repository and
nothing here builds it. This script is shipped so that anyone holding an
equivalent dump tool can rerun the comparison. The harness invokes the tool as
`<tool> <lang> --dump2` with one source file on stdin and expects one
tab-separated row per syntax node (at least eight columns): node kind in
column 2, 0-based start and end lines in columns 5 and 7, and `ERROR` rows
with byte offsets in columns 3 and 4.

The oracle binary comes from HALMI_ORACLE, else `ta-oracle` on PATH. The corpus
for a mode comes from HALMI_CORPUS_<MODE> (upper-case mode name): one or more
glob patterns separated by the platform PATH separator. A pattern without a
wildcard that names a directory is walked recursively for the mode's file
extensions. Paths containing one of HALMI_CORPUS_EXCLUDE (same separator,
default: build and dependency directories) are skipped. Corpora are never
vendored; an unset or empty corpus is a configuration error, not a pass.
"""
import glob
import json
import os
import shutil
import subprocess
import sys

MODES = {
    'swift': ('swift', ('*.swift',), {'function_declaration', 'init_declaration', 'deinit_declaration'}),
    'c': ('c', ('*.c', '*.h'), {'function_definition'}),
    'cpp': ('cpp', ('*.cpp', '*.cc', '*.cxx', '*.hpp', '*.hxx', '*.hh', '*.ipp'),
            {'function_definition'}),
    'java': ('java', ('*.java',), {'method_declaration', 'constructor_declaration', 'static_initializer'}),
    'cs': ('c_sharp', ('*.cs',), {'method_declaration', 'constructor_declaration'}),
    'kt': ('kotlin', ('*.kt',), {'function_declaration', 'secondary_constructor'}),
    'go': ('go', ('*.go',), {'function_declaration', 'method_declaration'}),
    'php': ('php', ('*.php',), {'function_definition', 'method_declaration'}),
    'rb': ('ruby', ('*.rb',), {'method', 'singleton_method'}),
    'lua': ('lua', ('*.lua',), {'function_declaration'}),
    'zig': ('zig', ('*.zig',), {'function_declaration', 'test_declaration'}),
    'script': (None, ('*.ts', '*.tsx', '*.js', '*.jsx', '*.mjs', '*.cjs'),
               {'function_declaration', 'function_expression', 'arrow_function', 'method_definition',
                'generator_function', 'generator_function_declaration'}),
}
DEFAULT_EXCLUDES = ('.build', 'node_modules', '/target/', '.zig-cache', 'zig-out', '/.git/')
# Modes whose adapters count definitions only: an oracle declaration whose span
# text opens no block is a body-less signature (Go assembly stubs) and is
# excluded from both sides before comparison.
DEFINITIONS_ONLY = {'go'}


def lang_of(path):
    if path.endswith('.tsx'):
        return 'tsx'
    if path.endswith(('.ts', '.mts', '.cts')):
        return 'typescript'
    return 'javascript'


def corpus_files(mode, extensions):
    raw = os.environ.get('HALMI_CORPUS_' + mode.upper(), '')
    patterns = [p for p in raw.split(os.pathsep) if p] if raw else []
    excludes = [e for e in os.environ.get('HALMI_CORPUS_EXCLUDE', '').split(os.pathsep) if e] or list(DEFAULT_EXCLUDES)
    files = []
    for pattern in patterns:
        pattern = os.path.expanduser(pattern)
        if not any(ch in pattern for ch in '*?[') and os.path.isdir(pattern):
            matches = []
            for ext in extensions:
                matches += glob.glob(os.path.join(pattern, '**', ext), recursive=True)
        else:
            matches = glob.glob(pattern, recursive=True)
        for p in matches:
            normalized = p.replace(os.sep, '/')
            if os.path.isfile(p) and p.endswith(tuple(e[1:] for e in extensions)) \
                    and not any(x in normalized for x in excludes):
                files.append(p)
    return list(dict.fromkeys(files)), patterns


ORACLE = os.environ.get('HALMI_ORACLE') or shutil.which('ta-oracle')
if not ORACLE:
    print("oracle binary not found: set HALMI_ORACLE or put `ta-oracle` on PATH")
    sys.exit(2)
if len(sys.argv) < 2:
    print(__doc__.strip())
    sys.exit(2)
HALMI = sys.argv[1]
mode = sys.argv[2] if len(sys.argv) > 2 else 'script'
limit = int(sys.argv[3]) if len(sys.argv) > 3 else 40
if mode not in MODES:
    print(f"unknown mode `{mode}`; expected one of {', '.join(sorted(MODES))}")
    sys.exit(2)
lang, extensions, FN_KINDS = MODES[mode]

files, patterns = corpus_files(mode, extensions)
files = files[:limit]
if not files:
    print(f"no source files selected for mode `{mode}`")
    print(f"  HALMI_CORPUS_{mode.upper()}={os.environ.get('HALMI_CORPUS_' + mode.upper(), '')!r}")
    print("  expected glob patterns separated by " + repr(os.pathsep)
          + f" matching {', '.join(extensions)}")
    sys.exit(2)

total_oracle = total_halmi = matched = 0
mismatches = []
evaluated = 0
bodyless = 0
oracle_skips = []
halmi_failures = []
print(f"mode: {mode} | oracle: {ORACLE} | files selected: {len(files)}")
for pattern in patterns:
    print("  corpus:", pattern)
for path in files:
    file_lang = lang if lang is not None else lang_of(path)
    try:
        dump = subprocess.run([ORACLE, file_lang, '--dump2'], stdin=open(path, 'rb'), capture_output=True, timeout=20)
    except Exception as exc:
        oracle_skips.append((path, f"{type(exc).__name__}: {exc}"))
        continue
    if dump.returncode != 0:
        detail = dump.stderr.decode('utf-8', 'replace').strip()[-400:]
        oracle_skips.append((path, f"exit {dump.returncode}: {detail}"))
        continue
    lines_dump = dump.stdout.decode('utf-8', 'replace').splitlines()
    err_bytes = sum(int(p.split('\t')[3]) - int(p.split('\t')[2]) for p in lines_dump
                    if p.split('\t')[1:2] == ['ERROR'] and p.count('\t') > 4)
    total_bytes = max(1, len(open(path, 'rb').read()))
    oracle_bad = err_bytes / total_bytes > 0.30
    oracle_spans = set()
    src_lines = open(path, 'rb').read().decode('utf-8', 'replace').splitlines()
    for line in lines_dump:
        parts = line.split('\t')
        if len(parts) < 8:
            continue
        kind = parts[1]
        if kind in FN_KINDS:
            try:
                sr, er = int(parts[4]), int(parts[6])
            except ValueError:
                continue
            if mode in DEFINITIONS_ONLY and '{' not in '\n'.join(src_lines[sr:er + 1]):
                bodyless += 1
                continue
            oracle_spans.add((sr + 1, er + 1))
    try:
        h = subprocess.run([HALMI, '--top', '100000', path], capture_output=True, timeout=20)
        hj = json.loads(h.stdout)
    except Exception as exc:
        halmi_failures.append((path, f"{type(exc).__name__}: {exc}"))
        continue
    if h.returncode != 0:
        detail = h.stderr.decode('utf-8', 'replace').strip()[-400:]
        halmi_failures.append((path, f"exit {h.returncode}: {detail}"))
        continue
    evaluated += 1
    halmi_spans = {(r['start_line'], r['end_line']) for r in hj['results']}
    total_oracle += len(oracle_spans)
    total_halmi += len(halmi_spans)
    matched += len(oracle_spans & halmi_spans)
    if oracle_spans != halmi_spans:
        halmi_error = next((e['message'] for e in hj['summary'].get('errors') or []
                            if e.get('path', '').replace(os.sep, '/').endswith(path.replace(os.sep, '/'))), '')
        mismatches.append((path, len(oracle_spans), len(halmi_spans), len(oracle_spans & halmi_spans),
                           oracle_bad, halmi_error))
bad_oracle = [m for m in mismatches if m[4]]
good_mismatches = [m for m in mismatches if not m[4]]
halmi_errored = [m for m in good_mismatches if m[5]]
print(f"files evaluated: {evaluated}/{len(files)} | oracle fns: {total_oracle} | halmi fns: {total_halmi} | exact span matches: {matched}")
if bodyless:
    print(f"oracle body-less declarations excluded by contract: {bodyless}")
if oracle_skips:
    print(f"oracle skips: {len(oracle_skips)}")
    for path, reason in oracle_skips[:8]:
        print("  [oracle-skip]", path.split('/')[-1], reason)
if halmi_failures:
    print(f"halmi execution failures: {len(halmi_failures)}")
    for path, reason in halmi_failures[:8]:
        print("  [halmi-failure]", path.split('/')[-1], reason)
if evaluated == 0:
    print("no usable oracle/halmi comparisons")
print(f"files with span differences: {len(good_mismatches)} (plus {len(bad_oracle)} files where the oracle itself is >30% ERROR nodes;"
      f" {len(halmi_errored)} of the span-difference files carry a halmi parse error)")
for m in good_mismatches[:8]:
    print("  ", m[0].split('/')[-1], "oracle", m[1], "halmi", m[2], "matched", m[3],
          f"[halmi-error: {m[5]}]" if m[5] else "")
for m in bad_oracle[:4]:
    print("  [oracle-error]", m[0].split('/')[-1], "oracle", m[1], "halmi", m[2], "matched", m[3])

if halmi_failures or evaluated == 0:
    sys.exit(1)
