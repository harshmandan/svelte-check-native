// `npm pack` every package under dist-packs/pkgs/ into dist-packs/ for
// local smoke-testing the wrapper before a real publish.
//
// After this, you can verify end-to-end with:
//
//   mkdir -p /tmp/scn-test && cd /tmp/scn-test
//   npm init -y
//   npm i <repo>/dist-packs/svelte-check-native-<platform>-<arch>-*.tgz \
//         <repo>/dist-packs/svelte-check-native-<version>.tgz
//   npx svelte-check-native --help
//
// Pipeline assumption: `npm run prepare-release` wrote the package
// dirs under dist-packs/pkgs/, and `npm run build:all` /
// `npm run build:native` populated each one's bin/<binary>.

import { execSync } from 'node:child_process';
import { mkdirSync, readdirSync, statSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');
const pkgsRoot = join(repoRoot, 'dist-packs', 'pkgs');
const distDir = join(repoRoot, 'dist-packs');

mkdirSync(distDir, { recursive: true });

let packages;
try {
  packages = readdirSync(pkgsRoot).filter((entry) => {
    const p = join(pkgsRoot, entry);
    return statSync(p).isDirectory();
  });
} catch (err) {
  if (err.code === 'ENOENT') {
    console.error(`dist-packs/pkgs/ is missing — run 'npm run prepare-release' first.`);
    process.exit(1);
  }
  throw err;
}

for (const pkg of packages) {
  const pkgDir = join(pkgsRoot, pkg);
  console.log(`packing ${pkg}...`);
  // `npm pack --pack-destination` lands the .tgz in the target dir directly.
  execSync(`npm pack --pack-destination "${distDir}"`, {
    cwd: pkgDir,
    stdio: 'inherit',
  });
}

console.log(`\npacked into ${distDir}`);
for (const f of readdirSync(distDir)) {
  if (f.endsWith('.tgz')) {
    console.log(`  ${f}`);
  }
}
