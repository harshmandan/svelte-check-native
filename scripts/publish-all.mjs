// Publish every npm package — platform packages first, then the wrapper.
//
// Order matters: the main `svelte-check-native` package pins each
// platform package as an exact-version `optionalDependencies` entry, so
// the platforms have to exist on the registry before the wrapper is
// published. Otherwise `npm install svelte-check-native` succeeds but
// the runtime `require.resolve` of the platform-specific binary fails.
//
// Fails fast: if any single `npm publish` call exits non-zero, the
// script aborts. That leaves the registry in a partial state but
// surfaces the problem immediately — better than silently continuing.
//
// Usage (all packages must already be at the same version, binaries
// already built via `node scripts/build-all.mjs`):
//
//   node scripts/publish-all.mjs
//   node scripts/publish-all.mjs --dry-run    # pack + validate, no registry write
//
// Requires: a logged-in `npm` session with publish rights
// (`npm whoami` returns a user).

import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TARGETS } from './targets.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');
const dryRun = process.argv.includes('--dry-run');

const pkgsRoot = join(repoRoot, 'dist-packs', 'pkgs');
const wrapperDir = join(pkgsRoot, 'svelte-check-native');
const platformDirs = TARGETS.map((t) =>
  join(pkgsRoot, `svelte-check-native-${t.npmPlatform}`),
);

// Sanity: all six packages must be pinned to the same version. If the
// main package's optionalDependencies disagree with any platform
// package's own version, the wrapper will install but fail at runtime.
// Catch it locally before the registry sees a mismatched set.
const wrapperPkg = JSON.parse(readFileSync(join(wrapperDir, 'package.json'), 'utf8'));
const targetVersion = wrapperPkg.version;
if (!targetVersion) {
  console.error(`wrapper package.json missing "version" at ${wrapperDir}`);
  process.exit(1);
}
for (const dir of platformDirs) {
  const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  if (pkg.version !== targetVersion) {
    console.error(
      `version mismatch: wrapper=${targetVersion}, ${pkg.name}=${pkg.version} (${dir})`,
    );
    process.exit(1);
  }
  const pinned = wrapperPkg.optionalDependencies?.[pkg.name];
  if (pinned !== targetVersion) {
    console.error(
      `wrapper's optionalDependencies.${pkg.name} = ${pinned}, expected ${targetVersion}`,
    );
    process.exit(1);
  }
}

function publish(dir) {
  const name = dir.split('/').pop();
  const header = dryRun ? `--- (dry-run) ${name}` : `--- publishing ${name}`;
  console.log(`\n${header}`);
  const args = ['publish', '--access', 'public'];
  if (dryRun) args.push('--dry-run');
  const result = spawnSync('npm', args, { cwd: dir, stdio: 'inherit' });
  if (result.status !== 0) {
    console.error(`\nnpm publish failed in ${dir} with exit code ${result.status}.`);
    if (!dryRun) {
      console.error(
        'Any earlier platform packages that already published are now live; ' +
          'the wrapper package has NOT been published. Re-run this script after ' +
          'resolving the failure to finish the release.',
      );
    }
    process.exit(result.status ?? 1);
  }
}

// Platforms first, wrapper last — enforced order.
for (const dir of platformDirs) {
  publish(dir);
}
publish(wrapperDir);

console.log(
  `\n${dryRun ? '(dry-run)' : 'done'} — ${platformDirs.length + 1} packages at ${targetVersion}`,
);
