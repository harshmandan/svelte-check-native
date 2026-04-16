// Copy the host's freshly-built `target/release/<bin>` into the
// matching npm platform package's bin/. Used by `npm run build:native`
// for fast single-target iteration on the dev's own machine.
//
// For cross-platform builds use `npm run build:all` instead — that
// uses cargo-zigbuild and the targets.mjs map to handle every target.

import { copyFileSync, chmodSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TARGETS } from './targets.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');

const hostNpmPlatform = `${process.platform}-${process.arch}`;
const target = TARGETS.find((t) => t.npmPlatform === hostNpmPlatform);
if (!target) {
  console.error(`no platform package for host ${hostNpmPlatform}.`);
  console.error(`known platforms: ${TARGETS.map((t) => t.npmPlatform).join(', ')}`);
  process.exit(1);
}

const sourceBin = join(repoRoot, 'target', 'release', target.binName);
const destDir = join(repoRoot, 'npm', `svelte-check-native-${target.npmPlatform}`, 'bin');
const destBin = join(destDir, target.binName);

if (!existsSync(sourceBin)) {
  console.error(`source binary missing: ${sourceBin}`);
  console.error('run `cargo build --release -p svelte-check-native` first.');
  process.exit(1);
}
if (!existsSync(destDir)) {
  mkdirSync(destDir, { recursive: true });
}
copyFileSync(sourceBin, destBin);
if (!target.binName.endsWith('.exe')) {
  chmodSync(destBin, 0o755);
}
console.log(`copied ${sourceBin} → ${destBin}`);
