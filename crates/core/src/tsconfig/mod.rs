//! The canonical `TsConfig` representation.
//!
//! One struct, used everywhere (CLI config resolution, overlay builder, watch
//! cache). **No parallel JSON-reading shortcuts** anywhere else in the
//! workspace — if you need a tsconfig field, add it here and parse it once.
//!
//! See `todo.md` architectural lesson #4: `-rs` had two parallel
//! representations (`CompilerOptions` struct parsing ~6 fields, overlay
//! builder reading ~12 fields directly from JSON). New fields fell in the
//! gap. This module is the single source of truth.
//!
//! ### Scope of this file
//!
//! - [`TsConfigFile`] — the contents of *one* tsconfig.json, parsed from
//!   JSON-with-comments. Has unresolved `extends` as raw strings.
//! - [`parse_str`], [`parse_file`] — one-shot parsing of a single file.
//!
//! Extends-chain resolution + `${configDir}` substitution + merging live in a
//! separate module (coming next). The types here are the inputs to that pass.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod parse;

pub use parse::{ParseError, parse_file, parse_str};

/// A single parsed tsconfig file.
///
/// Paths are stored **as the user wrote them** — no resolution, no
/// `${configDir}` substitution, no absolutization. That work happens in the
/// merge pass which has visibility into the full extends chain.
#[derive(Debug, Clone, Default)]
pub struct TsConfigFile {
    /// Absolute path to the config file itself. Set by the caller of
    /// [`parse_str`] / by [`parse_file`].
    pub path: PathBuf,

    /// Unresolved `extends` references. `extends: "a"` → `vec!["a"]`;
    /// `extends: ["a", "b"]` → `vec!["a", "b"]` (TS 5.0+). `extends: null` or
    /// missing → empty.
    pub extends: Vec<String>,

    pub compiler_options: CompilerOptions,

    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub files: Option<Vec<String>>,

    pub references: Vec<Reference>,
}

impl TsConfigFile {
    /// Directory containing this config file. `${configDir}` resolves here.
    ///
    /// Returns `Path::new("")` — which `Path::join` treats as the current
    /// dir — only if `path` has no parent. Callers constructing via
    /// [`parse_file`] always supply a real file path, so the fallback is
    /// only reachable via synthetic constructions in tests.
    pub fn config_dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }
}

/// `compilerOptions` — fields we parse explicitly. Everything we don't know
/// about is preserved in `raw` verbatim so the overlay can pass it through
/// untouched.
#[derive(Debug, Clone, Default)]
pub struct CompilerOptions {
    pub base_url: Option<String>,
    /// `paths`: key → array of path patterns. BTreeMap for deterministic
    /// iteration order (matters for overlay-generation reproducibility).
    pub paths: BTreeMap<String, Vec<String>>,
    pub root_dirs: Vec<String>,

    pub allow_js: Option<bool>,
    pub check_js: Option<bool>,
    pub no_unused_locals: Option<bool>,
    pub no_unused_parameters: Option<bool>,

    pub strict: Option<bool>,
    pub strict_null_checks: Option<bool>,
    pub strict_function_types: Option<bool>,
    pub strict_bind_call_apply: Option<bool>,
    pub no_implicit_any: Option<bool>,
    pub no_implicit_this: Option<bool>,
    pub always_strict: Option<bool>,

    pub module_resolution: Option<ModuleResolution>,
    pub module: Option<String>,
    pub target: Option<String>,
    pub jsx: Option<String>,
    pub jsx_import_source: Option<String>,

    pub type_roots: Option<Vec<String>>,
    pub types: Option<Vec<String>>,

    pub composite: Option<bool>,
    pub declaration: Option<bool>,
    pub declaration_map: Option<bool>,
    pub declaration_dir: Option<String>,

    pub allow_arbitrary_extensions: Option<bool>,
    pub skip_lib_check: Option<bool>,
    pub verbatim_module_syntax: Option<bool>,
    pub isolated_modules: Option<bool>,
    pub resolve_json_module: Option<bool>,
    pub allow_synthetic_default_imports: Option<bool>,
    pub es_module_interop: Option<bool>,

    /// Every compilerOptions field that we don't explicitly parse, preserved
    /// verbatim for pass-through to tsgo via the overlay.
    pub raw: serde_json::Map<String, serde_json::Value>,
}

/// `moduleResolution` enum. TypeScript accepts a few legacy/alias spellings;
/// we normalize to the canonical form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleResolution {
    /// Classic `node` (aka `node10`).
    Node,
    /// Node16 — CJS-vs-ESM via package type.
    Node16,
    /// NodeNext — latest Node behavior, tracks `moduleResolution` evolution.
    NodeNext,
    /// Bundler — optimistic, for Vite/webpack/bundler-aware consumers.
    Bundler,
    /// Classic (legacy pre-node resolution).
    Classic,
}

impl ModuleResolution {
    /// Parse the textual value. Case-insensitive per TypeScript's own reader.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "node" | "node10" => Some(Self::Node),
            "node16" => Some(Self::Node16),
            "nodenext" => Some(Self::NodeNext),
            "bundler" => Some(Self::Bundler),
            "classic" => Some(Self::Classic),
            _ => None,
        }
    }

    /// Canonical spelling suitable for emitting back to a generated tsconfig.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Node16 => "node16",
            Self::NodeNext => "nodenext",
            Self::Bundler => "bundler",
            Self::Classic => "classic",
        }
    }

    /// Does this resolution mode require explicit `.js`/`.ts` extensions in
    /// import paths? Matters for the svelte-check overlay (we have to match
    /// what the user's code expects).
    pub fn requires_explicit_extensions(self) -> bool {
        matches!(self, Self::Node16 | Self::NodeNext)
    }
}

/// A single entry in `references`.
///
/// The reference may include additional fields (`prepend`, `circular`) which
/// we'll add here if they become relevant.
#[derive(Debug, Clone)]
pub struct Reference {
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_resolution_parse() {
        assert_eq!(
            ModuleResolution::parse("node"),
            Some(ModuleResolution::Node)
        );
        assert_eq!(
            ModuleResolution::parse("Node"),
            Some(ModuleResolution::Node)
        );
        assert_eq!(
            ModuleResolution::parse("NODE10"),
            Some(ModuleResolution::Node)
        );
        assert_eq!(
            ModuleResolution::parse("nodenext"),
            Some(ModuleResolution::NodeNext)
        );
        assert_eq!(
            ModuleResolution::parse("NodeNext"),
            Some(ModuleResolution::NodeNext)
        );
        assert_eq!(
            ModuleResolution::parse("bundler"),
            Some(ModuleResolution::Bundler)
        );
        assert_eq!(ModuleResolution::parse(""), None);
        assert_eq!(ModuleResolution::parse("garbage"), None);
    }

    #[test]
    fn module_resolution_as_str_round_trip() {
        for variant in [
            ModuleResolution::Node,
            ModuleResolution::Node16,
            ModuleResolution::NodeNext,
            ModuleResolution::Bundler,
            ModuleResolution::Classic,
        ] {
            assert_eq!(ModuleResolution::parse(variant.as_str()), Some(variant));
        }
    }

    #[test]
    fn requires_explicit_extensions_only_for_node16_and_nodenext() {
        assert!(!ModuleResolution::Node.requires_explicit_extensions());
        assert!(!ModuleResolution::Bundler.requires_explicit_extensions());
        assert!(!ModuleResolution::Classic.requires_explicit_extensions());
        assert!(ModuleResolution::Node16.requires_explicit_extensions());
        assert!(ModuleResolution::NodeNext.requires_explicit_extensions());
    }
}
