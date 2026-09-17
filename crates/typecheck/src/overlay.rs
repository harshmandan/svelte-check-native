//! Generate the overlay tsconfig that tsgo runs against.
//!
//! The overlay extends the user's tsconfig and re-points it at the
//! generated `.svelte.ts` files in the cache. Forces the flags tsgo needs
//! to consume our virtual files (`allowArbitraryExtensions`, `noEmit`,
//! incremental build info location).
//!
//! ### rootDirs merging
//!
//! TS treats `rootDirs` as an array — and arrays do NOT merge across the
//! `extends` chain (inner config wins outright). That means just setting
//! a child `rootDirs` here would clobber whatever the user's tsconfig had
//! (commonly SvelteKit's `[".." , "./types"]`).
//!
//! To keep relative imports resolving from generated `++Foo.svelte.ts`
//! files in the overlay back to the original source tree, we compute the
//! union of:
//!   - `<overlay>/svelte`        — where our generated `.ts` files live
//!   - every `rootDirs` entry from the user's `extends` chain (resolved
//!     to absolute paths so the absolute-vs-relative distinction is gone)
//!   - the workspace root         — fallback for projects that don't
//!     declare `rootDirs` themselves
//!
//! TS then virtually merges all those folders, so a relative import
//! `./loading-labels` from
//! `<overlay>/svelte/src/lib/components/ai/++AssistantOverlay.svelte.ts`
//! ALSO resolves against
//! `<workspace>/src/lib/components/ai/loading-labels`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use svn_core::tsconfig::{TsConfigFile, load_chain};

use crate::cache::CacheLayout;

