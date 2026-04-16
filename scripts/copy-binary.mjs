// Copy the freshly-built Rust binary into the platform package matching
// the host machine. Runs after `cargo build --release`.
//
// Cross-platform builds (CI) should hand-pick the right target triple
// per platform package and copy each into its directory; this script
// only handles the local-host case for `npm run build:native`.

import { copyFileSync, chmodSync, existsSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');

function platformPackageDir() {
  const platform = process.platform;
  const arch = process.arch;
  return `svelte-check-native-${platform}-${arch}`;
}

const isWin = process.platform === 'win32';
const binName = isWin ? 'svelte-check-native.exe' : 'svelte-check-native';

const sourceBin = join(repoRoot, 'target', 'release', binName);
const destDir = join(repoRoot, 'npm', platformPackageDir(), 'bin');
const destBin = join(destDir, binName);

if (!existsSync(sourceBin)) {
  console.error(`source binary missing: ${sourceBin}`);
  console.error('run `cargo build --release -p svelte-check-native` first.');
  process.exit(1);
}
if (!existsSync(destDir)) {
  mkdirSync(destDir, { recursive: true });
}
copyFileSync(sourceBin, destBin);
if (!isWin) {
  chmodSync(destBin, 0o755);
}
console.log(`copied ${sourceBin} → ${destBin}`);
