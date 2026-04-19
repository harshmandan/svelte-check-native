// scripts/prepare-release.mjs
//
// Single source of truth for every version number + README in the npm
// distribution layout. Run before `npm run pack` or before pushing a
// tag — the output lives entirely under gitignored `dist-packs/pkgs/`,
// so every run is equivalent to a clean regeneration.
//
// Outputs (ALL are regenerated on every run):
//   - dist-packs/pkgs/svelte-check-native/package.json         (main)
//   - dist-packs/pkgs/svelte-check-native/README.md            (mirror of /README.md)
//   - dist-packs/pkgs/svelte-check-native/bin/svelte-check-native.js  (copy of scripts/templates/wrapper.js)
//   - dist-packs/pkgs/svelte-check-native-<platform>/package.json   (× 5 platforms)
//
// Binaries (copied separately by `copy-binary.mjs` / `build-all.mjs`)
// land at `dist-packs/pkgs/svelte-check-native-<platform>/bin/<bin>`.
//
// Does NOT touch: crates/**/Cargo.toml (workspace inherits version
// from root Cargo.toml; bump that one instead), the hand-written JS
// wrapper at scripts/templates/wrapper.js.
//
// Usage:
//   node scripts/prepare-release.mjs               # derive version from Cargo.toml
//   node scripts/prepare-release.mjs 0.2.0         # override explicit version
//   node scripts/prepare-release.mjs --check       # dry-run: fail if dist-packs/pkgs/
//                                                    doesn't match the generator's output
//
// The `--check` mode is for CI — pairs with `pack-all.mjs` to catch
// stale generator output before tarballs get produced.

import { readFileSync, writeFileSync, mkdirSync, existsSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

import { TARGETS } from './targets.mjs';

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(__dirname, '..');

const ARGS = process.argv.slice(2);
const CHECK_MODE = ARGS.includes('--check');
const VERSION_ARG = ARGS.find((a) => !a.startsWith('--'));

const MAIN_PKG = 'svelte-check-native';
const PKGS_DIR = join(repoRoot, 'dist-packs', 'pkgs');
const WRAPPER_SRC = join(__dirname, 'templates', 'wrapper.js');

function parseWorkspaceVersion() {
  const cargo = readFileSync(join(repoRoot, 'Cargo.toml'), 'utf8');
  // Look for `version = "X.Y.Z"` within the [workspace.package] block.
  // Simple-enough parse: the workspace block starts with
  // `[workspace.package]` and every top-level key in that block sits
  // before the next `[...]` header.
  const block = cargo.split(/\n\[/).find((s) => s.startsWith('workspace.package]'));
  if (!block) throw new Error('Cargo.toml: no [workspace.package] block found');
  const m = block.match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error('Cargo.toml [workspace.package]: no version = "..." line');
  return m[1];
}

const version = VERSION_ARG ?? parseWorkspaceVersion();

// Basic sanity — npm won't accept `"0.2"` or `"v0.2.0"`.
if (!/^\d+\.\d+\.\d+([-+].+)?$/.test(version)) {
  console.error(`invalid version "${version}"; expected SemVer (e.g. 0.2.0)`);
  process.exit(2);
}

// ---- templates --------------------------------------------------------

function mainPackageJson(v) {
  return {
    name: MAIN_PKG,
    version: v,
    description: `Fast CLI type-checker for Svelte 4 and Svelte 5 projects. Drop-in replacement for svelte-check, written in Rust, powered by tsgo.`,
    keywords: [
      'svelte',
      'sveltekit',
      'svelte-check',
      'rust',
      'native',
      'typescript-native',
      'tsgo',
      'oxc',
      'linter',
      'parser',
      'dx',
    ],
    homepage: `https://github.com/harshmandan/${MAIN_PKG}`,
    bugs: { url: `https://github.com/harshmandan/${MAIN_PKG}/issues` },
    repository: {
      type: 'git',
      url: `https://github.com/harshmandan/${MAIN_PKG}.git`,
    },
    license: 'MIT',
    author: 'Harsh Mandan',
    bin: { [MAIN_PKG]: `bin/${MAIN_PKG}.js` },
    files: [`bin/${MAIN_PKG}.js`, 'README.md', 'LICENSE'],
    engines: { node: '>=18' },
    optionalDependencies: Object.fromEntries(
      TARGETS.map((t) => [`${MAIN_PKG}-${t.npmPlatform}`, v]),
    ),
    peerDependencies: { '@typescript/native-preview': '>=7.0.0-dev.0' },
    peerDependenciesMeta: {
      '@typescript/native-preview': { optional: false },
    },
  };
}

function platformPackageJson(target, v) {
  const [os, arch] = target.npmPlatform.split('-');
  return {
    name: `${MAIN_PKG}-${target.npmPlatform}`,
    version: v,
    description: `${target.npmPlatform} binary for ${MAIN_PKG}. Do not install directly — depend on \`${MAIN_PKG}\` instead.`,
    homepage: `https://github.com/harshmandan/${MAIN_PKG}`,
    repository: {
      type: 'git',
      url: `https://github.com/harshmandan/${MAIN_PKG}.git`,
    },
    license: 'MIT',
    author: 'Harsh Mandan',
    files: [`bin/${target.binName}`],
    os: [os],
    cpu: [arch],
    engines: { node: '>=18' },
  };
}

// ---- emit / check -----------------------------------------------------

const mismatches = [];

function writeJson(path, obj) {
  const next = JSON.stringify(obj, null, 2) + '\n';
  if (CHECK_MODE) {
    const current = existsSync(path) ? readFileSync(path, 'utf8') : '';
    if (current !== next) {
      mismatches.push(path);
    }
    return;
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, next);
}

function copyFile(src, dst) {
  const text = readFileSync(src, 'utf8');
  if (CHECK_MODE) {
    const current = existsSync(dst) ? readFileSync(dst, 'utf8') : '';
    if (current !== text) {
      mismatches.push(dst);
    }
    return;
  }
  mkdirSync(dirname(dst), { recursive: true });
  writeFileSync(dst, text);
}

// Main package
const mainPkgDir = join(PKGS_DIR, MAIN_PKG);
writeJson(join(mainPkgDir, 'package.json'), mainPackageJson(version));
copyFile(join(repoRoot, 'README.md'), join(mainPkgDir, 'README.md'));
copyFile(WRAPPER_SRC, join(mainPkgDir, 'bin', `${MAIN_PKG}.js`));

// Platform packages
for (const target of TARGETS) {
  const pkgDir = join(PKGS_DIR, `${MAIN_PKG}-${target.npmPlatform}`);
  writeJson(join(pkgDir, 'package.json'), platformPackageJson(target, version));
}

if (CHECK_MODE) {
  if (mismatches.length) {
    console.error(
      `[prepare-release] ${mismatches.length} file(s) out of sync with version ${version}:`,
    );
    for (const m of mismatches) {
      console.error('  ' + m.replace(repoRoot + '/', ''));
    }
    console.error(`run 'node scripts/prepare-release.mjs' to regenerate.`);
    process.exit(1);
  }
  console.log(`[prepare-release] dist-packs/pkgs/ is in sync at ${version}`);
} else {
  console.log(`[prepare-release] regenerated dist-packs/pkgs/ at ${version}`);
}
