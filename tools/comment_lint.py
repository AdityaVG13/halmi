#!/usr/bin/env python3
"""comment_lint: ban implementation comments in halmi (comment policy;
see the delete-comment forge guide and crates/*/CONTRACT.md).

Counts `//` and `/* */` comments in crates/*/src and crates/*/tests,
skipping string and char literals (including raw strings and escaped
chars). `///` and `//!` rustdoc is API documentation and is allowed.

  python3 tools/comment_lint.py            # fail if count > ratchet
  python3 tools/comment_lint.py --update   # lower the ratchet to the current count

The ratchet lives in tools/comment_ratchet.txt and may only decrease.
"""
import sys
import glob
import pathlib

RATCHET = pathlib.Path(__file__).with_name("comment_ratchet.txt")


def count_impl_comments(src: str) -> int:
    b = src.encode()
    n = i = 0
    state, hashes = "code", 0
    while i < len(b):
        c = b[i : i + 1]
        if state == "code":
            if c == b"/" and b[i + 1 : i + 2] == b"/":
                if not src[i : i + 3] in ("///", "//!"):
                    n += 1
                while i < len(b) and b[i : i + 1] != b"\n":
                    i += 1
            elif c == b"/" and b[i + 1 : i + 2] == b"*":
                n += 1
                depth = 1
                i += 2
                while i < len(b) and depth > 0:
                    if b[i : i + 2] == b"/*":
                        depth += 1
                        i += 2
                    elif b[i : i + 2] == b"*/":
                        depth -= 1
                        i += 2
                    else:
                        i += 1
            elif c == b'"':
                state = "str"
                i += 1
            elif c == b"'":
                if b[i + 1 : i + 2] == b"\\":  # escaped char literal: to closing quote
                    i += 2
                    while i < len(b) and b[i : i + 1] != b"'":
                        i += 1
                    i += 1
                elif b[i + 2 : i + 3] == b"'" and b[i + 1 : i + 2] != b"'":
                    i += 3
                else:  # lifetime/label
                    i += 1
            elif c in (b"r", b"b"):
                j = i
                if b[j : j + 1] == b"b":
                    j += 1
                has_r = b[j : j + 1] == b"r"
                if has_r:
                    j += 1
                matched = None
                if j < len(b):
                    if has_r:
                        h = 0
                        while j < len(b) and b[j : j + 1] == b"#":
                            h += 1
                            j += 1
                        if b[j : j + 1] == b'"':
                            matched = ("raw", h, j + 1 - i)
                    elif b[j : j + 1] == b'"':
                        matched = ("str", 0, j + 1 - i)
                    elif b[j : j + 1] == b"'":
                        k = j + 1
                        if b[k : k + 1] == b"\\":
                            k += 2
                            while k < len(b) and b[k : k + 1] != b"'":
                                k += 1
                            k += 1
                        else:
                            k += 2
                        matched = ("bytechar", 0, k - i)
                if matched:
                    if matched[0] == "raw":
                        state = "rawstr"
                    elif matched[0] == "str":
                        state = "str"
                    hashes = matched[1]
                    i += matched[2]
                else:
                    i += 1
            else:
                i += 1
        elif state == "str":
            if c == b"\\":
                i += 2
            elif c == b'"':
                state = "code"
                i += 1
            else:
                i += 1
        else:  # rawstr
            if c == b'"':
                j = i + 1
                ok = True
                for _ in range(hashes):
                    if j >= len(b) or b[j : j + 1] != b"#":
                        ok = False
                        break
                    j += 1
                if ok:
                    i = j
                    state = "code"
                    continue
            i += 1
    return n


def main() -> int:
    files = sorted(
        glob.glob("crates/*/src/**/*.rs", recursive=True)
        + glob.glob("crates/*/tests/**/*.rs", recursive=True)
    )
    counts = {}
    for f in files:
        counts[f] = count_impl_comments(pathlib.Path(f).read_text())
    total = sum(counts.values())
    limit = int(RATCHET.read_text().strip()) if RATCHET.exists() else 0
    if "--update" in sys.argv:
        RATCHET.write_text(f"{total}\n")
        print(f"ratchet updated to {total}")
        return 0
    for f, n in counts.items():
        if n:
            print(f"{f}: {n} implementation comment(s)")
    if total > limit:
        print(f"comment_lint: FAIL {total} > ratchet {limit} (comment policy; use tools/delete-comment forge guide)")
        return 1
    print(f"comment_lint: ok ({total}/{limit})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
