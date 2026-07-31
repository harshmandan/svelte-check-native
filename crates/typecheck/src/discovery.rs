//! Locate the native TypeScript compiler (TypeScript 7 `tsc` / tsgo).
//!
//! Two npm packages ship it: stable `typescript` 7+ (the supported
//! release channel going forward) and `@typescript/native-preview`
//! (tsgo, the nightly channel). Both are supported; when both are
//! installed, **stable `typescript` 7+ wins**, with tsgo as the
//! fallback — including when `typescript` is installed but below 7.
//!
//! That version gate is load-bearing: at 6 and below the `typescript`
//! package's `bin/tsc` is the JavaScript compiler, not the native
//! one, and nearly every workspace has some `typescript` installed
//! for its own toolchain. Without the gate we'd spawn classic `tsc`
//! in exactly the projects that installed tsgo to avoid it — the gate
//! is what lets a project keep `typescript@6` for its build while
//! type-checking with tsgo.
//!
//! Resolution order at each ancestor `node_modules`, closest first —
//! engine preference dominates, and within each engine the
//! platform-native binary beats the JS wrapper (skips Node.js startup
//! overhead, ~50-100 ms per check):
//! 1. `TSGO_BIN` env var (absolute path to an executable or wrapper).
//! 2. Stable TypeScript 7+ —
//!    `node_modules/@typescript/typescript-<platform>/lib/tsc[.exe]`
//!    (the scoped platform packages only exist for 7+, so no version
//!    gate is needed), then `node_modules/typescript/bin/tsc` gated
//!    on the installed package's `version` being 7+, then the same
//!    wrapper at `node_modules/@typescript/native/bin/tsc` — the
//!    npm-alias convention (`@typescript/native@npm:typescript@7`)
//!    for projects that keep `typescript@6` as their own toolchain.
//! 3. tsgo —
//!    `node_modules/@typescript/native-preview-<platform>/lib/tsgo[.exe]`,
//!    then `node_modules/@typescript/native-preview/bin/tsgo.js`.
//!
//! A wrapper hit at step 2 or 3 is upgraded to the platform-native
//! binary sitting beside the wrapper's *canonical* location when one
//! exists — symlinked stores (pnpm, bun) hoist only the wrapper
//! package, keeping the version-matched platform package next to it
//! inside the store entry.
//! 4. pnpm / bun per-package-store fallbacks:
//!    `node_modules/.pnpm/<pkg>@*/node_modules/...` and
//!    `node_modules/.bun/<pkg>@*/node_modules/...`, same engine
//!    preference and the same 7+ gate on `typescript@*` entries.
//!    Needed when `shamefully-hoist=false` (default in pnpm 8+) or
//!    `symlink=false` prevents the hoisted paths above from existing.
//!    When multiple versions are installed, the highest semver wins.
//!
//! Both packages ship the JS wrapper and install a platform-specific
//! package as an optionalDependency containing the real binary. We
//! invoke the native form when one is present and fall back to the
//! wrapper otherwise (e.g. on a platform with no native package).
//!
//! Returns a [`TsgoBinary`] handle that the runner uses to spawn the
//! correct command (a `.js` wrapper has to be invoked via `node`; a native
//! binary is invoked directly).

use std::path::{Path, PathBuf};

/// A located TypeScript compiler binary, ready to spawn.
#[derive(Debug, Clone)]
pub struct TsgoBinary {
    /// Path to the executable or JS wrapper.
    pub path: PathBuf,
    /// Whether the path is a JavaScript file that must be run via `node`.
    pub needs_node: bool,
}

/// Errors when looking for TypeScript.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error(
        "could not find a native TypeScript compiler. Install \
         `typescript` (7+) or `@typescript/native-preview` as a \
         devDependency, or set TSGO_BIN to an absolute path. Searched \
         upward from {searched_from} (hoisted under node_modules, or \
         isolated under .pnpm/.bun). TypeScript 6 and below is not \
         supported as a check engine."
    )]
    NotFound { searched_from: PathBuf },

    #[error("TSGO_BIN points at {path} which does not exist")]
    EnvBinNotFound { path: PathBuf },
}

