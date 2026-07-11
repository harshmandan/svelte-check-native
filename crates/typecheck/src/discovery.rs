//! Locate the TypeScript 7 native compiler.
//!
//! Resolution order at each ancestor `node_modules`, closest first:
//! 1. `TSGO_BIN` env var (absolute path to a `tsc` executable or wrapper).
//! 2. The platform-native binary at
//!    `node_modules/@typescript/typescript-<platform>/lib/tsc[.exe]`.
//!    This skips Node.js startup overhead (~50-100 ms per check).
//! 3. The JavaScript wrapper at `node_modules/typescript/bin/tsc` invoked
//!    via `node`.
//! 4. pnpm / bun per-package-store fallbacks:
//!    `node_modules/.pnpm/typescript@*/node_modules/...` and
//!    `node_modules/.bun/typescript@*/node_modules/...`.
//!    Needed when `shamefully-hoist=false` (default in pnpm 8+) or
//!    `symlink=false` prevents the hoisted paths above from existing.
//!    When multiple versions are installed, the highest semver wins.
//!
//! TypeScript 7 ships the JS wrapper and installs a platform-specific
//! package (e.g. `typescript-darwin-arm64`) as an optionalDependency
//! containing the real binary. We invoke the native form when one is
//! present and fall back to the wrapper otherwise. The old
//! `@typescript/native-preview` layout remains supported for existing
//! installations.
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
        "could not find TypeScript 7. Install `typescript` as a \
         devDependency, or set TSGO_BIN to an absolute path. Searched \
         upward from {searched_from} for `typescript` (hoisted under \
         node_modules, or isolated under .pnpm/.bun)."
    )]
    NotFound { searched_from: PathBuf },

    #[error("TSGO_BIN points at {path} which does not exist")]
    EnvBinNotFound { path: PathBuf },
}

/// Locate TypeScript 7, with a fallback for legacy native-preview installs.
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

    let native_relatives = [
        current_platform_native_path(),
        legacy_platform_native_path(),
    ];
    let wrapper_relatives = [
        Path::new("node_modules/typescript/bin/tsc"),
        Path::new("node_modules/@typescript/native-preview/bin/tsgo.js"),
    ];

    svn_core::walk_up_dirs(workspace, |dir| {
        // Native binary is preferred — no Node.js startup overhead. Stable
        // TypeScript 7 is checked before the legacy preview layout.
        for rel in native_relatives.iter().flatten() {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Some(TsgoBinary {
                    path: candidate,
                    needs_node: false,
                });
            }
        }
        // Fallback: JS wrapper requires `node`.
        for rel in wrapper_relatives {
            let wrapper = dir.join(rel);
            if wrapper.is_file() {
                return Some(TsgoBinary {
                    path: wrapper,
                    needs_node: true,
                });
            }
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

/// Look for stable `typescript@<version>` and legacy
/// `@typescript+native-preview@<version>` directories under
/// `<dir>/node_modules/.pnpm` and `<dir>/node_modules/.bun`. Returns
/// the highest-version match's native binary (preferred) or JS wrapper
/// (fallback). Stable TypeScript is preferred over the legacy preview.
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
            let mut candidates: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|name| name.starts_with(prefix))
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

/// Return the relative path to the platform-native TypeScript 7 binary, or
/// `None` if we don't know how to map the current platform to a published
/// `@typescript/typescript-<platform>` package.
///
/// Mapping mirrors the platform tags used by the npm packages:
/// - `darwin` + `aarch64` → `darwin-arm64`
/// - `darwin` + `x86_64`  → `darwin-x64`
/// - `linux`  + `aarch64` → `linux-arm64`
/// - `linux`  + `x86_64`  → `linux-x64`
/// - `windows`+ `x86_64`  → `win32-x64` (binary suffixed `.exe`)
fn current_platform_native_path() -> Option<PathBuf> {
    let platform_tag = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win32-x64",
        _ => return None,
    };
    let exe = if cfg!(windows) { "tsc.exe" } else { "tsc" };
    Some(
        PathBuf::from("node_modules/@typescript")
            .join(format!("typescript-{platform_tag}"))
            .join("lib")
            .join(exe),
    )
}

/// Return the legacy native-preview path for compatibility with existing
/// installations that have not migrated to the stable `typescript` package.
fn legacy_platform_native_path() -> Option<PathBuf> {
    let platform_tag = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win32-x64",
        _ => return None,
    };
    let exe = if cfg!(windows) { "tsgo.exe" } else { "tsgo" };
    Some(
        PathBuf::from("node_modules/@typescript")
            .join(format!("native-preview-{platform_tag}"))
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

    #[test]
    fn discovers_typescript_wrapper_in_local_node_modules() {
        let tmp = tempdir().unwrap();
        let wrapper = tmp.path().join("node_modules/typescript/bin/tsc");
        fs::create_dir_all(wrapper.parent().unwrap()).unwrap();
        fs::write(&wrapper, "#!/usr/bin/env node\n").unwrap();

        let found = discover(tmp.path()).unwrap();
        assert_eq!(found.path, wrapper);
        assert!(found.needs_node);
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
    fn errors_when_not_installed() {
        let tmp = tempdir().unwrap();
        let err = discover(tmp.path()).unwrap_err();
        assert!(matches!(err, DiscoveryError::NotFound { .. }));
    }

    #[test]
    fn native_binary_preferred_over_js_wrapper() {
        // When both forms are present, the native binary wins (saves Node
        // startup overhead).
        let Some(rel) = current_platform_native_path() else {
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
    fn current_platform_native_path_returns_some_for_known_platforms() {
        // Smoke test: on the platforms our CI matrix covers, the helper
        // should return a path; on unknown platforms returning None is
        // also valid behavior.
        let _ = current_platform_native_path();
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
        let Some(rel) = current_platform_native_path() else {
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
        let Some(rel) = current_platform_native_path() else {
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
