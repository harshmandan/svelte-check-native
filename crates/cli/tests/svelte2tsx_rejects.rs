//! Files svelte2tsx rejects are left out of the type check.
//!
//! A rejected file is listed in its emit-snapshot sample's
//! `upstream.error.txt` as `file: message`. Upstream svelte-check catches
//! the rejection, writes no overlay and drops the file's entry, so nothing
//! of it is type-checked and it is not counted. The emit-snapshot
//! comparison skips such files, since there is no upstream emit to
//! compare; this checks the other half, that our check leaves each one out
//! the same way.
//!
//! This runs a real check, so it needs tsgo in `node_modules` — which is
//! why it lives here and not in the tsgo-free `emit_snapshots` suite.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Every rejected file, as `(display name, absolute input path)`, read
/// from the `upstream.error.txt` files beside the emit snapshots. Each
/// snapshot corpus maps to the directory its sample inputs live in.
fn rejected_files(crate_dir: &Path) -> Vec<(String, PathBuf)> {
    let snapshots_root = crate_dir.join("tests/emit_snapshots");
    let corpora = [
        (
            "svelte2tsx",
            "../../language-tools/packages/svelte2tsx/test/svelte2tsx/samples",
        ),
        (
            "htmlx2jsx",
            "../../language-tools/packages/svelte2tsx/test/htmlx2jsx/samples",
        ),
        ("bugs", "../../fixtures/bugs"),
    ];
    let mut out = Vec::new();
    for (corpus, inputs) in corpora {
        let inputs_root = crate_dir.join(inputs).canonicalize().unwrap_or_else(|e| {
            panic!(
                "{corpus} inputs missing at {inputs} ({e}) — run \
                 `git submodule update --init --recursive`"
            )
        });
        let mut samples: Vec<PathBuf> = std::fs::read_dir(snapshots_root.join(corpus))
            .expect("read snapshot corpus")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        samples.sort();
        for sample in samples {
            let Ok(list) = std::fs::read_to_string(sample.join("upstream.error.txt")) else {
                continue;
            };
            let name = sample.file_name().unwrap().to_string_lossy().into_owned();
            for line in list.lines() {
                let Some((rel, _)) = line.split_once(": ") else {
                    continue;
                };
                out.push((
                    format!("{corpus}-{name}-{rel}").replace('/', "-"),
                    inputs_root.join(&name).join(rel),
                ));
            }
        }
    }
    out
}

/// All rejected files go into one workspace beside one component that is
/// accepted, so a single run covers them; the run must count that one
/// file and report no type-check diagnostic. Enhancements are off: they
/// deliberately report more than upstream's `--tsgo` mode for a rejected
/// file another component imports.
#[test]
fn files_svelte2tsx_rejects_are_left_out_of_the_check() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("rejected-by-svelte2tsx");
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::write(workspace.join("tsconfig.json"), "{}\n").expect("write tsconfig");
    std::fs::write(
        workspace.join("accepted.svelte"),
        "<script lang=\"ts\">\n  let count: number = 1;\n</script>\n\n{count}\n",
    )
    .expect("write accepted component");

    let mut rejected = Vec::new();
    for (copy, input) in rejected_files(&crate_dir) {
        std::fs::copy(&input, workspace.join(&copy))
            .unwrap_or_else(|e| panic!("copy {}: {e}", input.display()));
        rejected.push(copy);
    }
    assert!(
        !rejected.is_empty(),
        "no upstream.error.txt lists a file; run node scripts/gen-upstream-emit.mjs"
    );

    let out = Command::new(bin)
        .arg("--workspace")
        .arg(&workspace)
        .args(["--output", "machine-verbose", "--disable-enhance"])
        // The binary switches its output format when it sees an agent's
        // environment; this test parses machine output either way.
        .env("CLAUDECODE", "")
        .env("GEMINI_CLI", "")
        .env("CODEX_CI", "")
        .output()
        .expect("run the check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let type_checked: Vec<&str> = stdout
        .lines()
        .filter(|l| l.contains(r#""source":"ts""#) || l.contains(r#""source":"js""#))
        .collect();
    let completed = stdout
        .lines()
        .find(|l| l.contains(" COMPLETED "))
        .unwrap_or_default();
    assert!(
        out.status.success()
            && type_checked.is_empty()
            && completed.contains(" COMPLETED 1 FILES "),
        "the check must leave out every file svelte2tsx rejects ({}), counting only \
         accepted.svelte and type-checking none of them; got exit {:?}\n{stdout}{stderr}",
        rejected.join(", "),
        out.status.code(),
    );
}
