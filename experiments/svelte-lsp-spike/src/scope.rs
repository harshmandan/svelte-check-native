//! Project discovery, and the narrow tsconfig that makes the whole design work.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Default, Clone)]
pub struct Project {
    pub workspace: PathBuf,
    pub user_tsconfig: PathBuf,
    /// Whether the resolved config type-checks JavaScript.
    ///
    /// Load-bearing because tsgo's language server is more eager than its
    /// batch mode: it returns semantic diagnostics for an *open* `.js`
    /// document whether or not `checkJs` is set, leaving the client to decide.
    /// `tsc -p` on the same project reports nothing. Without this gate a plain
    /// `<script>` component picks up errors the CLI never reports.
    pub check_js: bool,
}

impl Project {
    /// Walk up from a file for the nearest tsconfig/jsconfig, treating its
    /// directory as the workspace root.
    pub fn discover(file: &Path) -> Option<Self> {
        let mut dir = file.parent()?;
        loop {
            for name in ["tsconfig.json", "jsconfig.json"] {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    let check_js = svn_core::tsconfig::load(&candidate)
                        .ok()
                        .and_then(|c| c.compiler_options.check_js)
                        .unwrap_or(false);
                    return Some(Self {
                        workspace: dir.to_path_buf(),
                        user_tsconfig: candidate,
                        check_js,
                    });
                }
            }
            dir = dir.parent()?;
        }
    }

    /// Write (and return) a tsconfig that inherits everything from the user's
    /// — compiler options, paths, types — but contributes no file globs of its
    /// own.
    ///
    /// This one file is the entire narrowing mechanism. The overlay tsconfig
    /// our typecheck crate generates lists the files it wants in `files`, but
    /// it also `extends` the user's config, and `include` is inherited through
    /// an extends chain. So the user's `src/**/*.svelte` glob quietly pulls all
    /// 1,207 components back into the program no matter what `files` says.
    /// Extending through this shim instead cuts the inherited glob, leaving the
    /// program to be exactly the overlays we asked for plus whatever they
    /// genuinely import.
    pub fn narrow_tsconfig(&self) -> std::io::Result<PathBuf> {
        let dir = self
            .workspace
            .join("node_modules")
            .join(".cache")
            .join("svelte-lsp-spike");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("narrow.tsconfig.json");
        // Glob bases, as strings, so the JSON below stays readable.
        let body = serde_json::json!({
            "extends": self.user_tsconfig,
            // Ambient declarations are the one thing narrowing cannot drop.
            // A `declare module` in `src/global.d.ts`, or SvelteKit's generated
            // `$env/*` declarations, are global by nature and reachable from no
            // import — following imports never finds them, and without them
            // every augmented or virtual module looks like it is missing.
            //
            // Listed as concrete paths rather than a glob: `include` globs are
            // resolved against the declaring config's directory and then
            // rewritten again by the overlay builder, which is a lot of
            // machinery to get right for a fixed, short list. `files` entries
            // from the user's config are appended to the overlay's own, so
            // these survive intact.
            "include": [],
            "files": ambients(&self.workspace),
            "compilerOptions": {
                // A composite project must list every file it contains, so the
                // `.ts` modules our components import — which we deliberately
                // leave to tsgo's own resolution — each raise TS6307. Composite
                // exists for build orchestration and buys a language server
                // nothing, so drop it along with the glob.
                "composite": false,
                "declaration": false,
                "declarationMap": false,
            },
        });
        let text = serde_json::to_string_pretty(&body)?;
        // Rewrite only on change: tsgo keys its incremental state off file
        // timestamps, and rewriting an identical config throws that away.
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, &text)?;
        }
        Ok(path)
    }
}

/// The workspace's ambient declaration files, computed once per workspace.
///
/// The walk reads every `.d.ts` under the workspace to decide which ones are
/// ambient, and it was the single most expensive thing in a check — ~21 ms of
/// a 23 ms one-file check on a 1,207-component app, dwarfing both the emit and
/// tsgo itself. The set changes only when someone adds a declaration file, so
/// it is cached for the life of the process.
///
/// The cost of that: a `.d.ts` added mid-session is not picked up until
/// restart. A real server would watch for it.
fn ambients(workspace: &Path) -> Vec<String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Vec<String>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(map) = cache.lock()
        && let Some(hit) = map.get(workspace)
    {
        return hit.as_ref().clone();
    }
    let found = Arc::new(scan_ambients(workspace));
    if let Ok(mut map) = cache.lock() {
        map.insert(workspace.to_path_buf(), Arc::clone(&found));
    }
    found.as_ref().clone()
}

/// The workspace's *ambient* declaration files.
///
/// Not every `.d.ts` is one. `$types.d.ts`, `components.d.ts`, `Foo.svelte.d.ts`
/// are ordinary modules that happen to contain only types — something imports
/// them, so following imports finds them, and listing them eagerly drags
/// unrelated code into the program. A declaration file earns its place here
/// only if it declares something globally: `declare global`, `declare module`,
/// or a triple-slash reference. That is exactly the set nothing can import and
/// therefore the set narrowing would otherwise lose.
fn scan_ambients(workspace: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>, depth: usize) {
        if depth > 8 || out.len() > 200 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                // `.svelte-kit` itself is kept — `ambient.d.ts` there declares
                // `$env/*`, which nothing imports. Its `types/` subtree is a
                // different animal: one generated `$types.d.ts` per route, each
                // importing that route's own modules. Those are reached by
                // import when a route is in scope, and listing all of them
                // eagerly drags most of the app back into the program — 98
                // files here, taking a one-component scope from a few hundred
                // files to 3,935.
                if matches!(name.as_ref(), "node_modules" | ".svelte-check" | ".git" | "build" | "dist")
                    || path.ends_with(".svelte-kit/types")
                {
                    continue;
                }
                walk(&path, out, depth + 1);
            } else if name.ends_with(".d.ts") && declares_globally(&path) {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(workspace, &mut out, 0);
    out.sort();
    out
}

/// Does this declaration file introduce anything reachable without an import?
fn declares_globally(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    text.contains("declare global")
        || text.contains("declare module")
        || text.contains("declare namespace")
        || text.contains("/// <reference")
}
