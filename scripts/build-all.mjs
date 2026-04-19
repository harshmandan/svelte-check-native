// Cross-build the rust binary for every target listed in targets.mjs
// using cargo-zigbuild (which uses Zig as the C linker so we don't
// need per-platform native toolchains).
//
// Each target's output binary is copied into its corresponding npm
// platform package's bin/ directory. After this completes, every
// npm/svelte-check-native-<plat>/bin/<bin> exists and is ready for
// `npm pack`.
//
// Usage:
//   node scripts/build-all.mjs              # build all targets
//   node scripts/build-all.mjs <triple>     # build just one target
//
// Requires: zig, cargo-zigbuild, and the relevant rustup targets.

import { execSync } from 'node:child_process';
import { copyFileSync, chmodSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TARGETS } from './targets.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');

const requested = process.argv[2];
const list = requested ? TARGETS.filter((t) => t.rustTarget === requested) : TARGETS;
if (requested && list.length === 0) {
  console.error(`unknown target: ${requested}`);
  console.error(`known targets: ${TARGETS.map((t) => t.rustTarget).join(', ')}`);
  process.exit(1);
}

for (const t of list) {
  console.log(`\n=== ${t.rustTarget} → ${t.npmPlatform} ===`);

  // cargo-zigbuild has the same surface as `cargo build`, just swaps
  // the linker. Native targets (matching the host) work fine through
  // it too, so we don't special-case them.
  execSync(`cargo zigbuild --release --target ${t.rustTarget} -p svelte-check-native`, {
    cwd: repoRoot,
    stdio: 'inherit',
  });

  const sourceBin = join(repoRoot, 'target', t.rustTarget, 'release', t.binName);
  if (!existsSync(sourceBin)) {
    console.error(`built binary missing at ${sourceBin}`);
    process.exit(1);
  }

  const destDir = join(
    repoRoot,
    'dist-packs',
    'pkgs',
    `svelte-check-native-${t.npmPlatform}`,
    'bin',
  );
  if (!existsSync(destDir)) {
    mkdirSync(destDir, { recursive: true });
  }
  const destBin = join(destDir, t.binName);
  copyFileSync(sourceBin, destBin);
  if (!t.binName.endsWith('.exe')) {
    chmodSync(destBin, 0o755);
  }
  console.log(`copied → ${destBin}`);
}

console.log(`\ndone — built ${list.length} target(s).`);
