<div align="center">

  <img src="./svelte-check-native.webp" alt="svelte-check-native" width="300" />

  <h1>svelte-check-native</h1>

[![version](https://img.shields.io/npm/v/svelte-check-native.svg)](https://www.npmjs.com/package/svelte-check-native)
[![downloads](https://img.shields.io/npm/dm/svelte-check-native.svg)](https://www.npmjs.com/package/svelte-check-native)

</div>

Fast CLI type checking for **Svelte 4, Svelte 5, and SvelteKit**, written in Rust and powered by TypeScript 7's native compiler.

Use it in place of `svelte-check` in check scripts, CI, and pre-commit hooks. It supports familiar flags and output formats, with [compatibility differences](#compatibility) documented below.

[Performance](#performance) · [Quick start](#quick-start) · [Compatibility](#compatibility) · [Configuration](#configuration) · [Troubleshooting](#troubleshooting) · [Contributing](./CONTRIBUTING.md)

## Performance

Recorded on a SvelteKit + TypeScript monorepo with **1,357 `.svelte` files**.

| Tool | Cold | Warm | After an edit | Errors / warnings / files with problems |
| --- | ---: | ---: | ---: | ---: |
| **svelte-check-native** | **2.8 s** | **0.3 s** | **2.5 s** | 7 / 49 / 23 |
| **svelte-check-native (tsrs)** | **1.5 s** | **0.3 s** | **1.4 s** | 7 / 49 / 23 |
| svelte-check | 27.4 s | 27.2 s | 27.3 s | 7 / 49 / 23 |
| svelte-check --tsgo | 13.3 s | 13.6 s | 13.7 s | 7 / 49 / 23 |

Benched on M1 Pro with 8 CPU. svelte-check-native `1.12.1` (mean of four runs) and svelte-check `4.7.8` with `--tsgo` (mean of two runs) both type-check with TypeScript `7.1.0-beta`; tsrs `0.25.0` is pinned to the same TypeScript. svelte-check without `--tsgo` (mean of two runs) uses the project's TypeScript `6.0.2`. The project is on svelte `5.55.4`.

```sh
svelte-check-native --tsconfig tsconfig.svn-parity.json --diagnostic-sources 'ts,svelte'
```

The monorepo's `tsconfig.json` sets `composite: true`. With `incremental` off, which is how svelte-check type-checks, TypeScript then stops at error TS6379 and checks nothing. So every tool is benchmarked against `tsconfig.svn-parity.json`, which extends it with `composite`, `incremental` and declaration output turned off ([the recipe](./scripts/parity-rigs.json)).

See the [benchmark script](./scripts/bench.mjs) for the cold, warm, and single-file-edit scenarios.

## Quick start

Install the checker and its TypeScript engine in your existing Svelte project:

```sh
npm install -D svelte-check-native typescript@^7.0.0
npx svelte-check-native --workspace .
```

Requires `typescript` 7+ or `@typescript/native-preview` (tsgo). TypeScript 6 and earlier cannot run checks; you can keep them in your project and install `@typescript/native-preview` instead of TypeScript 7. The checker prefers TypeScript 7+ when both are installed.

TypeScript 7.1 nightlies (`typescript@next`) work from `7.1.0-dev.20261027.1` on (`7.1.0-beta` reports false errors).

Replace `svelte-check` in your `package.json` check script, keeping your supported flags. For example:

```json
{
  "scripts": {
    "check": "svelte-check-native --workspace ."
  }
}
```

Then run `npm run check` locally or in CI. A `tsconfig.json` or `jsconfig.json` is required.

### Faster type checking with tsrs (opt-in)

[tsrs](https://github.com/oxc-project/tsrs) is a Rust port of the TypeScript 7 checker. In benchmarks on two SvelteKit apps (1,124 and 1,357 files), its type-check step ran **3.5–4× faster** with **2.7–3.1× less peak memory** than the TypeScript nightly it is pinned to, with identical diagnostics.

```sh
npm install -D tsrs
SVN_ENGINE=tsrs npx svelte-check-native --workspace .
```

It uses its own pinned TypeScript version, so results can differ from your installed version. Supported platforms: macOS arm64 and Linux x64/arm64 (glibc and musl). Selection is explicit; a missing tsrs installation fails without falling back. `TSGO_BIN` takes precedence.

## Compatibility

The project targets parity with [`svelte-check`](https://www.npmjs.com/package/svelte-check) diagnostics and CLI behavior. It is a command-line checker; editor integration and formatting are outside its scope.

| Feature or flag | Behavior |
| --- | --- |
| Svelte 4 and Svelte 5 | Supported, including SvelteKit projects. |
| TypeScript engine | Native TypeScript 7.0, 7.1 nightlies, or the native preview; `--tsgo` is accepted for compatibility. |
| Svelte warnings | Native Rust pass by default. Use `--svelte-warnings bridge` to run your installed `svelte/compiler`. |
| `--watch`, `--preserveWatchOutput` | Unsupported. Use an external watcher such as [watchexec](https://github.com/watchexec/watchexec). |
| `--no-tsconfig` | Unsupported. A tsconfig or jsconfig is required. |
| `--diagnostic-sources css` | Accepted, but CSS language-service diagnostics are not bundled. |
| `--incremental` | Opt-in, as in upstream. See [cache troubleshooting](#troubleshooting). |

Run `npx svelte-check-native --help` for the full CLI reference.

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | No errors, and no warnings when `--fail-on-warnings` is set. |
| `1` | Errors found, warnings with `--fail-on-warnings`, or the TypeScript compiler was missing or did not complete. |
| `2` | Invocation error, such as an invalid flag or missing tsconfig. |

## Configuration

### Additional flags

| Flag | Purpose |
| --- | --- |
| `--svelte-warnings <native\|bridge>` | Select the native warning pass (default) or the JavaScript compiler bridge. |
| `--timings` | Show wall-clock timings for each phase. |
| `--debug-paths` | Print resolved compiler and runtime paths, then exit. |
| `--tsgo-version` | Print the resolved compiler path and version, then exit. |
| `--tsgo-diagnostics` | Print compiler performance and memory statistics. |
| `--emit-ts` | Print the generated TypeScript for each file, then exit. |
| `--include-suggestions` | Include unused-code and deprecation hints, hidden by default. |
| `--disable-enhance` | Disable additional checks that intentionally differ from upstream tsgo mode, such as missing `.svelte` imports. |

### Environment variables

| Variable | Purpose |
| --- | --- |
| `TSGO_BIN` | Override compiler discovery with an absolute path to a native `tsc`/`tsgo` binary or JavaScript wrapper. Takes precedence over `SVN_ENGINE`. |
| `SVN_ENGINE=tsrs` | Opt into the [tsrs engine](#faster-type-checking-with-tsrs-opt-in). |
| `SVN_BRIDGE_WORKERS` | Set the `svelte/compiler` worker count. Defaults to half the cores, capped at 8; uses performance-core count on Apple Silicon. |
| `SVN_DISABLE_ENHANCE` | Disable the additional checks described under `--disable-enhance`. |
| `CLAUDECODE=1` | Default to `machine` output when running from Claude Code, matching upstream. |
| `NO_COLOR` / `FORCE_COLOR` | Disable or force colored output. |

### Android (Termux)

`svelte-check-native`'s `linux-arm64` binary is static, so it installs
and runs on Android too. TypeScript 7 doesn't publish an Android
build, so npm skips its native compiler there. Its `linux-arm64` build
runs fine on Android; install it explicitly:

```sh
npm i -D --force @typescript/typescript-linux-arm64@<your typescript version>
```

Or point `TSGO_BIN` at any Linux arm64 `tsc`/`tsgo` binary.

## Troubleshooting

### Compiler not found or unexpected compiler version

Inspect the resolved paths and version:

```sh
npx svelte-check-native --debug-paths
npx svelte-check-native --tsgo-version
```

Install `typescript` 7+ or `@typescript/native-preview` in your project, or set `TSGO_BIN` when the compiler lives elsewhere in a monorepo.

### Stale errors with `--incremental`

Clear the cache and rerun:

```sh
rm -rf node_modules/.cache/svelte-check-native
npm run check
```

The compiler's `tsbuildinfo` can retain stale results. Runs without `--incremental` (the default) do not read it.

### TS2321: “Excessive stack depth”

If this comes from your own types, check for a `UnionToRecord<T>` that round-trips through `UnionToTuple<T>[number]`. Iterating `T` directly in the mapped-type key can avoid the issue.

## How it works

The Rust CLI parses and analyzes Svelte files, generates a TypeScript overlay, invokes the selected native compiler, and maps diagnostics back to the original source. Svelte warnings come from the native lint pass or an optional JavaScript compiler bridge.

| Crate | Responsibility |
| --- | --- |
| [`parser`](./crates/parser) | Parse Svelte source into an AST. |
| [`analyze`](./crates/analyze) | Analyze runes, props, events, bindings, and scope. |
| [`emit`](./crates/emit) | Generate the TypeScript overlay and rewrite imports. |
| [`svn-lint`](./crates/svn-lint) | Produce Svelte compiler warnings in Rust. |
| [`svelte-compiler`](./crates/svelte-compiler) | Bridge to the project's `svelte/compiler` through persistent Bun/Node workers. |
| [`typecheck`](./crates/typecheck) | Manage the overlay, compiler invocation, incremental cache, and diagnostic mapping. |
| [`enhance`](./crates/enhance) | Add checks beyond upstream tsgo mode. |
| [`core`](./crates/core) | Share spans, diagnostics, position maps, and tsconfig types. |
| [`cli`](./crates/cli) | Handle flags, file discovery, output, and exit codes. |

Script and template-expression syntax errors come from [`acorn-rs`](https://github.com/harshmandan/acorn-rs), a separate crate that ports acorn and acorn-typescript so the errors match Svelte's parser exactly.

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for setup, tests, and parity requirements, and the [changelog](./CHANGELOG.md) for release history.

CSS language-service diagnostics remain on the roadmap.

## Acknowledgments

- [`svelte2tsx`](https://github.com/sveltejs/language-tools/tree/master/packages/svelte2tsx)
  / [`svelte-check`](https://github.com/sveltejs/language-tools/tree/master/packages/svelte-check)
  — transpiler + CLI whose output shape and flags we
  match. The `.v5` fixture corpus from `svelte2tsx` is our parity gate.
- [TypeScript 7](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/)
  — the native compiler that does the actual type-checking.
- [tsrs](https://github.com/oxc-project/tsrs) — Rust port of the
  TypeScript 7 checker, supported as an opt-in engine.

## License

[MIT](./LICENSE)