/// Build the overlay tsconfig JSON given the user's tsconfig path and the
/// list of generated `.svelte.ts` files we want type-checked.
///
/// `user_tsconfig` is the *original* user-supplied tsconfig path
/// (absolute), used as the `extends` target. `generated_files` are the
/// absolute paths of the generated `.svelte.ts` files we wrote into the
/// cache.
pub fn build(
    layout: &CacheLayout,
    user_tsconfig: &Path,
    generated_files: &[std::path::PathBuf],
    js_overlays: &[std::path::PathBuf],
    kit_overlay_sources: &[std::path::PathBuf],
    kit_types_mirror: Option<&Path>,
) -> Value {
    // `extends` is resolved relative to the overlay tsconfig dir.
    let extends_rel = relative_from(layout.root.as_path(), user_tsconfig);

    // `files` are absolutized so tsgo doesn't mis-resolve. Always
    // include the svelte type shim so generated files can reference
    // `svelte/*` modules even when the real package isn't installed.
    let mut files: Vec<String> = generated_files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.push(layout.svelte_shims.to_string_lossy().into_owned());
    // The user's own `files` entries are appended further down, once the
    // extends chain has been loaded — see `user_files`. They cannot be
    // left to inherit: `files` is replace-on-child, so the array we write
    // here shadows theirs completely.

    // Walk the user's extends chain once via the canonical loader.
    // Every derived field the overlay needs (paths, rootDirs, include,
    // exclude, types) is computed by iterating this single Vec — no
    // parallel JSON reads, no local extends resolver. The loader does
    // `${configDir}` substitution, `.json` inference, and
    // `node_modules/@tsconfig/...` walk-up for us.
    let chain: Vec<TsConfigFile> = load_chain(user_tsconfig).unwrap_or_default();

    // JS overlays join `files` whenever the project accepts JavaScript.
    // Reaching them only through `include` loses them to any `exclude`
    // that covers `node_modules` — every SvelteKit tsconfig has one, and
    // our cache lives there — after which they load only as imports of
    // their sidecars, as unchecked library JavaScript. Without `allowJs`
    // (on by default under `checkJs`) listing them is TS6504, and tsgo
    // skips them through `include` anyway.
    let allow_js = svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.allow_js)
        .or_else(|| svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.check_js))
        .is_some_and(|(_, on)| on);
    if allow_js {
        files.extend(js_overlays.iter().map(|p| p.to_string_lossy().into_owned()));
    }

    let mut root_dirs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push_root =
        |dir: &Path, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
            let s = dir.to_string_lossy().to_string();
            if seen.insert(s.clone()) {
                out.push(s);
            }
        };
    // Cache mirror of `.svelte-kit/types/` first, when present. tsgo's
    // rootDirs resolution prefers the FIRST entry whose virtual file
    // exists — listing the mirror ahead of the user's own
    // `.svelte-kit/types/` is what makes our path-rewritten
    // `$types.d.ts` win and keeps the `'…/src/routes/…/+page.js'`
    // chain inside the cache instead of leaking out to user source.
    if let Some(dir) = kit_types_mirror {
        push_root(dir, &mut root_dirs, &mut seen);
    }
    // The project's own `rootDirs` next, then the overlay's svelte
    // subdir, as svelte-check writes them: the WINNING declaration in the
    // extends chain (TypeScript replaces the field, it never merges), each
    // entry anchored on the config that declared it — or, when no config
    // declares one, the entry tsconfig's own directory. A relative import
    // written in a component then resolves from its overlay through this
    // virtual merge exactly where the compiler resolves it for the source,
    // and nowhere else: a `../shared/x` that leaves every root is TS2307.
    match svn_core::tsconfig::winning_field(&chain, |f| {
        (!f.compiler_options.root_dirs.is_empty()).then_some(&f.compiler_options.root_dirs)
    }) {
        Some((file, dirs)) => {
            let dir = file.config_dir();
            for rd in dirs {
                let resolved = if Path::new(rd).is_absolute() {
                    PathBuf::from(rd)
                } else {
                    dir.join(rd)
                };
                push_root(normalize(&resolved).as_path(), &mut root_dirs, &mut seen);
            }
        }
        None => {
            let entry_dir = user_tsconfig.parent().unwrap_or(layout.workspace.as_path());
            push_root(normalize(entry_dir).as_path(), &mut root_dirs, &mut seen);
            // The overlay tree mirrors the workspace, so the workspace is
            // the directory it stands in for. svelte-check's default root
            // is the tsconfig's directory, which is the same directory
            // whenever the tsconfig sits at the workspace root; a
            // tsconfig elsewhere (`--tsconfig ../tsconfig.json`) still
            // needs the workspace for a component's relative imports to
            // land beside the component.
            push_root(layout.workspace.as_path(), &mut root_dirs, &mut seen);
        }
    }
    push_root(layout.svelte_dir.as_path(), &mut root_dirs, &mut seen);

    // Path aliases. Each value keeps its place and is followed by its
    // cache-mirror candidate, the order svelte-check writes them in: a
    // path-mapped import resolves against the source tree first — so a
    // `Foo.svelte.ts` runes module beside `Foo.svelte` wins for
    // `$lib/Foo.svelte`, as the compiler's extension probing decides —
    // and reaches the component's overlay declaration only when nothing
    // in the source tree matches. Path-mapped specifiers skip `rootDirs`
    // entirely, so without the mirror candidate they would never reach
    // an overlay at all.
    let mut paths_map: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut paths_keys_order: Vec<String> = Vec::new();
    // Ordered Vec for emit-stable output alongside a parallel HashSet so
    // dedup-on-insert is O(1) instead of an O(n²) `Vec::contains` scan.
    let mut paths_accumulated: std::collections::HashMap<String, (Vec<String>, HashSet<String>)> =
        std::collections::HashMap::new();
    // `paths` is REPLACE-when-specified, not a per-pattern merge: the
    // first config in the chain that declares it wins outright and the
    // rest of the chain's patterns are discarded. TypeScript is
    // explicit about this, and it bites in a shape SvelteKit users hit
    // constantly — `$lib/*` comes from `.svelte-kit/tsconfig.json`, the
    // user adds an alias of their own without spreading Kit's, and
    // every `$lib` import stops resolving. Accumulating across the
    // chain resolved those imports and reported a clean run where the
    // compiler reports TS2307 on each one.
    //
    // Precedence, not load order — see `winning_field`.
    if let Some((file, file_paths)) =
        svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.paths.as_ref())
    {
        let dir = file.config_dir();
        // Relative `paths` values anchor on the DECLARING config's
        // directory — nothing else. The engine we drive is tsgo, and
        // TypeScript 7 removed `baseUrl` outright: a config that still
        // sets one draws a fatal TS5102, and a bare non-`./` value
        // draws TS5090, so the TS≤6 rule of re-anchoring values on an
        // effective `baseUrl` from elsewhere in the chain does not
        // exist on this surface. (Verified against tsgo directly:
        // `./`-prefixed values resolve against the config file that
        // declares them, wherever it sits in the extends chain.)
        for (pattern, values) in file_paths {
            let mut entry: (Vec<String>, HashSet<String>) = (Vec::new(), HashSet::new());
            for v in values {
                let abs = if Path::new(v).is_absolute() {
                    PathBuf::from(v)
                } else {
                    dir.join(v)
                };
                let s = normalize(&abs).to_string_lossy().into_owned();
                if entry.1.insert(s.clone()) {
                    entry.0.push(s);
                }
            }
            if !entry.0.is_empty() {
                paths_keys_order.push(pattern.clone());
                paths_accumulated.insert(pattern.clone(), entry);
            }
        }
    }

    // Sibling-project `paths` are deliberately NOT merged in. A
    // referenced project's `paths` have no effect on the project that
    // references it — TypeScript applies only the compiling project's
    // own map — so unioning them let imports resolve that the compiler
    // reports TS2307 on. Sibling `include`/`exclude` widening below is a
    // different matter: it exists so a transitive import into a
    // referenced project's SOURCE doesn't trip "File not listed within
    // project", and it does not change how specifiers resolve.

    for pattern in paths_keys_order {
        let (values, _) = paths_accumulated.remove(&pattern).unwrap_or_default();
        let mut merged: Vec<String> = Vec::with_capacity(values.len() * 2);
        let mut seen: HashSet<String> = HashSet::with_capacity(values.len() * 2);
        for v in values {
            let mirrored = mirror_into_overlay(layout, &v);
            if seen.insert(v.clone()) {
                merged.push(v);
            }
            if let Some(m) = mirrored
                && seen.insert(m.clone())
            {
                merged.push(m);
            }
        }
        paths_map.insert(
            pattern,
            Value::Array(merged.into_iter().map(Value::String).collect()),
        );
    }

    let mut compiler_options = serde_json::Map::new();
    compiler_options.insert("noEmit".into(), json!(true));
    compiler_options.insert("allowArbitraryExtensions".into(), json!(true));
    // `forceConsistentCasingInFileNames` is INHERITED, not forced.
    // Per CLAUDE.md ("not stricter or lax-er than upstream"), the
    // user's tsconfig setting wins. Earlier we forced this to `false`
    // to dodge a TS1149 our cache-mirror layout could trigger on
    // macOS case-insensitive filesystems (auto-extension resolution
    // case-collapses `./Code.svelte` onto a sibling `code.svelte.ts`
    // runes module). If that case actually surfaces in real benches,
    // fix the cache-mirror at its source rather than silently
    // disabling a strict check the user opted into.
    // `allowImportingTsExtensions` is INHERITED, not forced. Whatever
    // the user sets in their tsconfig carries through. Setting it to
    // `true` unconditionally here silently widened user-authored
    // `.ts`-extension imports that upstream svelte-check flags via
    // TS5097 — 44 such errors on one bench alone. Upstream's own
    // overlay doesn't set the flag either; our `.svelte` overlay
    // resolution doesn't need it (handled by `allowArbitraryExtensions`
    // + the `.d.svelte.ts` ambient sidecars whose `.ts` re-exports are
    // legal under declaration-file rules regardless of the flag).
    // Incremental only on request, as upstream (`incremental.ts`).
    let incremental = crate::incremental();
    compiler_options.insert("incremental".into(), json!(incremental));
    if incremental {
        compiler_options.insert(
            "tsBuildInfoFile".into(),
            json!(layout.tsbuildinfo.to_string_lossy()),
        );
    }
    // A composite project inherits `composite` untouched. With
    // incremental compilation off that is TS6379, and the compiler then
    // checks nothing — exactly what svelte-check's overlay produces, so
    // that one error is the whole report.
    // `skipLibCheck` is INHERITED, not forced. Per CLAUDE.md ("not
    // stricter or lax-er than upstream"), the user's tsconfig setting
    // wins — when unset, tsgo defaults to `false` and type-checks
    // node_modules `.d.ts` files. Forcing `true` here silently dropped
    // real third-party type-incompatibility errors that upstream
    // svelte-check surfaces (cryptgeon's `Invalidator` removed from
    // `svelte/store` in @zerodevx/svelte-toast was the canary —
    // upstream caught it, we silently passed). Users who want the
    // skip behaviour can set `"skipLibCheck": true` in their own
    // tsconfig.
    // `moduleResolution` is INHERITED, never rewritten. TypeScript 7
    // removed the legacy `node`/`node10` value outright and answers
    // TS5108 "Option 'moduleResolution=node10' has been removed",
    // abandoning the program — upstream surfaces exactly that and
    // reports nothing else.
    //
    // We used to silently rewrite the legacy value to `bundler` (and
    // `module` to `esnext` with it), which was friendlier but wrong in
    // two ways: it hid a config error the user has to fix, and it
    // type-checked their code under different resolution semantics
    // (`exports` handling, extension rules) than the ones they
    // configured. Per the parity rule we are neither stricter nor
    // laxer than upstream, and this was laxer.
    compiler_options.insert("rootDirs".into(), json!(root_dirs));
    // `types` is inherited as written, the way svelte-check's overlay
    // inherits it. An entry that does not resolve is a fatal TS2688 there:
    // the compiler abandons the program and svelte-check reports nothing,
    // and so do we.
    //
    // TypeScript resolves a path-shaped entry against the directory of
    // the ROOT config it compiles — `types` is not one of the options
    // `extends` rebases — and under svelte-check that root is its overlay
    // in `.svelte-check/`. Ours sits elsewhere, so path-shaped entries are
    // re-anchored on svelte-check's overlay directory to resolve (or
    // fail) exactly where they do there.
    // `typeRoots`, unlike `types`, IS an `isFilePath` option — TS rebases
    // it against the config that declared it during the extends merge, so
    // each entry anchors on its own declaring directory.
    //
    // The resolved value is re-emitted so the overlay doesn't depend on
    // TS re-deriving it. Re-emitting matters
    // because of `${configDir}`: TS substitutes that placeholder against
    // the ROOT config being compiled, which is our overlay, so an
    // inherited `"${configDir}/typings"` silently became a path inside
    // the cache directory. Every entry under it then failed to resolve —
    // a fatal TS2688 that took the project's ambients with it.
    let type_roots: Vec<PathBuf> =
        svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.type_roots.as_deref())
            .map(|(f, list)| {
                let dir = f.config_dir();
                list.iter()
                    .map(|r| normalize(&dir.join(r)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    // Emitted only when the chain actually declared it — `typeRoots`
    // REPLACES the default `node_modules/@types` walk-up, so writing one
    // where the user had none would narrow the lookup rather than
    // preserve it.
    if !type_roots.is_empty() {
        compiler_options.insert(
            "typeRoots".into(),
            json!(
                type_roots
                    .iter()
                    .map(|r| r.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            ),
        );
    }
    // Any OTHER inherited option carrying `${configDir}` has the same
    // problem, and enumerating which ones matter is hopeless — `rootDir`
    // is not even a field we parse, yet an inherited `"${configDir}/src"`
    // resolves into the cache and fires TS6059 on every source file.
    // The loader records which raw keys it resolved; re-emit those, with
    // the value it already computed against the user's entry config.
    // Options we set deliberately above win.
    //
    // Re-emitted per extends PRECEDENCE, not chain load order: the
    // winner for a key is whichever declaration TypeScript would let
    // win. And only when that winner itself used `${configDir}` — if
    // the winning declaration is a plain value (say a base declares
    // `"rootDir": "${configDir}/src"` but the entry overrides it with
    // `"rootDir": "."`), the overlay's `extends` already inherits it
    // correctly, and writing the base's resolved value here would
    // shadow the override.
    let mut config_dir_key_order: Vec<&String> = Vec::new();
    {
        let mut seen: HashSet<&str> = HashSet::new();
        for file in chain.iter() {
            for key in &file.config_dir_keys {
                if seen.insert(key.as_str()) {
                    config_dir_key_order.push(key);
                }
            }
        }
    }
    for key in config_dir_key_order {
        if compiler_options.contains_key(key) {
            continue;
        }
        let Some((file, value)) =
            svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.raw.get(key))
        else {
            continue;
        };
        if file.config_dir_keys.contains(key) {
            compiler_options.insert(key.clone(), value.clone());
        }
    }
    //
    // Without a real `svelte` install our shim declares the `svelte`
    // modules in its place, so a `svelte` entry is dropped there: it names
    // exactly the package the shim stands in for.
    let shim_stands_in = !crate::has_real_svelte(&layout.workspace);
    let is_svelte_entry = |t: &str| t == "svelte" || t.starts_with("svelte/");
    if let Some((_, list)) =
        svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.types.as_deref())
        && list
            .iter()
            .any(|t| is_filesystem_types_entry(t) || (shim_stands_in && is_svelte_entry(t)))
    {
        let anchor = crate::upstream_overlay::cache_dir(&layout.workspace);
        let types: Vec<String> = list
            .iter()
            .filter(|t| !(shim_stands_in && is_svelte_entry(t)))
            .map(|t| overlay_types_entry(t, &anchor))
            .collect();
        compiler_options.insert("types".into(), json!(types));
    }
    if !paths_map.is_empty() {
        compiler_options.insert("paths".into(), Value::Object(paths_map));
        // Intentionally NOT setting `baseUrl`. TypeScript 5.0 removed
        // `baseUrl` as a top-level compiler option (TS5102) and tsgo
        // (the TS 7.0 dev preview that this binary targets) doesn't
        // require it for `paths` to resolve — every paths-target value
        // we emit is absolute. Setting baseUrl had a real, hidden
        // cost: tsgo silently suppresses diagnostic emission for
        // files outside `baseUrl`'s tree AND, in some configurations,
        // suppresses diagnostics on overlay files entirely.
        //
        // Note: if the USER's tsconfig sets `baseUrl`, tsgo fires a
        // TS5102 attributed to our overlay (inherited via `extends`).
        // That is user-caused and upstream `svelte-check --tsgo`
        // surfaces it, so we surface it too — every diagnostic the
        // compiler attributes to the overlay tsconfig is surfaced.
    }

    // Pull the user's `include` patterns into our overlay so tsgo also
    // type-checks standalone TS modules the user authored (route loaders,
    // hooks, $lib helpers, .svelte.ts rune-helper modules, etc.). Without
    // this our overlay only sees the generated .svelte.ts overlays plus
    // their transitive imports — anything the user `include`s but that's
    // not reached from a .svelte file goes unchecked.
    //
    // Patterns matching `*.svelte` are dropped: tsgo can't parse raw
    // .svelte files, and the .svelte content is already covered by the
    // generated overlays we list in `files`. Patterns are emitted as
    // absolute path globs so the tsconfig works regardless of the
    // overlay's location relative to the workspace.
    // Inner wins for include/exclude. `.svelte` patterns flow
    // through verbatim — tsgo's include scan only admits its
    // supported extensions, so a `*.svelte` glob never pulls raw
    // `.svelte` sources into the program. Mirrors upstream
    // (`incremental.ts:417` keeps user `include` patterns as-is).
    let mut user_includes = winning_patterns_absolute(&chain, |f| f.include.as_deref());
    // Redirect `.svelte-kit/types/**/$types.d.ts` includes to the
    // cache mirror (when present). Load-bearing companion to the
    // mirror+rootDirs setup: without this redirect the user's
    // `$types.d.ts` files stay in the file set, tsgo loads them, and
    // the embedded `'../(…/)src/routes/…/+page.js'` chain still
    // walks back to the user's untyped source — defeating the
    // mirror entirely. With the redirect, only our path-rewritten
    // copies under the cache mirror enter the program. See
    // `kit_types_mirror::sync_mirror`.
    if let Some(mirror) = kit_types_mirror {
        let mirror_str = mirror.to_string_lossy();
        for pat in &mut user_includes {
            if let Some((seg_pos, seg_len)) = crate::kit_types_mirror::find_kit_types_segment(pat) {
                let mut rebuilt = String::with_capacity(pat.len() + 32);
                rebuilt.push_str(&mirror_str);
                rebuilt.push_str(&pat[seg_pos + seg_len..]);
                *pat = rebuilt;
            }
        }
    }
    // Per-pattern virtual projection. For every workspace-anchored
    // user/sibling include pattern, push a parallel pattern pointing
    // into `<cache>/svelte/` with `.svelte` rewritten to
    // `.d.svelte.ts`. Mirrors upstream's `virtualInclude` shape
    // (`incremental.ts:427` + `toVirtualSvelteDtsSpec` at :963-966).
    //
    // `.svelte` patterns project to `.d.svelte.ts` and catch the
    // ambient sidecars whose re-exports pull the `.svn.ts`/`.svn.js`
    // overlays into the program transitively. That chain replaces
    // listing `.svn.ts` in `compilerOptions.files` directly (step 4).
    //
    // Non-`.svelte` patterns project as-is (e.g. `src/**/*.ts` →
    // `<cache>/svelte/src/**/*.ts`). For us those projections are
    // structural-parity no-ops: Kit overlays land directly in
    // `compilerOptions.files` instead. Emitting them anyway keeps
    // the include shape isomorphic to upstream's.
    let mut projected: Vec<String> = Vec::new();
    for pat in &user_includes {
        if let Some(p) = project_to_virtual_svelte_dts(layout, pat) {
            if !user_includes.contains(&p) && !projected.contains(&p) {
                projected.push(p);
            }
        }
    }
    user_includes.extend(projected);
    // Baseline catch-all for cache overlays. Required because our
    // cache lives under `node_modules/.cache/svelte-check-native/`,
    // and TypeScript's default include scan hardcodes `node_modules`
    // exclusion. Without an explicit `include` glob into the cache,
    // overlays NEVER reach the program for tsconfigs that omit
    // `include` entirely (LS-fixture style: just `compilerOptions`
    // + `exclude`). Upstream svelte-check sidesteps this — their
    // cache lives at `<workspace>/.svelte-check/`, outside any
    // default-excluded path, so default scan finds overlays even
    // with no `include`. Different cache location is the structural
    // divergence; this glob is its workaround. Documented as the
    // third intentional divergence in `notes/PARITY_REFACTOR.md`.
    //
    // It is emitted ONLY when the user's chain declares neither
    // `include` nor `files`, which is exactly the case TypeScript
    // answers by scanning everything under the config directory. When
    // they DO declare one, the projection above already covers every
    // overlay their config admits, and adding the catch-all on top
    // re-admits the ones it doesn't: a `.svelte` file under an
    // `exclude`d directory, or outside a narrow `include`, came back
    // into the program through its `.d.svelte.ts` sidecar and got
    // type-checked anyway. That inflated both the error count and the
    // FILES denominator on any project whose include is narrower than
    // its workspace.
    let declares_file_set = svn_core::tsconfig::winning_patterns(&chain, |f| f.include.as_deref())
        .is_some()
        || svn_core::tsconfig::winning_patterns(&chain, |f| f.files.as_deref()).is_some();
    if !declares_file_set {
        let cache_dts_glob = format!("{}/**/*.d.svelte.ts", layout.svelte_dir.to_string_lossy());
        if !user_includes.contains(&cache_dts_glob) {
            user_includes.push(cache_dts_glob);
        }
    }

    // The user's `files`, rebased onto the config that declared them.
    //
    // `files` is replace-on-child, so the array we write shadows theirs
    // entirely — leaving it to inherit means their entries never reach
    // the program at all. Two ways that shows up, in opposite
    // directions: a project loading globals via `files: ["./ambient.d.ts"]`
    // gets a spurious "Cannot find name" at every use of them, and a
    // `files`-only project (no `include`, a deliberate closed world) has
    // its listed entry files silently unchecked while we check a
    // different set entirely.
    //
    // `.svelte` entries are swapped for their generated `.d.svelte.ts`
    // sidecar: the compiler can't parse a raw `.svelte` file and would
    // answer TS6054. Upstream does the same swap (`incremental.ts:409-411`).
    for entry in winning_patterns_absolute(&chain, |f| f.files.as_deref()) {
        let mapped = match entry.strip_suffix(".svelte") {
            Some(_) => {
                let mapped = project_to_virtual_svelte_dts(layout, &entry);
                if mapped.is_none() {
                    // Upstream checks a files-listed component wherever
                    // it lives — verified against default-engine
                    // svelte-check on an app whose `files` names
                    // `../shared/Comp.svelte` (the component's own
                    // errors are reported, relative filename and all).
                    // Our sidecar mirror only spans the workspace
                    // subtree, so an out-of-tree component has no
                    // overlay slot yet; surface the skip instead of
                    // silently checking a smaller program. Tracked in
                    // notes/OPEN.md.
                    eprintln!(
                        "svelte-check-native: warning: `files` entry {entry} lies outside the \
                         workspace and was not type-checked"
                    );
                }
                mapped
            }
            None => Some(entry),
        };
        if let Some(m) = mapped
            && !files.contains(&m)
        {
            files.push(m);
        }
    }

    let mut overlay = serde_json::Map::new();
    overlay.insert("extends".into(), Value::String(extends_rel));
    overlay.insert("compilerOptions".into(), Value::Object(compiler_options));
    overlay.insert("files".into(), json!(files));
    if !user_includes.is_empty() {
        overlay.insert("include".into(), json!(user_includes));
    }
    // Exclude list. Two sources that must union:
    //
    // 1. The user's own `exclude` from their tsconfig chain — e.g.
    //    `playwright/fixtures/videos/**/*` for binary-named-`.ts`
    //    files. tsconfig semantics REPLACE (not merge) exclude when
    //    the child config declares one, so dropping this would let
    //    user-excluded content back into the program.
    // 2. Original Kit-file source paths that have an injected
    //    overlay at a mirrored cache path. Without this, tsgo
    //    loads BOTH the untyped original and the typed overlay.
    //
    // Only emit the field if at least one source contributed — an
    // empty `exclude` field in our overlay would clobber the user's
    // inherited exclude with an empty list.
    let mut excludes: Vec<String> = winning_patterns_absolute(&chain, |f| f.exclude.as_deref());
    // Project the excludes into the cache tree, the same way includes
    // are projected. Without this counterpart an `exclude` never
    // reached the generated overlays: the pattern named the user's
    // `.svelte` source, our program contains its `.d.svelte.ts`
    // sidecar under the cache instead, and the file was type-checked
    // despite the user having excluded it. Upstream projects them too
    // (`virtualExclude`, `incremental.ts:415-420`).
    let mut projected_excludes: Vec<String> = Vec::new();
    for pat in &excludes {
        if let Some(p) = project_to_virtual_svelte_dts(layout, pat)
            && !excludes.contains(&p)
            && !projected_excludes.contains(&p)
        {
            projected_excludes.push(p);
        }
    }
    excludes.extend(projected_excludes);
    for p in kit_overlay_sources {
        excludes.push(p.to_string_lossy().into_owned());
    }
    // Deliberately NOT in this union: one exclude entry per source
    // `.svelte` file (upstream's `upsertedExcludes` shape,
    // `incremental.ts:430-431`). Upstream needs those because its
    // language-service host would otherwise load raw `.svelte`
    // sources matched by live `*.svelte` include patterns. tsgo's
    // include scan only admits its supported extensions
    // (`.ts`/`.tsx`/`.js`/`.jsx`/`.d.ts`/`.json`), so raw `.svelte`
    // files never enter the program with or without the entries —
    // while tsgo's config phase pays per-entry glob compilation for
    // every exclude on every walked path. On a 1350-component
    // workspace the per-file entries put ~200 KB of absolute paths
    // in `exclude` and held tsgo's config phase at ~0.9s; dropping
    // them cut it to ~0.05s with a byte-identical program file set
    // and diagnostics (verified via `--extendedDiagnostics` +
    // full-output diff on the same buildinfo-warm cache).
    if !excludes.is_empty() {
        overlay.insert("exclude".into(), json!(excludes));
    }
    // The entry config's own `references` (they are never inherited
    // through `extends`), as svelte-check's overlay carries them. An
    // import into a referenced project then resolves to that project's
    // build output, and an unbuilt one is TS6305 at the import.
    if let Some(entry) = chain.first()
        && !entry.references.is_empty()
    {
        let dir = entry.config_dir();
        let references: Vec<Value> = entry
            .references
            .iter()
            .map(|r| {
                let path = if Path::new(&r.path).is_absolute() {
                    PathBuf::from(&r.path)
                } else {
                    dir.join(&r.path)
                };
                json!({ "path": normalize(&path).to_string_lossy() })
            })
            .collect();
        overlay.insert("references".into(), Value::Array(references));
    }
    Value::Object(overlay)
}

/// Map an absolute paths-target path INTO the overlay svelte tree. If
/// the input path is not under the workspace root, return None — the
/// mirror only makes sense for paths inside the project we generated
/// overlays for.
///
/// Resolves relative paths against the workspace root explicitly. The
/// cache root's parent stopped equalling the workspace once the cache
/// moved under `node_modules/.cache/`, so taking `layout.root.parent()`
/// would point at `node_modules/.cache/` and strip-prefix would fail
/// for every path-target the user actually declared.
fn mirror_into_overlay(layout: &CacheLayout, path_str: &str) -> Option<String> {
    let p = Path::new(path_str);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        layout.workspace.join(p)
    };
    let normalized = normalize(&abs);
    let rel = normalized.strip_prefix(&layout.workspace).ok()?;
    let mirrored = layout.svelte_dir.join(rel);
    Some(mirrored.to_string_lossy().into_owned())
}

/// Project a workspace-anchored include pattern into the overlay's
/// `svelte/` cache tree, replacing a trailing `.svelte` glob suffix
/// with `.d.svelte.ts`. Mirrors upstream's `toVirtualSvelteDtsSpec`
/// (`incremental.ts:963-966`):
///
/// - `<workspace>/src/**/*.svelte` → `<cache>/svelte/src/**/*.d.svelte.ts`
/// - `<workspace>/src/**/*.ts`     → `<cache>/svelte/src/**/*.ts`
/// - `<other>/...` (not under workspace) → None (we don't mirror
///   external trees into the overlay's `svelte/` dir)
/// - any path already inside the cache → None (already projected,
///   would re-cache itself recursively)
///
/// The `.ts` projection is the analogue of upstream's mechanism for
/// catching Kit overlays under `<cache>/svelte/src/routes/+layout.ts`
/// via the user's `src/**/*.ts` include. We list Kit overlays
/// directly in `compilerOptions.files`, so this projection is a
/// structural-parity no-op for us — but emitting it keeps the
/// overlay's include shape isomorphic to upstream's.
fn project_to_virtual_svelte_dts(layout: &CacheLayout, abs_pattern: &str) -> Option<String> {
    let p = Path::new(abs_pattern);
    if !p.is_absolute() {
        return None;
    }
    let normalized = normalize(p);
    if normalized.starts_with(&layout.root) {
        return None;
    }
    let rel = normalized.strip_prefix(&layout.workspace).ok()?;
    let rel_str = rel.to_string_lossy();
    let projected_rel = if let Some(stripped) = rel_str.strip_suffix(".svelte") {
        format!("{stripped}.d.svelte.ts")
    } else {
        rel_str.into_owned()
    };
    let mirrored = layout.svelte_dir.join(projected_rel);
    Some(mirrored.to_string_lossy().into_owned())
}

/// The chain's winning `include` / `exclude` patterns under TS's
/// `extends` precedence (leaf wins, later array-extends entries beat
/// earlier ones — see [`svn_core::tsconfig::winning_patterns`]). Each
/// pattern is resolved against the DECLARING config's dir so the
/// overlay's absolute-path globs work regardless of where the overlay
/// tsconfig itself lives. Empty when the field is declared nowhere OR
/// declared as an explicit empty array (either way the overlay emits
/// no user patterns for it).
fn winning_patterns_absolute<F>(chain: &[TsConfigFile], get: F) -> Vec<String>
where
    F: for<'a> Fn(&'a TsConfigFile) -> Option<&'a [String]>,
{
    let Some((winner, patterns)) = svn_core::tsconfig::winning_patterns(chain, get) else {
        return Vec::new();
    };
    let dir = winner.config_dir();
    patterns
        .iter()
        .map(|s| {
            let resolved = if Path::new(s).is_absolute() {
                PathBuf::from(s)
            } else {
                dir.join(s)
            };
            normalize(&resolved).to_string_lossy().into_owned()
        })
        .collect()
}