/// Locate the native TypeScript compiler — stable `typescript` 7+
/// preferred, `@typescript/native-preview` (tsgo) as the fallback.
pub fn discover(workspace: &Path) -> Result<TsgoBinary, DiscoveryError> {
    if let Ok(env_path) = std::env::var("TSGO_BIN") {
        let path = PathBuf::from(env_path);
        if !path.exists() {
            return Err(DiscoveryError::EnvBinNotFound { path });
        }
        let needs_node = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| {
                ext.eq_ignore_ascii_case("js")
                    || ext.eq_ignore_ascii_case("cjs")
                    || ext.eq_ignore_ascii_case("mjs")
            })
            .unwrap_or(false);
        return Ok(TsgoBinary { path, needs_node });
    }

    // Stable 7+ before preview (tsgo); native before wrapper within
    // each engine — see module docs.
    let native_relatives = [
        stable_platform_native_path(),
        preview_platform_native_path(),
    ];

    svn_core::walk_up_dirs(workspace, |dir| {
        // Stable TypeScript 7+.
        if let Some(rel) = &native_relatives[0] {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Some(TsgoBinary {
                    path: candidate,
                    needs_node: false,
                });
            }
        }
        // Canonical `typescript` first, then the `@typescript/native`
        // npm-alias convention for projects that pin `typescript` at 6
        // and install the 7 engine under the alias.
        for pkg_dir in ["typescript", "@typescript/native"] {
            if let Some(found) = stable_typescript_wrapper(dir, pkg_dir) {
                // Same store-sibling preference as the preview wrapper
                // below: spawn the version-matched native binary
                // directly when the wrapper's canonical location has
                // one.
                if let Some(native) =
                    native_beside_wrapper(&found.path, native_relatives[0].as_ref())
                {
                    return Some(TsgoBinary {
                        path: native,
                        needs_node: false,
                    });
                }
                return Some(found);
            }
        }
        // tsgo (native-preview) fallback.
        if let Some(rel) = &native_relatives[1] {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Some(TsgoBinary {
                    path: candidate,
                    needs_node: false,
                });
            }
        }
        let preview_wrapper = dir.join("node_modules/@typescript/native-preview/bin/tsgo.js");
        if preview_wrapper.is_file() {
            // Symlinked-store installs (pnpm, bun) hoist only the
            // wrapper package; its platform-native sibling lives next
            // to the wrapper's canonical location in the store.
            if let Some(native) =
                native_beside_wrapper(&preview_wrapper, native_relatives[1].as_ref())
            {
                return Some(TsgoBinary {
                    path: native,
                    needs_node: false,
                });
            }
            return Some(TsgoBinary {
                path: preview_wrapper,
                needs_node: true,
            });
        }
        // pnpm / bun per-package store. Only reached when the
        // canonical hoisted paths above are absent (pnpm
        // `shamefully-hoist=false`, isolated installs, etc.).
        find_in_package_store(dir, &native_relatives)
    })
    .ok_or_else(|| DiscoveryError::NotFound {
        searched_from: workspace.to_path_buf(),
    })
}

/// The stable `typescript` package's `bin/tsc` wrapper — accepted only
/// when the installed package's `version` is 7 or newer. At 6 and
/// below the same path holds the JavaScript compiler: spawning it
/// would silently swap the check engine in any workspace that happens
/// to have `typescript` installed for its own build tooling, which is
/// most of them. A `typescript` install whose package.json is missing
/// or unparseable is skipped for the same reason — we only ever spawn
/// a wrapper we can positively identify as the native compiler.
///
/// `pkg_dir` is the install directory under `node_modules` — the
/// canonical `typescript`, or the `@typescript/native` npm-alias
/// convention (`npm i typescript@~6 @typescript/native@npm:typescript@7`)
/// that keeps TypeScript 6 as the project's own `typescript` while
/// installing the 7 engine under the alias. An alias install is the
/// real `typescript` package on disk, so the same layout and version
/// gate apply verbatim.
fn stable_typescript_wrapper(dir: &Path, pkg_dir: &str) -> Option<TsgoBinary> {
    let wrapper = dir.join("node_modules").join(pkg_dir).join("bin/tsc");
    if !wrapper.is_file() {
        return None;
    }
    let manifest = dir.join("node_modules").join(pkg_dir).join("package.json");
    let text = std::fs::read_to_string(manifest).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&text).ok()?;
    let version = semver::Version::parse(pkg.get("version")?.as_str()?).ok()?;
    if version.major < 7 {
        return None;
    }
    Some(TsgoBinary {
        path: wrapper,
        needs_node: true,
    })
}

