#!/usr/bin/env python3
"""Add compiler codes to crates/svn-lint/src/codes.rs in sorted position.

The catalog is generated from the compiler's warning messages; codes for
compiler *errors* that the lint pass also reports are added by hand, in
the same four places (enum, as_str, from_str, CODES) and in
COMPILER_ERROR_CODES.

    scripts/add-lint-code.py <code> [<code> ...]
"""
import re
import sys

path = "crates/svn-lint/src/codes.rs"
s = open(path).read()


def section(start_marker, end_marker):
    a = s.index(start_marker)
    return a, s.index(end_marker, a)


for code in sys.argv[1:]:
    if re.search(rf"\b{code}\b", s):
        continue
    # enum
    a, b = section("pub enum Code {", "\n}")
    body = s[a:b]
    names = re.findall(r"^    (\w+),$", body, re.M)
    names_sorted = sorted(names + [code])
    idx = names_sorted.index(code)
    if idx + 1 < len(names_sorted):
        nxt = names_sorted[idx + 1]
        body = re.sub(
            rf"\n    {nxt},(?=\n|$)", f"\n    {code},\n    {nxt},", body, count=1
        )
    else:
        body = body + f"\n    {code},"
    s = s[:a] + body + s[b:]
    # as_str
    a, b = section("pub const fn as_str(self)", "\n        }")
    body = s[a:b]
    names = re.findall(r"Self::(\w+) =>", body)
    nxt = next((n for n in sorted(names) if n > code), None)
    line = f'            Self::{code} => "{code}",\n'
    body = body.replace(f"            Self::{nxt} =>", line + f"            Self::{nxt} =>", 1)
    s = s[:a] + body + s[b:]
    # from_str
    a = s.index('"' + sorted(names)[0] + '" => ')
    b = s.index("_ => None", a)
    body = s[a:b]
    nxt_str = f'            "{nxt}" =>'
    body = body.replace(nxt_str, f'            "{code}" => Some(Self::{code}),\n' + nxt_str, 1)
    s = s[:a] + body + s[b:]
    # CODES
    a = s.index("pub const CODES")
    tail = s[a:]
    count = int(re.search(r"&\[&str; (\d+)\]", tail).group(1))
    tail = tail.replace(f"&[&str; {count}]", f"&[&str; {count + 1}]", 1)
    tail = tail.replace(f'    "{nxt}",\n', f'    "{code}",\n    "{nxt}",\n', 1)
    s = s[:a] + tail
    # COMPILER_ERROR_CODES: every hand-added code is a compiler error.
    a, b = section("pub const COMPILER_ERROR_CODES: &[&str] = &[\n", "\n];")
    body = s[a:b]
    names = re.findall(r'^    "(\w+)",$', body, re.M)
    entries = "".join(f'    "{n}",\n' for n in sorted(names + [code]))
    head = "pub const COMPILER_ERROR_CODES: &[&str] = &[\n"
    s = s[:a] + head + entries.rstrip("\n") + s[b:]

open(path, "w").write(s)
