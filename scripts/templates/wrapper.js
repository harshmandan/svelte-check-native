#!/usr/bin/env node
// svelte-check-native — wrapper that resolves the platform-specific
// binary package and execs it.
//
// The platform package (e.g. svelte-check-native-darwin-arm64) is
// installed automatically by npm via `optionalDependencies` on the
// main package — npm reads the platform package's `os` + `cpu` fields
// in its package.json and skips the ones that don't match the user's
// machine.
//
// We resolve the binary path by asking node to resolve the platform
// package's `package.json` (every package has one — guaranteed entry
// point that always exists) and then joining `bin/svelte-check-native`
// next to it. This sidesteps the `bin` field entirely so the platform
// package can ship the raw binary without npm trying to chmod / shim
// it.

const { spawnSync } = require('node:child_process');
const path = require('node:path');

function platformPackageName() {
  const platform = process.platform; // 'darwin' | 'linux' | 'win32' | ...
  const arch = process.arch;          // 'arm64' | 'x64' | ...
  return `svelte-check-native-${platform}-${arch}`;
}

function resolveBinary() {
  const pkg = platformPackageName();
  let pkgJsonPath;
  try {
    pkgJsonPath = require.resolve(`${pkg}/package.json`);
  } catch (_e) {
    const supported = [
      'darwin-arm64',
      'darwin-x64',
      'linux-arm64',
      'linux-x64',
      'win32-x64',
    ];
    process.stderr.write(
      `svelte-check-native: no prebuilt binary for ${process.platform}-${process.arch}.\n` +
        `Install one of: ${supported.map((s) => `svelte-check-native-${s}`).join(', ')}.\n` +
        `If npm did not install the matching platform package automatically,\n` +
        `you may have an unsupported OS/CPU combination.\n`,
    );
    process.exit(2);
  }
  const binName = process.platform === 'win32' ? 'svelte-check-native.exe' : 'svelte-check-native';
  return path.join(path.dirname(pkgJsonPath), 'bin', binName);
}

const binary = resolveBinary();
const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  process.stderr.write(`svelte-check-native: failed to execute binary at ${binary}: ${result.error.message}\n`);
  process.exit(2);
}
process.exit(result.status === null ? 1 : result.status);
