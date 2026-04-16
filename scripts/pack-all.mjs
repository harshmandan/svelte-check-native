// `npm pack` every package under npm/ into ./dist-packs/ for local
// smoke-testing the wrapper before a real publish.
//
// After this, you can verify end-to-end with:
//
//   mkdir -p /tmp/scn-test && cd /tmp/scn-test
//   npm init -y
//   npm i <repo>/dist-packs/svelte-check-native-<platform>-<arch>-*.tgz \
//         <repo>/dist-packs/svelte-check-native-0.1.0.tgz
//   npx svelte-check-native --help

import { execSync } from 'node:child_process';
import { mkdirSync, readdirSync, statSync, renameSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');
const npmRoot = join(repoRoot, 'npm');
const distDir = join(repoRoot, 'dist-packs');

mkdirSync(distDir, { recursive: true });

const packages = readdirSync(npmRoot).filter((entry) => {
  const p = join(npmRoot, entry);
  return statSync(p).isDirectory();
});

for (const pkg of packages) {
  const pkgDir = join(npmRoot, pkg);
  console.log(`packing ${pkg}...`);
  // `npm pack --pack-destination` lands the .tgz in the target dir directly.
  execSync(`npm pack --pack-destination "${distDir}"`, {
    cwd: pkgDir,
    stdio: 'inherit',
  });
}

console.log(`\npacked into ${distDir}`);
for (const f of readdirSync(distDir)) {
  console.log(`  ${f}`);
}
