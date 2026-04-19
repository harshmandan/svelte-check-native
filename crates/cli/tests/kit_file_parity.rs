//! Parity test: our Kit-file enumeration must match upstream
//! `svelte-check`'s `isKitFile` byte-for-byte on a fixture tree.
//!
//! ### What this test does
//!
//! 1. Materializes a fixture directory with a known set of `.svelte`,
//!    Kit (`+page.ts`, `+layout.server.ts`, `hooks.server.ts`, ...),
//!    and unrelated (`lib/foo.ts`, `README.md`) files.
//! 2. Spawns `node` with an inline reimplementation of upstream's
//!    `isKitFile` — the one it ships in `dist/src/index.js`. The
//!    script emits a JSON list of files upstream would count.
//! 3. Invokes our binary's `--list-relevant` introspection to get
//!    the files we count.
//! 4. Asserts the two sets are identical (as sorted path lists).
//!
//! The test skips (not fails) when `node` is not on PATH — matches
//! the pattern in `upstream_sanity.rs`.
//!
//! ### Why this shape
//!
//! The COMPLETED-line denominator upstream prints is
//! `|entries ∪ files-with-diagnostics|`. The *entries* half is
//! fully determined by the enumeration; the diagnostic half depends
//! on what tsgo reports. We pin the enumeration half here — the
//! only half we can deterministically compare — and trust the
//! diagnostic-union logic from the `seen` set construction to
//! layer on top identically for both tools.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Inline JS reimplementation of upstream svelte-check 4.4.6's Kit-file
/// detection. Source: node_modules/svelte-check/dist/src/index.js —
/// `isKitFile` / `isKitRouteFile` / `isHooksFile` / `isParamsFile`
/// plus the `findFiles` filter. Reproduced verbatim so this test
/// catches any drift from upstream's algorithm, not just our own bugs.
const UPSTREAM_ENUM_JS: &str = r#"
const fs = require('fs');
const path = require('path');

const kitPageFiles = new Set(['+page', '+layout', '+page.server', '+layout.server', '+server']);
const defaults = {
    paramsPath:         'src/params',
    serverHooksPath:    'src/hooks.server',
    clientHooksPath:    'src/hooks.client',
    universalHooksPath: 'src/hooks',
};

function isKitRouteFile(basename) {
    if (basename.includes('@')) basename = basename.split('@')[0];
    else basename = basename.slice(0, -path.extname(basename).length);
    return kitPageFiles.has(basename);
}
function isHooksFile(fileName, basename, hooksPath) {
    return (
        ((basename === 'index.ts' || basename === 'index.js') &&
            fileName.slice(0, -basename.length - 1).endsWith(hooksPath)) ||
        fileName.slice(0, -path.extname(basename).length).endsWith(hooksPath)
    );
}
function isParamsFile(fileName, basename, paramsPath) {
    return (
        fileName.slice(0, -basename.length - 1).endsWith(paramsPath) &&
        !basename.includes('.test') &&
        !basename.includes('.spec')
    );
}
function isKitFile(fileName, o) {
    const b = path.basename(fileName);
    return isKitRouteFile(b)
        || isHooksFile(fileName, b, o.serverHooksPath)
        || isHooksFile(fileName, b, o.clientHooksPath)
        || isHooksFile(fileName, b, o.universalHooksPath)
        || isParamsFile(fileName, b, o.paramsPath);
}

function walk(dir, out) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        if (entry.name.startsWith('.') || entry.name === 'node_modules') continue;
        const full = path.posix.join(dir.replace(/\\/g, '/'), entry.name);
        if (entry.isDirectory()) walk(full, out);
        else if (entry.isFile()) out.push(full);
    }
    return out;
}

const root = process.env.FIXTURE_ROOT;
const all = walk(root, []);
const relevant = all.filter(f =>
    f.endsWith('.svelte') ||
    ((f.endsWith('.ts') || f.endsWith('.js')) && isKitFile(f, defaults))
);
relevant.sort();
process.stdout.write(JSON.stringify(relevant));
"#;

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn make_fixture(root: &Path, files: &[&str]) {
    for rel in files {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create dir");
        }
        fs::write(&p, "").expect("write fixture file");
    }
}