/// Look for stable `typescript@<version>` and
/// `@typescript+native-preview@<version>` directories under
/// `<dir>/node_modules/.pnpm` and `<dir>/node_modules/.bun`. Returns
/// the highest-version match's native binary (preferred) or JS wrapper
/// (fallback). Stable TypeScript is preferred over native-preview,
/// and `typescript@*` entries below major 7 are ignored entirely —
/// same engine rules as the hoisted paths (see module docs).
///
/// pnpm store layout (shamefully-hoist=false or symlink=false):
///
/// ```text
/// node_modules/.pnpm/
///   typescript@7.0.2/
///     node_modules/@typescript/
///       typescript-darwin-arm64/lib/tsc
///     typescript/bin/tsc
/// ```
///
/// bun uses the same layout under `.bun/`.
fn find_in_package_store(
    dir: &Path,
    native_relatives: &[Option<PathBuf>; 2],
) -> Option<TsgoBinary> {
    for manager_root in [
        dir.join(svn_core::NODE_MODULES_DIR).join(".pnpm"),
        dir.join(svn_core::NODE_MODULES_DIR).join(".bun"),
    ] {
        for (prefix, wrapper_tail) in [
            ("typescript@", "typescript/bin/tsc"),
            (
                "@typescript+native-preview@",
                "@typescript/native-preview/bin/tsgo.js",
            ),
        ] {
            let Ok(entries) = std::fs::read_dir(&manager_root) else {
                continue;
            };
            // Collect every version in this package family and sort
            // newest-first by parsed semver. Lexicographic sorting would
            // mis-order a multi-digit version or preview suffix.
            // `typescript@*` entries carry the 7+ engine gate: below
            // that, `bin/tsc` is the JavaScript compiler.
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| name.starts_with(prefix))
                })
                .filter(|p| {
                    prefix != "typescript@"
                        || version_from_store_entry(p).is_some_and(|v| v.major >= 7)
                })
                .collect();
            candidates.sort_by(|a, b| {
                let va = version_from_store_entry(a);
                let vb = version_from_store_entry(b);
                match (va, vb) {
                    (Some(va), Some(vb)) => vb.cmp(&va),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => b.cmp(a),
                }
            });

            for pkg_root in candidates {
                // Native binary under the sibling platform package inside
                // the same store entry — prefer this form when present.
                for rel in native_relatives.iter().flatten() {
                    if let Ok(tail) = rel.strip_prefix("node_modules/") {
                        let native_candidate = pkg_root.join(svn_core::NODE_MODULES_DIR).join(tail);
                        if native_candidate.is_file() {
                            return Some(TsgoBinary {
                                path: native_candidate,
                                needs_node: false,
                            });
                        }
                    }
                }
                let wrapper_candidate =
                    pkg_root.join(svn_core::NODE_MODULES_DIR).join(wrapper_tail);
                if wrapper_candidate.is_file() {
                    return Some(TsgoBinary {
                        path: wrapper_candidate,
                        needs_node: true,
                    });
                }
            }
        }
    }
    None
}