/// Rewrite one `types` entry into the form the overlay tsconfig must
/// carry.
///
/// TypeScript resolves a path-shaped `types` entry (`"./worker.d.ts"`,
/// `"../shared/globals"`) against the directory of the ROOT config being
/// compiled, so the entry is anchored on `anchor_dir` — the directory of
/// the root config the entry is meant to be read from. Package-style
/// entries (`"node"`, `"vite/client"`, `"@types/foo"`) are left alone:
/// they resolve by walking `node_modules` upwards, and every candidate
/// overlay directory is nested inside the workspace, so that walk reaches
/// the same packages. Absolute entries are unambiguous wherever they sit.
fn overlay_types_entry(entry: &str, anchor_dir: &Path) -> String {
    if !is_filesystem_types_entry(entry) {
        return entry.to_string();
    }
    let path = Path::new(entry);
    if path.is_absolute() {
        return entry.to_string();
    }
    normalize(&anchor_dir.join(path))
        .to_string_lossy()
        .into_owned()
}

/// True when the entry should be treated as a filesystem path rather
/// than a package spec. Filesystem paths begin with `./`, `../`, or `/`
/// (POSIX-style absolute). Everything else — bare names, scoped names,
/// package subpaths — resolves through `node_modules`.
fn is_filesystem_types_entry(entry: &str) -> bool {
    entry.starts_with('.') || entry.starts_with('/')
}

