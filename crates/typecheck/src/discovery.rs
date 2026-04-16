//! Locate the tsgo binary.
//!
//! Resolution order:
//! 1. `TSGO_BIN` env var (absolute path to a tsgo executable or wrapper).
//! 2. Walk up from the workspace root checking for
//!    `node_modules/@typescript/native-preview/bin/tsgo.js`. The closest
//!    enclosing `node_modules` wins.
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

    let needle = Path::new("node_modules/@typescript/native-preview/bin/tsgo.js");
    let mut current: Option<&Path> = Some(workspace);
    while let Some(dir) = current {
        let candidate = dir.join(needle);
        if candidate.is_file() {
            return Ok(TsgoBinary {
                path: candidate,
                needs_node: true,
            });
        }
        current = dir.parent();
    }

    Err(DiscoveryError::NotFound {
        searched_from: workspace.to_path_buf(),
    })
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

    // Note: we intentionally don't have a unit test for the `TSGO_BIN`
    // env-var path. cargo test runs threads in parallel within a binary;
    // mutating process-global env vars is unsound (Rust 2024 marks
    // std::env::set_var unsafe for that reason). The env-var path is
    // covered by integration tests that spawn fresh subprocesses.
}
