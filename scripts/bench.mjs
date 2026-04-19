#!/usr/bin/env node
// scripts/bench.mjs
//
// Cold / warm / dirty benchmark harness for svelte-check-native.
//
// Measures three scenarios on the same workspace:
//   cold  — cache wiped before the run
//   warm  — second run right after cold (tsbuildinfo + overlay cache
//           fully populated)
//   dirty — touch one .svelte file to invalidate tsgo's incremental
//           state, then re-run warm-style (simulates the "edit one
//           file, re-check" loop)
//
// Runs the built release binary at target/release/svelte-check-native
// against $BENCH_TARGET. The bench target is NOT hardcoded here — pass
// it via the env var (or --target) so swapping control projects is one
// env change. See CLAUDE.md for the default target this repo benches
// against.
//
// Usage
//   BENCH_TARGET=bench/some-project node scripts/bench.mjs
//   node scripts/bench.mjs --target bench/some-project
//   node scripts/bench.mjs --target bench/some-project --csv out.csv
//   node scripts/bench.mjs --target bench/some-project --runs 5
//
// Flags
//   --target <path>   Workspace to bench (defaults to $BENCH_TARGET).
//   --csv <path>      Write CSV rows to file (otherwise stdout only).
//   --runs <N>        Per-scenario sample count, median reported (default 3).
//   --tsconfig <path> Pass through to the binary.
//   --quiet           Suppress per-run timing lines (CSV still prints).
//
// Exit 0 on success; 2 on bad invocation.

import { execFileSync } from 'node:child_process';
import { rmSync, existsSync, statSync, utimesSync, readdirSync, readFileSync, writeFileSync, appendFileSync } from 'node:fs';
import { join, resolve, isAbsolute } from 'node:path';
import { performance } from 'node:perf_hooks';

const argv = process.argv.slice(2);
const args = parseArgs(argv);
if (args.error) fail(args.error);

const target = args.target ?? process.env.BENCH_TARGET;
if (!target) fail('no bench target — set $BENCH_TARGET or pass --target <path>');

const targetAbs = isAbsolute(target) ? target : resolve(process.cwd(), target);
if (!existsSync(targetAbs)) fail(`target does not exist: ${targetAbs}`);
if (!statSync(targetAbs).isDirectory()) fail(`target is not a directory: ${targetAbs}`);

const repoRoot = resolve(import.meta.dirname, '..');
const binary = join(repoRoot, 'target/release/svelte-check-native');
if (!existsSync(binary)) fail(`binary not found — run \`cargo build --release\` first (${binary})`);

const cacheDirs = [
    join(targetAbs, 'node_modules/.cache/svelte-check-native'),
    join(targetAbs, '.svelte-check'),
];

// Samples per scenario; we take the median so one-off jitter doesn't win.
const runs = args.runs ?? 3;
const extraArgs = [];
if (args.tsconfig) extraArgs.push('--tsconfig', args.tsconfig);

// Scenarios in order. `setup` primes the state; `measure` is the thing
// whose wall-clock we record.
const scenarios = [
    {
        name: 'cold',
        setup: () => wipeCaches(),
        measure: () => runBinary(),
    },
    {
        name: 'warm',
        setup: () => {/* no-op: previous run left cache in place */},
        measure: () => runBinary(),
    },
    {
        name: 'dirty',
        setup: () => touchOneSvelteFile(targetAbs),
        measure: () => runBinary(),
    },
];

const results = [];
for (const s of scenarios) {
    const samples = [];
    for (let i = 0; i < runs; i++) {
        s.setup();
        const t = measure(() => s.measure());
        samples.push(t);
        if (!args.quiet) console.error(`  ${s.name} run ${i + 1}/${runs}: ${t.toFixed(2)}s`);
    }
    samples.sort((a, b) => a - b);
    const median = samples[Math.floor(samples.length / 2)];
    results.push({ scenario: s.name, median, samples });
}

// Emit CSV. Header first, then one row per scenario.
const csvLines = [
    'scenario,median_seconds,runs,target',
    ...results.map(r => `${r.scenario},${r.median.toFixed(3)},${r.samples.map(s => s.toFixed(3)).join('|')},${JSON.stringify(targetAbs)}`),
];

const csvText = csvLines.join('\n') + '\n';
if (args.csv) {
    writeFileSync(args.csv, csvText);
    if (!args.quiet) console.error(`wrote ${args.csv}`);
}
process.stdout.write(csvText);

// ---

function runBinary() {
    const out = execFileSync(
        binary,
        ['--workspace', targetAbs, '--output', 'machine', ...extraArgs],
        { stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 256 * 1024 * 1024 },
    );
    // Sanity: require the COMPLETED line so we catch silent failures.
    if (!/\bCOMPLETED\b/.test(out.toString('utf8'))) {
        throw new Error('binary exited cleanly but produced no COMPLETED line — likely crash');
    }
}

function measure(fn) {
    const start = performance.now();
    try { fn(); }
    catch (err) {
        // svelte-check-native exits 1 when errors are found — that's
        // still a successful bench; the wall-clock measurement is valid.
        // exit 2 (invocation error) propagates.
        if (err.status === 1) {
            // fine
        } else {
            throw err;
        }
    }
    return (performance.now() - start) / 1000;
}

function wipeCaches() {
    for (const d of cacheDirs) {
        try { rmSync(d, { recursive: true, force: true }); } catch {}
    }
}

function touchOneSvelteFile(root) {
    const svelte = findFirstSvelteFile(root);
    if (!svelte) throw new Error(`no .svelte file found under ${root} — dirty scenario needs at least one`);
    const now = new Date();
    utimesSync(svelte, now, now);
}

function findFirstSvelteFile(root) {
    // BFS that skips common "not user source" directories to avoid
    // walking node_modules tails in a monorepo. Returns the first match.
    const skip = new Set([
        'node_modules', '.svelte-kit', '.svelte-check', 'target',
        'dist', 'build', '.git', '.next', '.turbo',
    ]);
    const queue = [root];
    while (queue.length) {
        const dir = queue.shift();
        let entries;
        try { entries = readdirSync(dir, { withFileTypes: true }); } catch { continue; }
        for (const e of entries) {
            if (e.isDirectory()) {
                if (!skip.has(e.name) && !e.name.startsWith('.')) queue.push(join(dir, e.name));
            } else if (e.name.endsWith('.svelte')) {
                return join(dir, e.name);
            }
        }
    }
    return null;
}

function parseArgs(a) {
    const out = { target: null, csv: null, runs: null, tsconfig: null, quiet: false };
    for (let i = 0; i < a.length; i++) {
        const v = a[i];
        switch (v) {
            case '--target': out.target = a[++i]; break;
            case '--csv': out.csv = a[++i]; break;
            case '--runs': out.runs = Number(a[++i]); break;
            case '--tsconfig': out.tsconfig = a[++i]; break;
            case '--quiet': out.quiet = true; break;
            case '--help':
            case '-h': printHelp(); process.exit(0);
            default: return { error: `unknown flag: ${v}` };
        }
    }
    if (out.runs !== null && (!Number.isInteger(out.runs) || out.runs < 1)) {
        return { error: '--runs must be a positive integer' };
    }
    return out;
}

function printHelp() {
    console.log(readFileSync(import.meta.filename, 'utf8')
        .split('\n')
        .slice(2)
        .filter(l => l.startsWith('//'))
        .map(l => l.replace(/^\/\/ ?/, ''))
        .join('\n'));
}

function fail(msg) {
    console.error(`bench.mjs: ${msg}`);
    process.exit(2);
}