/// Extract and parse the version from a pnpm/bun store directory name.
///
/// Store entries are named `typescript@<version>` or
/// `@typescript+native-preview@<version>`. When the version is valid semver
/// (including tsgo's own `7.0.0-dev.YYYYMMDD.N` form), the
/// parsed [`Version`](semver::Version) sorts correctly by semver rules —
/// `1.0.0-dev.10` > `1.0.0-dev.9` (numeric identifier compare) and
/// `10.0.0` > `9.0.0`. Returns `None` for entries that don't match the
/// expected name or whose version doesn't parse.
fn version_from_store_entry(path: &Path) -> Option<semver::Version> {
    let name = path.file_name().and_then(|s| s.to_str())?;
    let ver = name
        .strip_prefix("typescript@")
        .or_else(|| name.strip_prefix("@typescript+native-preview@"))?;
    semver::Version::parse(ver).ok()
}

/// npm platform tag for the running host, or `None` on platforms
/// where neither package publishes a native binary we know about.
///
/// Mapping mirrors the platform tags used by the npm packages:
/// - `darwin` + `aarch64` → `darwin-arm64`
/// - `darwin` + `x86_64`  → `darwin-x64`
/// - `linux`  + `aarch64` → `linux-arm64`
/// - `linux`  + `x86_64`  → `linux-x64`
/// - `windows`+ `x86_64`  → `win32-x64` (binary suffixed `.exe`)
fn platform_tag() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-arm64"),
        ("macos", "x86_64") => Some("darwin-x64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("linux", "x86_64") => Some("linux-x64"),
        ("windows", "x86_64") => Some("win32-x64"),
        _ => None,
    }
}

/// Resolve the platform-native binary that a JS wrapper would itself
/// exec into, by mirroring node's module resolution from the wrapper's
/// real location: canonicalize the wrapper (symlinked stores point
/// `node_modules/<pkg>` into `.pnpm`/`.bun`), walk up to the nearest
/// `node_modules` ancestor, and look for the platform package there.
/// That is where the store keeps the version-matched sibling
/// (`.bun/<pkg>@<v>/node_modules/@typescript/<pkg>-<platform>/…`), so
/// this never mixes wrapper and binary from different versions. Spawning
/// the native binary directly skips a Node.js startup (~40-80ms per
/// run) the wrapper would spend before `execve`-ing into the same
/// binary. `native_rel` is a `node_modules/…`-anchored relative path
/// from the `*_platform_native_path` helpers. Returns `None` when no
/// native sibling exists (platform without a native package) — the
/// caller falls back to spawning the wrapper via `node`.
fn native_beside_wrapper(wrapper: &Path, native_rel: Option<&PathBuf>) -> Option<PathBuf> {
    let tail = native_rel?.strip_prefix("node_modules").ok()?;
    let canonical = wrapper.canonicalize().ok()?;
    // Strip `bin/<wrapper-file>` to reach the package dir, then walk
    // up to the `node_modules` dir the package is installed in (one
    // level for `typescript`, two for scoped `@typescript/*`).
    let pkg_dir = canonical.parent()?.parent()?;
    let mut dir = pkg_dir.parent()?;
    for _ in 0..2 {
        if dir.file_name().is_some_and(|n| n == "node_modules") {
            let candidate = dir.join(tail);
            return candidate.is_file().then_some(candidate);
        }
        dir = dir.parent()?;
    }
    None
}

/// Relative path to `@typescript/native-preview-<platform>`'s tsgo
/// binary (the preferred engine — see module docs).
fn preview_platform_native_path() -> Option<PathBuf> {
    let tag = platform_tag()?;
    let exe = if cfg!(windows) { "tsgo.exe" } else { "tsgo" };
    Some(
        PathBuf::from("node_modules/@typescript")
            .join(format!("native-preview-{tag}"))
            .join("lib")
            .join(exe),
    )
}

