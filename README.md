# svelte-check-native

A fast, CLI-only type checker for **Svelte 5+** projects. Written in Rust,
powered by [tsgo](https://github.com/microsoft/typescript-go).

Drop-in replacement for [`svelte-check`](https://www.npmjs.com/package/svelte-check)
on the command-line surface — same flags, same output formats, same exit codes.

> **Svelte 5 only.** Components using Svelte 4 syntax (`export let foo`, `$:`
> reactive statements, `<slot>`, `on:` event directives) are not a supported
> input. They will mostly parse but downstream type-checking is undefined.
> If you need Svelte 4 support, use upstream `svelte-check`.

> **Status:** pre-alpha. See `todo.md` for the implementation plan. The
> compatibility scoreboard below auto-updates from CI.

## Scoreboard

Parity is tracked by a fixed suite of upstream `svelte-check` fixtures plus one
test per bug we rescued from a related Rust fork. When this hits 39/39, v0.1
ships.

<!-- SCOREBOARD-START -->
```
svelte-check-native scoreboard: 0/39 passing
```
<!-- SCOREBOARD-END -->

## What it is

- **CLI-only.** Accepts files, emits diagnostics, exits with a status code. No
  LSP server, no editor integration, no autocomplete, no hover docs, no
  go-to-definition. Use your editor's Svelte plugin for those.
- **tsgo-only.** Uses `@typescript/native-preview` (tsgo) for TypeScript
  diagnostics. No fallback to `tsc`. tsgo isn't bundled — install
  `@typescript/native-preview` as a devDependency in your project.
- **Svelte 5 only.** Handles all runes (`$state`, `$derived`, `$effect`,
  `$bindable`, `$props`, `$inspect`, `$host`), snippets, `{@attach}`,
  `{@const}`, `{@render}`, all `<svelte:*>` specials including
  `<svelte:boundary>`. No Svelte 4 prop syntax (`export let`), reactive
  statements (`$:`), slots (`<slot>`), or legacy event directives (`on:`).
- **SvelteKit aware.** `+page.svelte`, `+server.ts`, `+layout.ts`, etc. get
  their route-type injection automatically.

## What it is not

- An IDE extension. If you want autocomplete, hover, or go-to-definition, use
  the `svelte.svelte-vscode` extension (or your editor's equivalent) alongside
  this tool.
- A watch-mode daemon. This tool runs once and exits. For continuous checking,
  wrap it with a tool like [`watchexec`](https://github.com/watchexec/watchexec):
  ```sh
  watchexec -e svelte,ts,js -- svelte-check-native
  ```

## Install

*(will be published once v0.1 ships)*

```sh
npm i -D svelte-check-native

# or via cargo
cargo install svelte-check-native
```

## Usage

```sh
svelte-check-native                                     # check current workspace
svelte-check-native --workspace ./apps/admin            # check specific path
svelte-check-native --output machine                    # CI-friendly output
svelte-check-native --threshold error                   # hide warnings
svelte-check-native --fail-on-warnings                  # non-zero exit on warnings
svelte-check-native --diagnostic-sources ts,svelte      # filter sources
```

Run `svelte-check-native --help` for the full flag list.

## Development

This repo is a Cargo workspace. `language-tools/` is a git submodule pinned to
the upstream `svelte-check` we target for parity.

```sh
git clone --recurse-submodules <repo-url>
cd svelte-check-native
cargo build --release
cargo test
```

See `todo.md` for the full implementation plan and `CLAUDE.md` for working
conventions when using AI assistance on this codebase.

## License

MIT © Harsh Mandan
