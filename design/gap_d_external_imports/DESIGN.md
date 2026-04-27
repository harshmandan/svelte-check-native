# Gap D — External-import rewrite (validated 2026-04-27)

## Problem

`threlte/packages/xr` over-fires +3 errors:

> `xr/src/routes/Gamepad.svelte:2:30 "Cannot find module '../../../extras/src/lib/index.js'"`
> `Gamepad.svelte:7:25 "Parameter 'event' implicitly has an 'any' type."`
> `Gamepad.svelte:8:26 "Parameter 'event' implicitly has an 'any' type."`

The two `implicit any` errors cascade from the import failure: when
`useGamepad`'s type can't resolve, its return type collapses to `any`,
the inferred `(event) => ...` arrow loses contextual typing, and TS
fires implicit-any warnings.

## Root cause

User code:
```ts
// xr/src/routes/Gamepad.svelte
import { useGamepad } from '../../../extras/src/lib/index.js'
```

The `../../../` segments are relative to the source dir
`packages/xr/src/routes/`. Resolving:
- `..` → `packages/xr/src/`
- `../..` → `packages/xr/`
- `../../..` → `packages/`
- target = `packages/extras/src/lib/index.js` (a sibling package)

Our overlay lives at:
```
packages/xr/node_modules/.cache/svelte-check-native/svelte/src/routes/Gamepad.svelte.svn.ts
```

If we emit the specifier verbatim (`../../../extras/...`), tsgo
resolves it relative to the OVERLAY's directory, which is 7 levels
deep inside the cache — three `..` only escapes to
`.cache/svelte-check-native/`, never reaching `packages/extras/`.
Hence TS2307.

## What upstream does

Upstream rewrites the specifier at emit time. See
`language-tools/packages/svelte2tsx/src/helpers/rewriteExternalImports.ts`:

```ts
function getExternalImportRewrite(specifier, options) {
    const sourceDir = path.dirname(options.sourcePath);
    const generatedDir = path.dirname(options.generatedPath);
    if (!pathPart.startsWith('../')) return null;
    const targetPath = path.resolve(sourceDir, pathPart);
    if (isWithinDirectory(targetPath, options.workspacePath)) return null;
    return { rewritten: path.relative(generatedDir, targetPath) + suffix };
}
```

The rewrite kicks in only when the resolved target sits OUTSIDE the
workspace (in-workspace `../`-imports stay as-is — they pass through
TS's `rootDirs` virtual mapping, which the overlay tsconfig sets up
for both source and cache directories).

`forEachExternalImportRewrite` walks the TS AST and applies the
rewrite to:
- `ImportDeclaration` / `ExportDeclaration` `moduleSpecifier`
- Dynamic `import('...')` and `require('...')` (CJS)
- `ImportTypeNode` (`import('foo').Bar` in TS type position)
- JSDoc `@type {import('...')...}` (recursive walk)

The call site (`packages/svelte-check/src/incremental.ts:252`) passes:
```ts
rewriteExternalImports: { workspacePath, generatedPath: outPath }
```
to `svelte2tsx` per-file at emit time.

## Concrete diff (Gamepad.svelte)

```
Source:    import { useGamepad } from '../../../extras/src/lib/index.js'

Upstream cache:                       '../../../../../../extras/src/lib/index.js'
Ours cache:                          '../../../../../../../extras/src/lib/index.js'
```

Both rewrite to a longer relative path — differs only because our
cache lives at `node_modules/.cache/svelte-check-native/svelte/`
(7 levels) while upstream's lives at `.svelte-kit/.svelte-check/svelte/`
(6 levels). Both resolve to the same target file.

## Implementation

Post-emit rewrite in `crates/typecheck/src/lib.rs`. Just before
`write_if_changed(&gen_path, &input.generated_ts)`, call
`rewrite_external_imports(generated_ts, source_path, gen_path,
workspace)`.

The function uses a byte-scan rather than an AST walk:

1. Look for any `'../` or `"../` byte sequence.
2. Verify it's in an import context via `is_in_import_context` —
   the preceding tokens must be `from`, `import`, or `import(`.
3. Find the closing quote.
4. Run `compute_rewrite`:
   - Resolve target relative to source_dir.
   - If target is within workspace, skip.
   - Compute new specifier = `path_relative(overlay_dir, target)`.
5. Substitute the new specifier between the original quotes.

### Trade-off vs upstream

Less general than the AST walk:

| Pattern | Upstream | Ours |
|---|---|---|
| `import x from '../../X'` | ✓ | ✓ |
| `export { x } from '../../X'` | ✓ | ✓ |
| `import('../../X')` | ✓ | ✓ |
| `require('../../X')` | ✓ | ✗ |
| `import('../../X').Foo` (type pos) | ✓ | ✓ if at expression; ✗ in JSDoc |
| `@type {import('../../X').Foo}` JSDoc | ✓ | ✗ |

For Threlte/xr the common cases (regular `import`) are sufficient.
TS-overlay output (always TS, never JSDoc) covers the realistic
surface. If a future bench surfaces the missing patterns we can
extend the byte-scan or migrate to an AST walk.

### Bug fixed during implementation

First version cast bytes individually to `char`
(`out.push(bytes[i] as char)`), which corrupts multi-byte UTF-8
characters (em-dashes, smart quotes, non-Latin text in comments)
and shifts byte positions for everything downstream. Fixed by
copying string slices verbatim and only modifying the rewritten
specifier substring.

## Bench impact

| Bench | Before | After |
| :--- | :--- | :--- |
| threlte/xr | 5E (3 from this gap) | 2E byte-perfect on errors |

No regressions on the 18 other benches.
