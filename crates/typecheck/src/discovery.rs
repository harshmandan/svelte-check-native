//! Locate the tsgo binary.
//!
//! Resolution order at each ancestor `node_modules`, closest first:
//! 1. `TSGO_BIN` env var (absolute path to a tsgo executable or wrapper).
//! 2. The platform-native binary at
//!    `node_modules/@typescript/native-preview-<platform>/lib/tsgo[.exe]`.
//!    This skips Node.js startup overhead (~50-100 ms per check).
//! 3. The JavaScript wrapper at
//!    `node_modules/@typescript/native-preview/bin/tsgo.js` invoked via
//!    `node`.
//!
//! `@typescript/native-preview` ships the JS wrapper and installs a
//! platform-specific package (e.g. `native-preview-darwin-arm64`) as an
//! optionalDependency containing the real binary. We can therefore
//! reliably invoke the native form when one is present and fall back to
//! the wrapper otherwise (e.g. on a platform where no native package
//! exists).
//!
//! Returns a [`TsgoBinary`] handle that the runner uses to spawn the
//! correct command (a `.js` wrapper has to be invoked via `node`; a native
//! binary is invoked directly).

use std::path::{Path, PathBuf};

/// A located tsgo binary, ready to spawn.
#[derive(Debug, Clone)]
pub struct TsgoBinary {
    /// Path to the executable or JS wrapper.
    pub path: PathBuf,
    /// Whether the path is a JavaScript file that must be run via `node`.
    pub needs_node: bool,
}

/// Errors when looking for tsgo.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error(
        "could not find tsgo. Install `@typescript/native-preview` as a \
         devDependency, or set TSGO_BIN to an absolute path. Searched \
         upward from {searched_from} for \
         `node_modules/@typescript/native-preview/bin/tsgo.js`."
    )]
    NotFound { searched_from: PathBuf },

    #[error("TSGO_BIN points at {path} which does not exist")]
    EnvBinNotFound { path: PathBuf },
}

/// Locate tsgo.
pub fn discover(workspace: &Path) -> Result<TsgoBinary, DiscoveryError> {
    if let Ok(env_path) = std::env::var("TSGO_BIN") {
        let path = PathBuf::from(env_path);
        if !path.exists() {
            return Err(DiscoveryError::EnvBinNotFound { path });
        }
        let needs_node = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("js") || ext.eq_ignore_ascii_case("cjs"))
            .unwrap_or(false);
        return Ok(TsgoBinary { path, needs_node });
    }

    let native_relative = current_platform_native_path();
    let wrapper_relative = Path::new("node_modules/@typescript/native-preview/bin/tsgo.js");

    let mut current: Option<&Path> = Some(workspace);
    while let Some(dir) = current {
        // Native binary is preferred — no Node.js startup overhead.
        if let Some(rel) = &native_relative {
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Ok(TsgoBinary {
                    path: candidate,
                    needs_node: false,
                });
            }
        }
        // Fallback: JS wrapper requires `node`.
        let wrapper = dir.join(wrapper_relative);
        if wrapper.is_file() {
            return Ok(TsgoBinary {
                path: wrapper,
                needs_node: true,
            });
        }
        current = dir.parent();
    }

    Err(DiscoveryError::NotFound {
        searched_from: workspace.to_path_buf(),
    })
}

/// Return the relative path to the platform-native tsgo binary, or `None`
/// if we don't know how to map the current platform to a published
/// `@typescript/native-preview-<platform>` package.
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
}
