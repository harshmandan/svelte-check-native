// Single source of truth: the mapping between Rust target triples
// (what cargo + cargo-zigbuild understand) and npm platform names
// (what package.json `os`/`cpu` use, and what the wrapper script
// resolves at runtime via `${platform}-${arch}`).
//
// The wrapper at npm/svelte-check-native/bin/svelte-check-native.js
// computes `svelte-check-native-${process.platform}-${process.arch}`
// and `require.resolve`s its package.json — so npmPlatform here MUST
// match `${process.platform}-${process.arch}` for each target.

export const TARGETS = [
  {
    rustTarget: 'aarch64-apple-darwin',
    npmPlatform: 'darwin-arm64',
    binName: 'svelte-check-native',
    nativeOnly: false, // builds via cargo on macOS without zigbuild
  },
  {
    rustTarget: 'x86_64-apple-darwin',
    npmPlatform: 'darwin-x64',
    binName: 'svelte-check-native',
    nativeOnly: false,
  },
  {
    rustTarget: 'aarch64-unknown-linux-gnu',
    npmPlatform: 'linux-arm64',
    binName: 'svelte-check-native',
    nativeOnly: false,
  },
  {
    rustTarget: 'x86_64-unknown-linux-gnu',
    npmPlatform: 'linux-x64',
    binName: 'svelte-check-native',
    nativeOnly: false,
  },
  {
    rustTarget: 'x86_64-pc-windows-gnu',
    npmPlatform: 'win32-x64',
    binName: 'svelte-check-native.exe',
    nativeOnly: false,
  },
];
