#!/usr/bin/env node
// scripts/bootstrap-kit-bench.mjs
//
// Bootstrap a SvelteKit test-app workspace as a bench target. Clones
// `sveltejs/kit` shallowly into bench/_kit/, runs the package
// manager's install at the monorepo root (so the workspace's
// `@sveltejs/kit` resolves), then prints the path to one specific
// test app for use with `node scripts/bench.mjs --target …`.
//
// `bench/_kit/` is gitignored along with the rest of `bench/*` (per
// `.gitignore` rule `bench/*` + the single re-include for
// `bench/.parity-exceptions.json`). This is a DEV-LOCAL bootstrap
// — the resulting workspace stays out of the tracked tree.
//
// Why dev-local: the kit monorepo is large (~150MB source-only), needs
// pnpm to install, and produces a node_modules tree that wouldn't
// belong in our repo regardless. Cloning on demand keeps the parity
// review-time and skips the install when not needed.
//
// Usage
//   # First-time bootstrap. Defaults to the kit `main` branch's HEAD;
//   # pin via $KIT_REF to a commit / tag / branch.
//   node scripts/bootstrap-kit-bench.mjs
//   KIT_REF=v2.16.0 node scripts/bootstrap-kit-bench.mjs
//
//   # After bootstrap, run bench.mjs against a chosen test app:
//   node scripts/bench.mjs \
//     --target bench/_kit/packages/kit/test/apps/basics \
//     --mode parity --diagnostic-detail
//
// Flags
//   --app <name>      Print the path of just this app and exit (no
//                     re-bootstrap). Useful in CI scripts.
//   --refresh         Re-fetch the upstream ref into the existing
//                     clone (skips the install step unless
//                     `--reinstall` also passed).
//   --reinstall       Re-run the package-manager install. Implied on
//                     first bootstrap.
//
// Exit codes: 0 success, 2 bad invocation, 1 anything else.

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const argv = process.argv.slice(2);
const args = parseArgs(argv);
if (args.error) fail(args.error);

const repoRoot = resolve(import.meta.dirname, '..');
const kitDir = join(repoRoot, 'bench/_kit');
const ref = process.env.KIT_REF ?? 'main';

if (args.app && !args.refresh && !args.reinstall && existsSync(join(kitDir, '.git'))) {
    // Fast path: just resolve the path and exit.
    printAppPath(args.app);
    process.exit(0);
}

if (!existsSync(kitDir)) {
    mkdirSync(kitDir, { recursive: true });
    run('git', [
        'clone',
        '--filter=blob:none',
        '--depth=1',
        '--branch', ref === 'main' ? 'main' : ref,
        'https://github.com/sveltejs/kit.git',
        kitDir,
    ]);
} else if (args.refresh) {
    run('git', ['-C', kitDir, 'fetch', 'origin', ref, '--depth=1']);
    run('git', ['-C', kitDir, 'checkout', '--detach', `origin/${ref.replace(/^origin\//, '')}`]);
}

const wantsInstall = args.reinstall
    || !existsSync(join(kitDir, 'node_modules'));
if (wantsInstall) {
    // sveltejs/kit uses pnpm. Don't fall back to npm — the workspace
    // layout depends on pnpm-workspace.yaml resolution.
    const pnpm = spawnSync('pnpm', ['--version'], { stdio: 'ignore' });
    if (pnpm.status !== 0) {
        fail('pnpm required but not found on PATH. Install via `npm i -g pnpm`.');
    }
    run('pnpm', ['install', '--frozen-lockfile=false'], { cwd: kitDir });
}

if (args.app) {
    printAppPath(args.app);
} else {
    printAvailableApps();
}

function printAppPath(name) {
    const appDir = join(kitDir, 'packages/kit/test/apps', name);
    if (!existsSync(appDir)) {
        const apps = listApps();
        fail(
            `app "${name}" not found. Available apps:\n  ${apps.join('\n  ')}`
        );
    }
    // SvelteKit's tsconfig extends from `./.svelte-kit/tsconfig.json`,
    // which `svelte-kit sync` generates from the routes tree. Without
    // it, every `+page.svelte` reports `Cannot find module './$types'`
    // and the bench's diagnostics-list compares apples-to-oranges.
    // Run sync once per app on demand so the bench target is in a
    // diagnostically-clean baseline state.
    const syncMarker = join(appDir, '.svelte-kit/tsconfig.json');
    if (!existsSync(syncMarker)) {
        // Kit test apps wire `svelte-kit sync` under either a
        // `sync` or `prepare` package-script entry depending on the
        // pinned monorepo version. Try `sync` first, fall back to
        // `prepare`. If neither exists, fall through to direct
        // `svelte-kit sync` against the workspace install.
        console.error(`# Running svelte-kit sync in ${appDir} (one-time setup)…`);
        const tries = [['pnpm', ['run', 'sync']], ['pnpm', ['run', 'prepare']]];
        let synced = false;
        for (const [cmd, runArgs] of tries) {
            const r = spawnSync(cmd, runArgs, { stdio: 'inherit', cwd: appDir });
            if (r.status === 0) {
                synced = true;
                break;
            }
        }
        if (!synced) {
            fail(`could not run svelte-kit sync in ${appDir} — neither \`sync\` nor \`prepare\` script worked`);
        }
    }
    process.stdout.write(`${appDir}\n`);
}

function printAvailableApps() {
    const apps = listApps();
    console.log('SvelteKit test-app bench targets bootstrapped:');
    for (const name of apps) {
        console.log(`  - bench/_kit/packages/kit/test/apps/${name}`);
    }
    console.log('\nRun a parity bench with:');
    console.log(
        '  node scripts/bench.mjs \\\n'
        + '    --target bench/_kit/packages/kit/test/apps/<name> \\\n'
        + '    --mode parity --diagnostic-detail'
    );
}

function listApps() {
    const appsDir = join(kitDir, 'packages/kit/test/apps');
    if (!existsSync(appsDir)) {
        fail(`expected ${appsDir} to exist after bootstrap — kit layout may have changed`);
    }
    return readdirSync(appsDir, { withFileTypes: true })
        .filter(d => d.isDirectory() && !d.name.startsWith('.'))
        .map(d => d.name)
        .sort();
}

function run(cmd, runArgs, opts = {}) {
    const result = spawnSync(cmd, runArgs, { stdio: 'inherit', ...opts });
    if (result.status !== 0) {
        fail(`${cmd} ${runArgs.join(' ')} exited ${result.status}`);
    }
}

function parseArgs(a) {
    const out = { app: null, refresh: false, reinstall: false };
    for (let i = 0; i < a.length; i++) {
        const v = a[i];
        switch (v) {
            case '--app': out.app = a[++i]; break;
            case '--refresh': out.refresh = true; break;
            case '--reinstall': out.reinstall = true; break;
            case '--help':
            case '-h': printHelp(); process.exit(0);
            default: return { error: `unknown flag: ${v}` };
        }
    }
    return out;
}

function printHelp() {
    // Match scripts/bench.mjs's printHelp shape — show the file's
    // top comment block.
    const text = readFileSync(import.meta.filename, 'utf8')
        .split('\n')
        .slice(2)
        .filter(l => l.startsWith('//'))
        .map(l => l.replace(/^\/\/ ?/, ''))
        .join('\n');
    console.log(text);
}

function fail(msg) {
    console.error(`bootstrap-kit-bench: ${msg}`);
    process.exit(typeof msg === 'string' && msg.startsWith('unknown flag') ? 2 : 1);
}