/// Collapse `..` segments without touching the filesystem. Pure path
/// arithmetic — necessary because `Path::canonicalize` requires the path
/// to exist, and rootDirs entries from extends chains often point at
/// locations that won't exist for every user.
fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Compute a relative path from `from` to `to`, falling back to the
/// absolute `to` if a relative path can't be expressed (different roots).
///
/// Used so the overlay's `extends` path is relative when possible — keeps
/// generated tsconfigs portable across machines / CI cache layouts.
fn relative_from(from: &Path, to: &Path) -> String {
    if let Ok(rel) = pathdiff(to, from) {
        return rel.to_string_lossy().into_owned();
    }
    to.to_string_lossy().into_owned()
}

/// Tiny inline path-diff implementation. Returns the path you'd append to
/// `from` to reach `to`, using `..` segments as needed.
///
/// Doesn't follow symlinks or canonicalize; both inputs should already be
/// absolute and in the same logical filesystem.
fn pathdiff(to: &Path, from: &Path) -> Result<std::path::PathBuf, ()> {
    use std::path::{Component, PathBuf};

    let to_components: Vec<_> = to.components().collect();
    let from_components: Vec<_> = from.components().collect();

    if to.has_root() != from.has_root() {
        return Err(());
    }

    let mut common = 0;
    while common < to_components.len()
        && common < from_components.len()
        && to_components[common] == from_components[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();
    for _ in common..from_components.len() {
        // Each remaining segment in `from` requires a `..`.
        result.push(Component::ParentDir);
    }
    for c in &to_components[common..] {
        result.push(c);
    }

    if result.as_os_str().is_empty() {
        Ok(PathBuf::from("."))
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pathdiff_sibling_dirs() {
        let to = PathBuf::from("/a/b/foo.json");
        let from = PathBuf::from("/a/b/.cache");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("../foo.json"));
    }

    #[test]
    fn pathdiff_descendant() {
        let to = PathBuf::from("/a/b/c/d.json");
        let from = PathBuf::from("/a/b");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("c/d.json"));
    }

    #[test]
    fn pathdiff_same_dir() {
        let to = PathBuf::from("/a/b/x.json");
        let from = PathBuf::from("/a/b");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("x.json"));
    }

    #[test]
    fn build_overlay_sets_required_compiler_options() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let gen_files = vec![PathBuf::from(
            "/projects/app/.svelte-check/svelte/++Index.svelte.ts",
        )];
        let overlay = build(&layout, &user_ts, &gen_files, &[], &[], None);

        let opts = &overlay["compilerOptions"];
        assert_eq!(opts["noEmit"], json!(true));
        assert_eq!(opts["allowArbitraryExtensions"], json!(true));
        // Incremental only under `--incremental`, as upstream.
        assert_eq!(opts["incremental"], json!(false));
        assert!(opts.get("tsBuildInfoFile").is_none());
    }

    #[test]
    fn build_overlay_extends_user_tsconfig_relatively() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let overlay = build(&layout, &user_ts, &[], &[], &[], None);
        // extends should point ../tsconfig.json (overlay is in
        // /projects/app/.svelte-check/, user ts in /projects/app/).
        assert_eq!(overlay["extends"], json!("../tsconfig.json"));
    }

    #[test]
    fn build_overlay_lists_generated_files_absolute() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let gen_files = vec![
            PathBuf::from("/projects/app/.svelte-check/svelte/++A.svelte.ts"),
            PathBuf::from("/projects/app/.svelte-check/svelte/sub/++B.svelte.ts"),
        ];
        let overlay = build(&layout, &user_ts, &gen_files, &[], &[], None);
        let files = overlay["files"].as_array().unwrap();
        // 2 generated + 1 svelte-shims.d.ts = 3.
        assert_eq!(files.len(), 3);
        assert!(files[0].as_str().unwrap().ends_with("++A.svelte.ts"));
        assert!(files[1].as_str().unwrap().ends_with("++B.svelte.ts"));
        assert!(files[2].as_str().unwrap().ends_with("svelte-shims.d.ts"));
    }

    #[test]
    fn build_overlay_includes_svelte_shims_when_no_generated_files() {
        // Even with zero `.svelte` files, the shim must still appear so
        // standalone `.ts`/`.js` files in the project can import from
        // svelte/* modules.
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let overlay = build(&layout, &user_ts, &[], &[], &[], None);
        let files = overlay["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().ends_with("svelte-shims.d.ts"));
    }

    // ===== Canonical-loader-driven overlay behaviors ====================
    //
    // These write real tsconfigs into a tempdir and run `build()` end-to-
    // end through `load_chain`. Guards against regressions in the three
    // places the overlay's loader integration matters most:
    //
    //   * package `extends` via `node_modules/<pkg>/…`
    //   * `${configDir}` substitution per-declaring-file
    //   * array-form `extends` (TS 5.0+) merge order
    //
    // Each test sets up the minimal on-disk shape and asserts on the
    // overlay JSON that `build()` returns.

    use std::fs;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn build_overlay_inherits_paths_and_rootdirs_from_package_extends() {
        // Workspace tsconfig extends `@tsconfig/svelte` from a local
        // node_modules. The overlay builder should walk the package-
        // extends target, inherit its `paths` + `rootDirs`, and
        // project them into the overlay with absolute-path values.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();

        let pkg_ts = ws.join("node_modules/@tsconfig/svelte/tsconfig.json");
        write_file(
            &pkg_ts,
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "$lib": ["./src/lib"],
                        "$lib/*": ["./src/lib/*"]
                    },
                    "rootDirs": ["./extra-types"]
                }
            }"#,
        );

        let user_ts = ws.join("tsconfig.json");
        write_file(
            &user_ts,
            r#"{ "extends": "@tsconfig/svelte/tsconfig.json" }"#,
        );

        let layout = CacheLayout::for_workspace(&ws);
        let overlay = build(&layout, &user_ts, &[], &[], &[], None);

        let opts = &overlay["compilerOptions"];
        // rootDirs union includes svelte cache, workspace, AND the
        // inherited rootDirs entry, resolved against the package
        // tsconfig's dir (not the user's).
        let root_dirs: Vec<&str> = opts["rootDirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_extra = ws
            .join("node_modules/@tsconfig/svelte/extra-types")
            .to_string_lossy()
            .into_owned();
        assert!(
            root_dirs.iter().any(|r| *r == expected_extra),
            "expected {expected_extra:?} in rootDirs, got {root_dirs:?}",
        );

        // paths inherit from the package extends and get projected with
        // a cache-mirror candidate prepended for each value.
        let paths = opts["paths"].as_object().unwrap();
        assert!(
            paths.contains_key("$lib"),
            "paths keys: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
        assert!(paths.contains_key("$lib/*"));
        let lib_values: Vec<&str> = paths["$lib"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_original = ws
            .join("node_modules/@tsconfig/svelte/src/lib")
            .to_string_lossy()
            .into_owned();
        assert!(
            lib_values.iter().any(|v| *v == expected_original),
            "original paths-target not present: {lib_values:?}",
        );
    }

    #[test]
    fn build_overlay_substitutes_configdir_to_entry_dir() {
        // Base config uses `${configDir}` for both baseUrl and rootDirs;
        // the user extends it from a DIFFERENT directory. Overlay must
        // resolve the placeholder against the ENTRY (user/project) dir —
        // TS semantics: a shared base resolves into the consuming project.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let base_dir = ws.join("configs");
        let project_dir = ws.join("project");

        let base_ts = base_dir.join("base.json");
        write_file(
            &base_ts,
            r#"{
                "compilerOptions": {
                    "baseUrl": "${configDir}/src",
                    "rootDirs": ["${configDir}/types"],
                    "paths": {
                        "$lib": ["./local/lib"],
                        "$abs": ["${configDir}/abs-target"]
                    }
                }
            }"#,
        );

        let user_ts = project_dir.join("tsconfig.json");
        write_file(&user_ts, r#"{ "extends": "../configs/base.json" }"#);

        let layout = CacheLayout::for_workspace(&project_dir);
        let overlay = build(&layout, &user_ts, &[], &[], &[], None);

        let opts = &overlay["compilerOptions"];

        // ${configDir} in rootDirs resolves to the ENTRY (project) dir.
        let root_dirs: Vec<&str> = opts["rootDirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_types = project_dir.join("types").to_string_lossy().into_owned();
        assert!(
            root_dirs.iter().any(|r| *r == expected_types),
            "expected ${{configDir}}-resolved rootDirs entry {expected_types:?}, got {root_dirs:?}",
        );
        // Must NOT resolve against the base config's own dir.
        let wrong_types = base_dir.join("types").to_string_lossy().into_owned();
        assert!(
            !root_dirs.iter().any(|r| *r == wrong_types),
            "${{configDir}} wrongly resolved to base's dir: {wrong_types:?}",
        );

        // Absolute `${configDir}/abs-target` resolves to the ENTRY dir:
        // project_dir/abs-target.
        let paths = opts["paths"].as_object().unwrap();
        let abs_values: Vec<&str> = paths["$abs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_abs = project_dir
            .join("abs-target")
            .to_string_lossy()
            .into_owned();
        assert!(
            abs_values.iter().any(|v| *v == expected_abs),
            "${{configDir}}-in-paths not resolved correctly: {abs_values:?}",
        );
    }

    #[test]
    fn build_overlay_applies_array_extends_precedence_to_paths() {
        // `extends: ["./a.json", "./b.json"]` — later array entries beat
        // earlier ones, and `paths` is REPLACE-when-specified rather
        // than a per-pattern merge. So `b`'s map wins outright and
        // `from-a` does not survive.
        //
        // Verified against tsc --showConfig on this exact shape:
        // paths = {"from-b": ["./b-target"]}.
        //
        // This test previously asserted the opposite — that both
        // patterns flow through — which is what our per-pattern union
        // produced. That union made unresolvable imports resolve: the
        // SvelteKit shape where `$lib/*` comes from
        // `.svelte-kit/tsconfig.json` and the user restates `paths`
        // without spreading it reports TS2307 from the compiler and a
        // clean run from us.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();

        write_file(
            &ws.join("a.json"),
            r#"{
                "compilerOptions": {
                    "paths": { "from-a": ["./a-target"] }
                }
            }"#,
        );
        write_file(
            &ws.join("b.json"),
            r#"{
                "compilerOptions": {
                    "paths": { "from-b": ["./b-target"] }
                }
            }"#,
        );

        let user_ts = ws.join("tsconfig.json");
        write_file(&user_ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let layout = CacheLayout::for_workspace(&ws);
        let overlay = build(&layout, &user_ts, &[], &[], &[], None);

        let paths = overlay["compilerOptions"]["paths"].as_object().unwrap();
        assert!(
            paths.contains_key("from-b"),
            "the later extends entry must win; got {:?}",
            paths.keys().collect::<Vec<_>>(),
        );
        assert!(
            !paths.contains_key("from-a"),
            "paths is replaced, not merged, so from-a must not survive; got {:?}",
            paths.keys().collect::<Vec<_>>(),
        );
    }

    #[test]
    fn is_filesystem_types_entry_picks_relative_and_absolute() {
        assert!(is_filesystem_types_entry("./foo"));
        assert!(is_filesystem_types_entry("../foo/bar.d.ts"));
        assert!(is_filesystem_types_entry("/abs/path/foo.d.ts"));
        assert!(!is_filesystem_types_entry("foo"));
        assert!(!is_filesystem_types_entry("vite/client"));
        assert!(!is_filesystem_types_entry("@scope/pkg/sub"));
    }

    /// A relative entry is anchored on the directory passed in.
    #[test]
    fn overlay_types_entry_absolutises_relative_paths() {
        let dir = Path::new("/proj/apps/dash");
        assert_eq!(
            overlay_types_entry("./worker-configuration.d.ts", dir),
            "/proj/apps/dash/worker-configuration.d.ts"
        );
        assert_eq!(
            overlay_types_entry("../shared/globals", dir),
            "/proj/apps/shared/globals"
        );
    }

    /// Package specs resolve by walking `node_modules` upwards, and the
    /// cache dir is nested inside the workspace, so that walk still
    /// finds the same packages — leave them untouched. Already-absolute
    /// entries are unambiguous wherever they sit.
    #[test]
    fn overlay_types_entry_leaves_packages_and_absolute_paths_alone() {
        let dir = Path::new("/proj/apps/dash");
        assert_eq!(overlay_types_entry("node", dir), "node");
        assert_eq!(overlay_types_entry("vite/client", dir), "vite/client");
        assert_eq!(overlay_types_entry("@types/foo", dir), "@types/foo");
        assert_eq!(
            overlay_types_entry("/abs/globals.d.ts", dir),
            "/abs/globals.d.ts"
        );
    }
}