/// Relative path to `@typescript/typescript-<platform>`'s native `tsc`
/// binary — stable TypeScript's layout since 7.0. The scoped platform
/// packages only exist for 7+, so unlike the `typescript` JS wrapper
/// this path needs no version gate.
fn stable_platform_native_path() -> Option<PathBuf> {
    let tag = platform_tag()?;
    let exe = if cfg!(windows) { "tsc.exe" } else { "tsc" };
    Some(
        PathBuf::from("node_modules/@typescript")
            .join(format!("typescript-{tag}"))
            .join("lib")
            .join(exe),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_tsgo_in_local_node_modules() {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let tsgo = bin_dir.join("tsgo.js");
        fs::write(&tsgo, "// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, tsgo);
        assert!(found.needs_node);
    }

    /// Write a stub `typescript` package (wrapper + package.json at the
    /// given version) under `root/node_modules` and return the wrapper
    /// path.
    fn write_typescript_pkg(root: &Path, version: &str) -> PathBuf {
        let pkg_dir = root.join("node_modules/typescript");
        let wrapper = pkg_dir.join("bin/tsc");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "#!/usr/bin/env node\n").unwrap();
        fs::write(
            pkg_dir.join("package.json"),
            format!(r#"{{"name":"typescript","version":"{version}"}}"#),
        )
        .unwrap();
        wrapper
    }

    #[test]
    fn discovers_typescript_wrapper_in_local_node_modules() {
        let tmp = tempdir().unwrap();
        let wrapper = write_typescript_pkg(tmp.path(), "7.0.2");

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper);
        assert!(found.needs_node);
    }

    #[test]
    fn typescript_6_wrapper_is_not_an_engine() {
        // `typescript@6`'s bin/tsc is the JavaScript compiler. A
        // workspace that has it installed for its own tooling (most
        // do) must not have it picked up as the check engine.
        let tmp = tempdir().unwrap();
        write_typescript_pkg(tmp.path(), "6.1.0");

        let err = discover(tmp.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotFound { .. }));
    }

    #[test]
    fn typescript_wrapper_without_manifest_is_skipped() {
        // No package.json → can't positively identify the wrapper as
        // the native compiler → skip rather than guess.
        let tmp = tempdir().unwrap();
        let wrapper = tmp.path().join("node_modules/typescript/bin/tsc");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "#!/usr/bin/env node\n").unwrap();

        let err = discover(tmp.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotFound { .. }));
    }

    #[test]
    fn typescript_7_preferred_over_preview_wrapper() {
        // Both engines installed side by side: stable `typescript` 7+
        // wins — it's the supported release channel; tsgo is the
        // fallback for projects that haven't moved yet.
        let tmp = tempdir().unwrap();
        let tsc = write_typescript_pkg(tmp.path(), "7.0.2");
        let tsgo = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(tsgo.parent().unwrap()).unwrap();
        fs::write(&tsgo, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, tsc);
    }

    #[test]
    fn typescript_6_alongside_preview_uses_preview() {
        // The exact mixed setup the gate exists for: TypeScript 6 for
        // the project's own toolchain, tsgo for checking.
        let tmp = tempdir().unwrap();
        write_typescript_pkg(tmp.path(), "6.0.4");
        let tsgo = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(tsgo.parent().unwrap()).unwrap();
        fs::write(&tsgo, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, tsgo);
    }

    #[test]
    fn discovers_tsgo_in_ancestor_node_modules() {
        let tmp = tempdir().unwrap();
        let bin_dir = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let tsgo = bin_dir.join("tsgo.js");
        fs::write(&tsgo, "// stub").unwrap();

        let nested = tmp.path().join("apps/inner");
        fs::create_dir_all(&nested).unwrap();

        let found = discover(&nested).unwrap();
        assert_eq!(found.path, tsgo);
    }

    #[test]
    fn typescript_native_alias_used_when_typescript_is_6() {
        // The npm-alias convention: `typescript@6` stays as the
        // project's own toolchain, the 7 engine installs under
        // `@typescript/native` (`@typescript/native@npm:typescript@7`
        // puts the real typescript package at that path).
        let tmp = tempdir().unwrap();
        write_typescript_pkg(tmp.path(), "6.0.4");
        let alias_dir = tmp.path().join("node_modules/@typescript/native");
        let alias_wrapper = alias_dir.join("bin/tsc");
        fs::create_dir_all(alias_wrapper.parent().unwrap()).unwrap();
        fs::write(&alias_wrapper, "#!/usr/bin/env node\n").unwrap();
        fs::write(
            alias_dir.join("package.json"),
            r#"{"name":"typescript","version":"7.0.2"}"#,
        )
        .unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, alias_wrapper);
        assert!(found.needs_node);
    }

    #[test]
    fn typescript_native_alias_below_7_is_skipped() {
        // An alias install that itself resolves below 7 is the JS
        // compiler — same gate as the canonical `typescript` path.
        let tmp = tempdir().unwrap();
        let alias_dir = tmp.path().join("node_modules/@typescript/native");
        let alias_wrapper = alias_dir.join("bin/tsc");
        fs::create_dir_all(alias_wrapper.parent().unwrap()).unwrap();
        fs::write(&alias_wrapper, "#!/usr/bin/env node\n").unwrap();
        fs::write(
            alias_dir.join("package.json"),
            r#"{"name":"typescript","version":"6.1.0"}"#,
        )
        .unwrap();

        assert!(matches!(
            discover(tmp.path()),
            Err(DiscoveryError::NotFound { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_store_wrapper_upgrades_to_native_sibling() {
        // Symlinked-store layout (pnpm, bun): the hoisted wrapper
        // package is a symlink into the store, and the version-matched
        // platform package sits beside the wrapper's canonical
        // location. Discovery should spawn that native binary, not the
        // wrapper.
        let Some(rel) = preview_platform_native_path() else {
            return;
        };
        let tmp = tempdir().unwrap();
        let store_entry = tmp
            .path()
            .join("node_modules/.bun/@typescript+native-preview@7.0.0-dev.1");
        let canonical_pkg = store_entry.join("node_modules/@typescript/native-preview");
        fs::create_dir_all(canonical_pkg.join("bin")).unwrap();
        fs::write(canonical_pkg.join("bin/tsgo.js"), "// stub").unwrap();
        let native = store_entry.join(&rel);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"\x7fELF stub").unwrap();

        let scope_dir = tmp.path().join("node_modules/@typescript");
        fs::create_dir_all(&scope_dir).unwrap();
        std::os::unix::fs::symlink(&canonical_pkg, scope_dir.join("native-preview")).unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(
            found.path.canonicalize().unwrap(),
            native.canonicalize().unwrap()
        );
        assert!(!found.needs_node);
    }

    #[test]
    fn errors_when_not_installed() {
        let tmp = tempdir().unwrap();
        let err = discover(tmp.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotFound { .. }));
    }

    #[test]
    fn native_binary_preferred_over_js_wrapper() {
        // When both forms are present, the native binary wins (saves Node
        // startup overhead).
        let Some(rel) = stable_platform_native_path() else {
            // Skip on platforms we don't have a native package for.
            return;
        };

        let tmp = tempdir().unwrap();
        let native_path = tmp.path().join(&rel);
        fs::create_dir_all(native_path.parent().unwrap()).unwrap();
        fs::write(&native_path, b"\x7fELF stub").unwrap();

        let wrapper_path = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(wrapper_path.parent().unwrap()).unwrap();
        fs::write(&wrapper_path, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, native_path);
        assert!(!found.needs_node);
    }

    #[test]
    fn falls_back_to_wrapper_when_no_native_binary() {
        // Wrapper-only install (e.g. an unsupported platform with no
        // platform-specific package shipped).
        let tmp = tempdir().unwrap();
        let wrapper_path = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(wrapper_path.parent().unwrap()).unwrap();
        fs::write(&wrapper_path, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper_path);
        assert!(found.needs_node);
    }

    #[test]
    fn stable_platform_native_path_returns_some_for_known_platforms() {
        // Smoke test: on the platforms our CI matrix covers, the helper
        // should return a path; on unknown platforms returning None is
        // also valid behavior.
        let _ = stable_platform_native_path();
    }

    // Note: we intentionally don't have a unit test for the `TSGO_BIN`
    // env-var path. cargo test runs threads in parallel within a binary;
    // mutating process-global env vars is unsound (Rust 2024 marks
    // std::env::set_var unsafe for that reason). The env-var path is
    // covered by integration tests that spawn fresh subprocesses.

    #[test]
    fn discovers_tsgo_in_pnpm_package_store_wrapper() {
        // pnpm with shamefully-hoist=false: the hoisted
        // `node_modules/@typescript/native-preview/` symlink is
        // absent; tsgo lives under `.pnpm/@typescript+native-preview@X/`.
        let tmp = tempdir().unwrap();
        let pkg_root = tmp
            .path()
            .join("node_modules/.pnpm/@typescript+native-preview@7.0.0-dev.20260101.1");
        let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper);
        assert!(found.needs_node);
    }

    #[test]
    fn discovers_typescript_in_pnpm_package_store_wrapper() {
        let tmp = tempdir().unwrap();
        let pkg_root = tmp.path().join("node_modules/.pnpm/typescript@7.0.2");
        let wrapper = pkg_root.join("node_modules/typescript/bin/tsc");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "#!/usr/bin/env node\n").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper);
        assert!(found.needs_node);
    }

    #[test]
    fn discovers_typescript_native_binary_in_pnpm_package_store() {
        let Some(rel) = stable_platform_native_path() else {
            return;
        };
        let tmp = tempdir().unwrap();
        let pkg_root = tmp.path().join("node_modules/.pnpm/typescript@7.0.2");
        let tail = rel.strip_prefix("node_modules/").unwrap();
        let native = pkg_root.join("node_modules").join(tail);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"native stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, native);
        assert!(!found.needs_node);
    }

    #[test]
    fn pnpm_store_picks_highest_version() {
        // Multiple versions in the same store; newest by semver wins
        // (matches newest dev-release).
        let tmp = tempdir().unwrap();
        for version in ["7.0.0-dev.20260101.1", "7.0.0-dev.20260201.1"] {
            let pkg_root = tmp.path().join(format!(
                "node_modules/.pnpm/@typescript+native-preview@{version}"
            ));
            let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
            fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
            fs::write(&wrapper, b"// stub").unwrap();
        }
        let found = discover(tmp.path()).unwrap();
        assert!(
            found
                .path
                .to_string_lossy()
                .contains("@typescript+native-preview@7.0.0-dev.20260201.1"),
            "expected newest version, got {:?}",
            found.path,
        );
    }

    #[test]
    fn pnpm_store_picks_dev_suffix_10_over_9() {
        // Regression for the lexicographic-sort bug: under string
        // compare, `...9` beat `...10` because '9' > '1' byte-wise,
        // silently downgrading users to an older tsgo. Semver-aware
        // compare treats dev-release trailing identifiers as numeric.
        let tmp = tempdir().unwrap();
        for version in ["7.0.0-dev.20260101.9", "7.0.0-dev.20260101.10"] {
            let pkg_root = tmp.path().join(format!(
                "node_modules/.pnpm/@typescript+native-preview@{version}"
            ));
            let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
            fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
            fs::write(&wrapper, b"// stub").unwrap();
        }
        let found = discover(tmp.path()).unwrap();
        assert!(
            found.path.to_string_lossy().contains("20260101.10"),
            "expected .10 (newer) to win over .9, got {:?}",
            found.path,
        );
    }

    #[test]
    fn pnpm_store_picks_major_10_over_major_9() {
        // Same class of bug on the major-version axis. If tsgo ever
        // ships 10.x alongside 9.x, string compare picks 9.x (because
        // '9' > '1' byte-wise). Semver compares numerically.
        let tmp = tempdir().unwrap();
        for version in ["9.0.0", "10.0.0"] {
            let pkg_root = tmp.path().join(format!(
                "node_modules/.pnpm/@typescript+native-preview@{version}"
            ));
            let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
            fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
            fs::write(&wrapper, b"// stub").unwrap();
        }
        let found = discover(tmp.path()).unwrap();
        assert!(
            found.path.to_string_lossy().contains("@10.0.0"),
            "expected 10.0.0 to win over 9.0.0, got {:?}",
            found.path,
        );
    }

    #[test]
    fn pnpm_store_ignores_unparseable_entry_and_picks_real_version() {
        // A malformed or future-format store entry shouldn't shadow a
        // real one. Unparseable entries sort last.
        let tmp = tempdir().unwrap();
        for suffix in ["not-a-version", "7.0.0-dev.20260101.1"] {
            let pkg_root = tmp.path().join(format!(
                "node_modules/.pnpm/@typescript+native-preview@{suffix}"
            ));
            let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
            fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
            fs::write(&wrapper, b"// stub").unwrap();
        }
        let found = discover(tmp.path()).unwrap();
        assert!(
            found.path.to_string_lossy().contains("7.0.0-dev"),
            "expected the parseable version to win, got {:?}",
            found.path,
        );
    }

    #[test]
    fn pnpm_store_prefers_native_binary_when_present() {
        // Store entry has BOTH the JS wrapper and a platform-native
        // binary. Native wins.
        let Some(rel) = preview_platform_native_path() else {
            return;
        };
        let tmp = tempdir().unwrap();
        let pkg_root = tmp
            .path()
            .join("node_modules/.pnpm/@typescript+native-preview@7.0.0-dev.20260101.1");
        // native_relative is rooted at `node_modules/...` — strip
        // the prefix and attach under the store entry's own
        // `node_modules/`.
        let tail = rel.strip_prefix("node_modules/").unwrap();
        let native = pkg_root.join("node_modules").join(tail);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"\x7fELF stub").unwrap();

        let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, native);
        assert!(!found.needs_node);
    }

    #[test]
    fn pnpm_store_ignores_typescript_below_7() {
        // `.pnpm/typescript@5.x` is the JavaScript compiler — same
        // engine gate as the hoisted layout.
        let tmp = tempdir().unwrap();
        let wrapper = tmp
            .path()
            .join("node_modules/.pnpm/typescript@5.8.3")
            .join("node_modules/typescript/bin/tsc");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "#!/usr/bin/env node\n").unwrap();

        let err = discover(tmp.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotFound { .. }));
    }

    #[test]
    fn pnpm_store_prefers_typescript_7_over_preview() {
        // Both engines in the store: stable `typescript` 7+ wins, same
        // preference as the hoisted layout.
        let tmp = tempdir().unwrap();
        let ts_wrapper = tmp
            .path()
            .join("node_modules/.pnpm/typescript@7.0.2")
            .join("node_modules/typescript/bin/tsc");
        fs::create_dir_all(ts_wrapper.parent().unwrap()).unwrap();
        fs::write(&ts_wrapper, "#!/usr/bin/env node\n").unwrap();

        let tsgo = tmp
            .path()
            .join("node_modules/.pnpm/@typescript+native-preview@7.0.0-dev.20260101.1")
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(tsgo.parent().unwrap()).unwrap();
        fs::write(&tsgo, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, ts_wrapper);
    }

    #[test]
    fn discovers_tsgo_in_bun_package_store() {
        // bun's layout mirrors pnpm's under `.bun/` instead of
        // `.pnpm/`.
        let tmp = tempdir().unwrap();
        let pkg_root = tmp
            .path()
            .join("node_modules/.bun/@typescript+native-preview@7.0.0-dev.20260101.1");
        let wrapper = pkg_root.join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper);
        assert!(found.needs_node);
    }

    #[test]
    fn hoisted_layout_beats_package_store() {
        // Both hoisted and store paths exist (common in
        // `shamefully-hoist=true`). The hoisted layout wins so we
        // don't pay an extra readdir per ancestor when the user's
        // config doesn't need the fallback.
        let tmp = tempdir().unwrap();
        let hoisted = tmp
            .path()
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(hoisted.parent().unwrap()).unwrap();
        fs::write(&hoisted, b"// stub").unwrap();

        let store_wrapper = tmp
            .path()
            .join("node_modules/.pnpm/@typescript+native-preview@7.0.0-dev.20260101.1")
            .join("node_modules/@typescript/native-preview/bin/tsgo.js");
        fs::create_dir_all(store_wrapper.parent().unwrap()).unwrap();
        fs::write(&store_wrapper, b"// stub").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, hoisted);
    }
}