fn run_upstream_enum(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("node")
        .arg("-e")
        .arg(UPSTREAM_ENUM_JS)
        .env("FIXTURE_ROOT", root)
        .output()
        .expect("spawn node");
    assert!(
        output.status.success(),
        "node failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let paths: Vec<String> = serde_json::from_str(&stdout).expect("parse json output");
    paths.into_iter().map(PathBuf::from).collect()
}

fn run_ours_enum(root: &Path) -> Vec<PathBuf> {
    // Invoke the binary's `--list-relevant` debug flag so we exercise the
    // exact code path the real run uses. Output format: one absolute
    // path per line.
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let output = Command::new(bin)
        .arg("--list-relevant")
        .arg("--workspace")
        .arg(root)
        .output()
        .expect("spawn ours");
    assert!(
        output.status.success(),
        "ours failed (status {:?}): {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut paths: Vec<PathBuf> = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();
    paths.sort();
    paths
}

#[test]
fn matches_upstream_on_canonical_sveltekit_layout() {
    if !node_available() {
        eprintln!("skipping kit_file_parity: `node` not on PATH");
        return;
    }

    // Prefix MUST NOT start with a dot — our walker and upstream's both
    // skip dot-prefixed dirs (.git, .svelte-kit, .tmp-foo, ...), so the
    // workspace ROOT can't be dot-prefixed either. `tempfile::tempdir()`
    // defaults to `.tmpXXXX` which would make both tools skip everything
    // under it. Use an explicit non-dot prefix.
    let tmp = tempfile::Builder::new()
        .prefix("kit_parity")
        .tempdir()
        .expect("tempdir");
    let root = tmp.path();

    // Canonical SvelteKit app tree with every Kit-file category +
    // distractors that MUST NOT get counted.
    make_fixture(
        root,
        &[
            // .svelte files — always counted
            "src/routes/+page.svelte",
            "src/routes/+layout.svelte",
            "src/routes/about/+page.svelte",
            "src/lib/components/Button.svelte",
            // Kit route files — all 5 variants + route group suffix
            "src/routes/+page.ts",
            "src/routes/+layout.ts",
            "src/routes/+page.server.ts",
            "src/routes/+layout.server.ts",
            "src/routes/api/+server.ts",
            "src/routes/(auth)/+layout@default.ts",
            // Hooks — extension form + dir-index form
            "src/hooks.server.ts",
            "src/hooks.client.ts",
            "src/hooks.ts",
            // Params
            "src/params/videoId.ts",
            "src/params/channelId.js",
            // Distractors (these must NOT be counted)
            "src/lib/util.ts",
            "src/lib/types.ts",
            "src/app.d.ts",
            "src/params/videoId.test.ts",
            "src/params/videoId.spec.ts",
            "src/routes/helper.ts",
            "src/routes/page.ts", // no leading `+`
            "README.md",
            "package.json",
            "vite.config.ts",
        ],
    );

    let mut upstream = run_upstream_enum(root);
    upstream.sort();
    let ours = run_ours_enum(root);

    // Normalize to the workspace-relative form so the comparison isn't
    // thrown off by realpath differences between node (which we hand
    // the un-canonicalized tempdir path) and our binary (which
    // canonicalizes the workspace at startup).
    let norm = |set: Vec<PathBuf>| -> Vec<String> {
        let mut out: Vec<String> = set
            .into_iter()
            .filter_map(|p| {
                // Strip until we hit "src/" — every fixture path has it.
                let s = p.to_string_lossy().to_string();
                s.find("src/").map(|i| s[i..].to_string())
            })
            .collect();
        out.sort();
        out.dedup();
        out
    };
    let upstream_n = norm(upstream);
    let ours_n = norm(ours);

    assert_eq!(
        upstream_n, ours_n,
        "Kit-file enumeration diverged from upstream svelte-check.\n\
         upstream-only: {:?}\n\
         ours-only: {:?}",
        upstream_n
            .iter()
            .filter(|f| !ours_n.contains(f))
            .collect::<Vec<_>>(),
        ours_n
            .iter()
            .filter(|f| !upstream_n.contains(f))
            .collect::<Vec<_>>(),
    );
}
