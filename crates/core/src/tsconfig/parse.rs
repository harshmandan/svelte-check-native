//! Parse a single tsconfig.json file into a [`TsConfigFile`].
//!
//! Uses the `json5` crate to tolerate comments (`//`, `/* */`) and trailing
//! commas — both common in real tsconfigs.
//!
//! Extends-chain resolution, `${configDir}` substitution, and merging happen
//! in a higher-level pass; this module handles only the bytes-to-struct
//! conversion for a single file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::{CompilerOptions, ModuleResolution, Reference, TsConfigFile};

/// Errors that can arise while parsing a tsconfig.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: json5::Error,
    },

    #[error("tsconfig root must be a JSON object (in {path})")]
    NotAnObject { path: PathBuf },
}

/// Parse a tsconfig from its file path. Reads the file, then delegates.
pub fn parse_file(path: impl AsRef<Path>) -> Result<TsConfigFile, ParseError> {
    let path = path.as_ref();
    let source = std::fs::read_to_string(path).map_err(|source| ParseError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    parse_str(&source, path.to_path_buf())
}

/// Parse a tsconfig from an in-memory string.
///
/// `path` is the absolute path the source was read from (or would be written
/// to); stored on the returned [`TsConfigFile`] for diagnostics and extends
/// resolution. It is NOT used to read anything.
pub fn parse_str(source: &str, path: PathBuf) -> Result<TsConfigFile, ParseError> {
    let value: Value = json5::from_str(source).map_err(|source| ParseError::Json {
        path: path.clone(),
        source,
    })?;

    let Value::Object(mut root) = value else {
        return Err(ParseError::NotAnObject { path });
    };

    let extends = take_string_or_array(&mut root, "extends");
    let compiler_options = take_compiler_options(&mut root);

    let include = take_opt_string_array(&mut root, "include");
    let exclude = take_opt_string_array(&mut root, "exclude");
    let files = take_opt_string_array(&mut root, "files");
    let references = take_references(&mut root);

    Ok(TsConfigFile {
        path,
        extends,
        compiler_options,
        include,
        exclude,
        files,
        references,
    })
}

fn take_compiler_options(root: &mut Map<String, Value>) -> CompilerOptions {
    let Some(Value::Object(mut obj)) = root.remove("compilerOptions") else {
        return CompilerOptions::default();
    };

    CompilerOptions {
        base_url: take_string(&mut obj, "baseUrl"),
        paths: take_paths(&mut obj),
        root_dirs: take_string_array(&mut obj, "rootDirs"),

        allow_js: take_bool(&mut obj, "allowJs"),
        check_js: take_bool(&mut obj, "checkJs"),
        no_unused_locals: take_bool(&mut obj, "noUnusedLocals"),
        no_unused_parameters: take_bool(&mut obj, "noUnusedParameters"),

        strict: take_bool(&mut obj, "strict"),
        strict_null_checks: take_bool(&mut obj, "strictNullChecks"),
        strict_function_types: take_bool(&mut obj, "strictFunctionTypes"),
        strict_bind_call_apply: take_bool(&mut obj, "strictBindCallApply"),
        no_implicit_any: take_bool(&mut obj, "noImplicitAny"),
        no_implicit_this: take_bool(&mut obj, "noImplicitThis"),
        always_strict: take_bool(&mut obj, "alwaysStrict"),

        module_resolution: take_module_resolution(&mut obj),
        module: take_string(&mut obj, "module"),
        target: take_string(&mut obj, "target"),
        jsx: take_string(&mut obj, "jsx"),
        jsx_import_source: take_string(&mut obj, "jsxImportSource"),

        type_roots: take_opt_string_array(&mut obj, "typeRoots"),
        types: take_opt_string_array(&mut obj, "types"),

        composite: take_bool(&mut obj, "composite"),
        declaration: take_bool(&mut obj, "declaration"),
        declaration_map: take_bool(&mut obj, "declarationMap"),
        declaration_dir: take_string(&mut obj, "declarationDir"),

        allow_arbitrary_extensions: take_bool(&mut obj, "allowArbitraryExtensions"),
        skip_lib_check: take_bool(&mut obj, "skipLibCheck"),
        verbatim_module_syntax: take_bool(&mut obj, "verbatimModuleSyntax"),
        isolated_modules: take_bool(&mut obj, "isolatedModules"),
        resolve_json_module: take_bool(&mut obj, "resolveJsonModule"),
        allow_synthetic_default_imports: take_bool(&mut obj, "allowSyntheticDefaultImports"),
        es_module_interop: take_bool(&mut obj, "esModuleInterop"),

        // Everything else — preserve verbatim for overlay pass-through.
        raw: obj,
    }
}

// ===== Typed extractors ===================================================

fn take_string(obj: &mut Map<String, Value>, key: &str) -> Option<String> {
    match obj.remove(key)? {
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn take_bool(obj: &mut Map<String, Value>, key: &str) -> Option<bool> {
    obj.remove(key).and_then(|v| v.as_bool())
}

fn take_string_array(obj: &mut Map<String, Value>, key: &str) -> Vec<String> {
    take_opt_string_array(obj, key).unwrap_or_default()
}

fn take_opt_string_array(obj: &mut Map<String, Value>, key: &str) -> Option<Vec<String>> {
    match obj.remove(key)? {
        Value::Array(arr) => Some(
            arr.into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    }
}

fn take_string_or_array(obj: &mut Map<String, Value>, key: &str) -> Vec<String> {
    match obj.remove(key) {
        Some(Value::String(s)) => vec![s],
        Some(Value::Array(arr)) => arr
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn take_paths(obj: &mut Map<String, Value>) -> BTreeMap<String, Vec<String>> {
    let Some(Value::Object(entries)) = obj.remove("paths") else {
        return BTreeMap::new();
    };

    let mut out = BTreeMap::new();
    for (key, value) in entries {
        if let Value::Array(arr) = value {
            let patterns = arr
                .into_iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s),
                    _ => None,
                })
                .collect::<Vec<_>>();
            out.insert(key, patterns);
        }
    }
    out
}

fn take_module_resolution(obj: &mut Map<String, Value>) -> Option<ModuleResolution> {
    match obj.remove("moduleResolution")? {
        Value::String(s) => ModuleResolution::parse(&s),
        _ => None,
    }
}

fn take_references(root: &mut Map<String, Value>) -> Vec<Reference> {
    let Some(Value::Array(arr)) = root.remove("references") else {
        return Vec::new();
    };

    arr.into_iter()
        .filter_map(|v| {
            let Value::Object(obj) = v else { return None };
            let Some(Value::String(path)) = obj.get("path") else {
                return None;
            };
            Some(Reference { path: path.clone() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse(src: &str) -> TsConfigFile {
        parse_str(src, PathBuf::from("/test/tsconfig.json")).expect("parse ok")
    }

    #[test]
    fn empty_object_gives_default() {
        let cfg = parse("{}");
        assert!(cfg.extends.is_empty());
        assert!(cfg.include.is_none());
        assert!(cfg.exclude.is_none());
        assert!(cfg.files.is_none());
        assert!(cfg.references.is_empty());
        assert!(cfg.compiler_options.base_url.is_none());
    }

    #[test]
    fn handles_line_and_block_comments() {
        let src = r#"
            // leading comment
            {
                /* block
                   comment */
                "compilerOptions": {
                    "strict": true // trailing
                }
            }
        "#;
        let cfg = parse(src);
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn handles_trailing_commas() {
        let src = r#"{
            "compilerOptions": {
                "strict": true,
                "target": "ES2022",
            },
            "include": ["src/**/*",],
        }"#;
        let cfg = parse(src);
        assert_eq!(cfg.compiler_options.strict, Some(true));
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
        assert_eq!(cfg.include.as_deref(), Some(&["src/**/*".to_string()][..]));
    }

    #[test]
    fn extends_as_string() {
        let cfg = parse(r#"{"extends": "./base.json"}"#);
        assert_eq!(cfg.extends, vec!["./base.json"]);
    }

    #[test]
    fn extends_as_array_ts5() {
        let cfg = parse(r#"{"extends": ["./a.json", "./b.json"]}"#);
        assert_eq!(cfg.extends, vec!["./a.json", "./b.json"]);
    }

    #[test]
    fn extends_missing_is_empty() {
        let cfg = parse("{}");
        assert!(cfg.extends.is_empty());
    }

    #[test]
    fn parses_common_compiler_options() {
        let src = r#"{
            "compilerOptions": {
                "target": "ES2022",
                "module": "ESNext",
                "moduleResolution": "bundler",
                "strict": true,
                "noUnusedLocals": true,
                "skipLibCheck": true,
                "baseUrl": ".",
                "paths": {
                    "$lib/*": ["./src/lib/*"],
                    "$app/*": ["./src/app/*"]
                },
                "rootDirs": ["./src", "./.svelte-kit"]
            }
        }"#;
        let cfg = parse(src);
        let co = &cfg.compiler_options;
        assert_eq!(co.target.as_deref(), Some("ES2022"));
        assert_eq!(co.module.as_deref(), Some("ESNext"));
        assert_eq!(co.module_resolution, Some(ModuleResolution::Bundler));
        assert_eq!(co.strict, Some(true));
        assert_eq!(co.no_unused_locals, Some(true));
        assert_eq!(co.skip_lib_check, Some(true));
        assert_eq!(co.base_url.as_deref(), Some("."));
        assert_eq!(
            co.paths.get("$lib/*").map(|v| v.as_slice()),
            Some(&["./src/lib/*".to_string()][..])
        );
        assert_eq!(co.root_dirs, vec!["./src", "./.svelte-kit"]);
    }

    #[test]
    fn preserves_unknown_compiler_options_in_raw() {
        let src = r#"{
            "compilerOptions": {
                "strict": true,
                "experimentalDecorators": true,
                "useDefineForClassFields": false
            }
        }"#;
        let cfg = parse(src);
        assert_eq!(cfg.compiler_options.strict, Some(true));
        // Unknown fields survive in raw for pass-through.
        assert_eq!(
            cfg.compiler_options.raw.get("experimentalDecorators"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            cfg.compiler_options.raw.get("useDefineForClassFields"),
            Some(&Value::Bool(false))
        );
    }

    #[test]
    fn parses_include_exclude_files() {
        let src = r#"{
            "include": ["src/**/*", "tests/**/*"],
            "exclude": ["node_modules", "dist"],
            "files": ["Entry.svelte"]
        }"#;
        let cfg = parse(src);
        assert_eq!(
            cfg.include.as_deref(),
            Some(&["src/**/*".to_string(), "tests/**/*".to_string()][..])
        );
        assert_eq!(
            cfg.exclude.as_deref(),
            Some(&["node_modules".to_string(), "dist".to_string()][..])
        );
        assert_eq!(
            cfg.files.as_deref(),
            Some(&["Entry.svelte".to_string()][..])
        );
    }

    #[test]
    fn parses_references() {
        let src = r#"{
            "references": [
                { "path": "./packages/a" },
                { "path": "./packages/b", "prepend": false }
            ]
        }"#;
        let cfg = parse(src);
        assert_eq!(cfg.references.len(), 2);
        assert_eq!(cfg.references[0].path, "./packages/a");
        assert_eq!(cfg.references[1].path, "./packages/b");
    }

    #[test]
    fn non_object_root_errors() {
        let err = parse_str("[]", PathBuf::from("/x")).unwrap_err();
        assert!(matches!(err, ParseError::NotAnObject { .. }));
    }

    #[test]
    fn malformed_json_errors() {
        let err = parse_str("{not json", PathBuf::from("/x")).unwrap_err();
        assert!(matches!(err, ParseError::Json { .. }));
    }

    #[test]
    fn config_dir_is_parent_of_path() {
        let cfg = parse_str("{}", PathBuf::from("/home/user/project/tsconfig.json")).unwrap();
        assert_eq!(cfg.config_dir(), Path::new("/home/user/project"));
    }

    #[test]
    fn module_resolution_coerces_case_and_aliases() {
        let cfg = parse(r#"{"compilerOptions": {"moduleResolution": "NodeNext"}}"#);
        assert_eq!(
            cfg.compiler_options.module_resolution,
            Some(ModuleResolution::NodeNext)
        );
        let cfg = parse(r#"{"compilerOptions": {"moduleResolution": "node10"}}"#);
        assert_eq!(
            cfg.compiler_options.module_resolution,
            Some(ModuleResolution::Node)
        );
    }

    #[test]
    fn granular_strict_flags_parsed() {
        let src = r#"{
            "compilerOptions": {
                "strict": true,
                "strictNullChecks": false,
                "strictFunctionTypes": true,
                "noImplicitAny": true
            }
        }"#;
        let cfg = parse(src);
        assert_eq!(cfg.compiler_options.strict, Some(true));
        assert_eq!(cfg.compiler_options.strict_null_checks, Some(false));
        assert_eq!(cfg.compiler_options.strict_function_types, Some(true));
        assert_eq!(cfg.compiler_options.no_implicit_any, Some(true));
        assert_eq!(cfg.compiler_options.strict_bind_call_apply, None);
    }
}
