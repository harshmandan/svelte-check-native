# svelte-check-native

Fast CLI type-checker for Svelte 5+ projects. Drop-in replacement for
`svelte-check`. See the [project README](https://github.com/harshmandan/svelte-check-native#readme)
for full docs.

## Install

```sh
npm i -D svelte-check-native @typescript/native-preview
```

The right prebuilt binary for your OS/CPU is fetched automatically via
the platform-specific package (one of `svelte-check-native-darwin-arm64`,
`-darwin-x64`, `-linux-arm64`, `-linux-x64`, `-win32-x64`).

## Use

```sh
npx svelte-check-native --workspace .
```

Same flags as upstream `svelte-check`. Run `svelte-check-native --help`
for the full list.
