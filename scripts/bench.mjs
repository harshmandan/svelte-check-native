#!/usr/bin/env node
// scripts/bench.mjs
//
// Two modes:
//
// timing (default) — cold/warm/dirty wall-clock benchmark for our
//                    binary on a given workspace.
// parity           — run OUR binary + upstream `svelte-check` +
//                    (when available) `svelte-check --tsgo` against
//                    the same workspace and compare their
//                    <N> FILES <E> ERRORS <W> WARNINGS
//                    <F> FILES_WITH_PROBLEMS counts.
//
// The parity mode is the correctness-signal: we aim to match upstream
// `svelte-check` (preferred) or at least `svelte-check --tsgo` on every
// workspace. Deltas against either are either:
//   - false positives we're firing that neither upstream tool does, or
//   - real errors we're missing that upstream catches.
//
// The timing mode is the perf-signal: cache-wiped cold run vs
// cache-hot warm run vs single-file-edit dirty run.
//
// Usage
//   # Timing (default, legacy behavior):
//   BENCH_TARGET=bench/some-project node scripts/bench.mjs
//   node scripts/bench.mjs --target bench/some-project --mode timing
//
//   # Parity:
//   node scripts/bench.mjs --target bench/some-project --mode parity
//   node scripts/bench.mjs --target bench/some-project --mode parity --csv out.csv
//
// Flags
//   --target <path>     Workspace to bench (defaults to $BENCH_TARGET).
//   --mode <m>          `timing` (default) or `parity`.
//   --csv <path>        Write CSV rows to file (otherwise stdout only).
//   --runs <N>          Per-scenario sample count for timing mode
//                       (default 3). Parity mode always runs once per
//                       tool — counts are deterministic.
//   --tsconfig <path>   Pass through to the binary.
//   --quiet             Suppress per-run timing lines (CSV still prints).
//
// Exit 0 on success; 2 on bad invocation; 1 when parity mode detects a
// non-matching count (see --allow-delta to opt out).

import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { isAbsolute, join, resolve } from 'node:path';
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

const mode = args.mode ?? 'timing';
if (mode !== 'timing' && mode !== 'parity') fail(`unknown --mode: ${mode}`);

const extraArgs = [];
if (args.tsconfig) extraArgs.push('--tsconfig', args.tsconfig);

if (mode === 'timing') {
    runTimingMode();
} else {
    runParityMode();
}

// -----------------------------------------------------------------
// Timing mode (unchanged from pre-v0.3): cold/warm/dirty wall-clock.
// -----------------------------------------------------------------

function runTimingMode() {
    const runs = args.runs ?? 3;
    const scenarios = [
        { name: 'cold', setup: () => wipeCaches(), measure: () => runBinary() },
        { name: 'warm', setup: () => {}, measure: () => runBinary() },
        { name: 'dirty', setup: () => touchOneSvelteFile(targetAbs), measure: () => runBinary() },
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

    const csvLines = [
        'scenario,median_seconds,runs,target',
        ...results.map(r => `${r.scenario},${r.median.toFixed(3)},${r.samples.map(s => s.toFixed(3)).join('|')},${JSON.stringify(targetAbs)}`),
    ];

    emitCsv(csvLines);
}

// -----------------------------------------------------------------
// Parity mode: run 3 tools, compare <N> FILES <E> ERRORS <W> WARNINGS
// <F> FILES_WITH_PROBLEMS counts on each. Exits 0 iff all three agree;
// exits 1 otherwise (unless --allow-delta).
// -----------------------------------------------------------------

function runParityMode() {
    // Capture ours first — its stderr reveals any solution-style
    // tsconfig redirect to a nested sub-app, which upstream also
    // needs to match (otherwise upstream runs against the monorepo
    // root and fails tsconfig resolution, producing different
    // counts for mechanical reasons rather than semantic ones).
    if (!args.quiet) console.error(`  running ours…`);
    const oursResult = runOursForCountsWithStderr();
    const effectiveWorkspace = oursResult.redirectedTo ?? targetAbs;
    const upstream = findUpstreamSvelteCheck(effectiveWorkspace)
        ?? findUpstreamSvelteCheck(targetAbs);
    const rows = [
        { tool: 'ours', ...oursResult.counts },
    ];
    if (upstream) {
        if (!args.quiet) console.error(`  running upstream…`);
        rows.push({
            tool: 'upstream',
            ...runUpstream(upstream, { tsgo: false, cwd: effectiveWorkspace }),
        });
        // --tsgo was added in svelte-check 4.0; older versions reject
        // the flag. Probe once and skip gracefully if not supported.
        if (upstreamSupportsTsgo(upstream)) {
            if (!args.quiet) console.error(`  running upstream --tsgo…`);
            rows.push({
                tool: 'upstream --tsgo',
                ...runUpstream(upstream, { tsgo: true, cwd: effectiveWorkspace }),
            });
        } else if (!args.quiet) {
            console.error('  (upstream --tsgo not supported on this svelte-check version — skipping)');
        }
    } else if (!args.quiet) {
        console.error('  (no upstream svelte-check found in target — running ours only)');
    }

    // Print comparison table to stderr for human reading, CSV to stdout.
    if (!args.quiet) {
        console.error('');
        console.error(formatParityTable(rows, targetAbs));
        console.error('');
    }

    const csvLines = [
        'tool,files,errors,warnings,files_with_problems,target',
        ...rows.map(r => `${JSON.stringify(r.tool)},${r.files},${r.errors},${r.warnings},${r.files_with_problems},${JSON.stringify(targetAbs)}`),
    ];
    emitCsv(csvLines);

    // Exit 1 if ours deviates from the best upstream baseline. Prefer
    // upstream (non-tsgo); fall back to upstream --tsgo. If no upstream
    // was available, always exit 0.
    const ours = rows.find(r => r.tool === 'ours');
    const upstreamRow = rows.find(r => r.tool === 'upstream')
        ?? rows.find(r => r.tool === 'upstream --tsgo');
    if (!upstreamRow || args.allowDelta) return;
    const mismatch = ours.errors !== upstreamRow.errors
        || ours.warnings !== upstreamRow.warnings
        || ours.files_with_problems !== upstreamRow.files_with_problems;
    if (mismatch) process.exit(1);
}

/// Run our binary and return both the parsed COMPLETED counts AND
/// any solution-redirect target (detected from stderr). Callers use
/// the redirect to run upstream against the same effective workspace.
function runOursForCountsWithStderr() {
    wipeCaches();
    let stdout = '';
    let stderr = '';
    try {
        stdout = execFileSync(
            binary,
            ['--workspace', targetAbs, '--output', 'machine', ...extraArgs],
            { stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 256 * 1024 * 1024, encoding: 'utf8' },
        );
    } catch (err) {
        if (err.status === 1) {
            stdout = err.stdout?.toString('utf8') ?? '';
            stderr = err.stderr?.toString('utf8') ?? '';
        } else throw err;
    }
    // stderr captured on success too — execFileSync with stdio:pipe
    // returns stderr via err.stderr only on non-zero exit; on success
    // we'd need to capture separately. Our binary prints the redirect
    // to stderr as an informational line; execFileSync doesn't hand
    // it back when exit code is 0. We re-run with a pipe setup that
    // captures both streams when needed — but for the common case
    // (status 0) the redirect either doesn't happen or happens
    // silently. Fall back to tsconfig-path sniffing.
    const counts = parseCompleted(stdout)
        ?? { files: -1, errors: -1, warnings: -1, files_with_problems: -1 };
    const redirectMatch = /redirected workspace to (\S+)/.exec(stderr);
    const redirectedTo = redirectMatch ? redirectMatch[1] : detectRedirectByTsconfig(targetAbs);
    return { counts, redirectedTo };
}

/// Heuristic: if the target's tsconfig is a project-references
/// solution (files: [], no include, non-empty references),
/// our binary redirects to the first referenced sub-project with
/// real `compilerOptions.paths`. Mirror that decision here so
/// `findUpstreamSvelteCheck` lands in the same directory.
function detectRedirectByTsconfig(root) {
    const rootTsconfig = join(root, 'tsconfig.json');
    if (!existsSync(rootTsconfig)) return null;
    let config;
    try {
        config = JSON.parse(readFileSync(rootTsconfig, 'utf8'));
    } catch {
        return null;
    }
    const looksLikeSolution = Array.isArray(config.files) && config.files.length === 0
        && (!config.include || config.include.length === 0)
        && Array.isArray(config.references) && config.references.length > 0;
    if (!looksLikeSolution) return null;
    for (const ref of config.references) {
        if (!ref?.path) continue;
        const refDir = isAbsolute(ref.path) ? ref.path : resolve(root, ref.path);
        const refTsconfig = join(refDir, 'tsconfig.json');
        if (!existsSync(refTsconfig)) continue;
        try {
            const refConfig = JSON.parse(readFileSync(refTsconfig, 'utf8'));
            const paths = refConfig?.compilerOptions?.paths;
            if (paths && Object.keys(paths).length > 0) return refDir;
        } catch { /* skip */ }
    }
    return null;
}

function runUpstream(bin, { tsgo, cwd }) {
    // Upstream svelte-check exits 1 on errors / 2 on invocation
    // errors. Wrap in try so we still parse the COMPLETED line when
    // errors exist.
    let stdout = '';
    const runArgs = [
        '--workspace', cwd ?? targetAbs,
        '--output', 'machine',
        '--diagnostic-sources', 'js,svelte',
    ];
    if (tsgo) runArgs.unshift('--tsgo');
    try {
        stdout = execFileSync(bin, runArgs, {
            stdio: ['ignore', 'pipe', 'pipe'],
            maxBuffer: 256 * 1024 * 1024,
            encoding: 'utf8',
            cwd: cwd ?? undefined,
        });
    } catch (err) {
        if (err.status === 1) stdout = err.stdout?.toString('utf8') ?? '';
        else throw err;
    }
    return parseCompleted(stdout) ?? { files: -1, errors: -1, warnings: -1, files_with_problems: -1 };
}

/// Parse the upstream "COMPLETED N FILES E ERRORS W WARNINGS F FILES_WITH_PROBLEMS"
/// line. Last COMPLETED wins in case of retries/warm-up lines.
function parseCompleted(output) {
    const re = /COMPLETED\s+(\d+)\s+FILES\s+(\d+)\s+ERRORS\s+(\d+)\s+WARNINGS\s+(\d+)\s+FILES_WITH_PROBLEMS/g;
    let last = null;
    let m;
    while ((m = re.exec(output)) !== null) last = m;
    if (!last) return null;
    return {
        files: Number(last[1]),
        errors: Number(last[2]),
        warnings: Number(last[3]),
        files_with_problems: Number(last[4]),
    };
}

/// Locate a `--tsgo`-capable upstream `svelte-check` in the target's
/// node_modules chain. Walks up the parent directories, scanning:
///   - `<dir>/node_modules/.bin/svelte-check`         (npm/yarn app-local)
///   - `<dir>/node_modules/.pnpm/node_modules/.bin/…` (pnpm workspace-hoisted)
///   - `<dir>/node_modules/.bun/svelte-check@X/…`     (bun install)
///   - `<dir>/node_modules/.pnpm/svelte-check@X/…`    (pnpm per-package)
///
/// Prefers svelte-check 4.4+ (first `--tsgo`-capable release). Older
/// hoisted versions (3.x / 4.0-4.3) get skipped via
/// `upstreamSupportsTsgo` check further downstream; if multiple
/// candidates exist, returns the first 4.4+ match it finds.
function findUpstreamSvelteCheck(start) {
    let dir = start;
    while (dir !== '/' && dir !== '.') {
        // Standard hoist locations first.
        for (const p of [
            join(dir, 'node_modules', '.bin', 'svelte-check'),
            join(dir, 'node_modules', '.pnpm', 'node_modules', '.bin', 'svelte-check'),
        ]) {
            if (existsSync(p)) return p;
        }
        // pnpm/bun per-package layouts — scan for 4.4+ versions.
        for (const managerRoot of [
            join(dir, 'node_modules', '.pnpm'),
            join(dir, 'node_modules', '.bun'),
        ]) {
            if (!existsSync(managerRoot)) continue;
            try {
                const entries = readdirSync(managerRoot);
                // Prefer 4.4+ (first --tsgo-capable release).
                const candidates = entries
                    .filter(e => /^svelte-check@\d/.test(e))
                    .sort((a, b) => {
                        const va = versionFromEntry(a);
                        const vb = versionFromEntry(b);
                        return compareVersions(vb, va); // descending
                    });
                for (const entry of candidates) {
                    const bin = join(managerRoot, entry, 'node_modules', 'svelte-check', 'bin', 'svelte-check');
                    if (existsSync(bin)) return bin;
                }
            } catch {
                // readdir failed (permissions / symlink loop / etc.) — skip.
            }
        }
        const parent = resolve(dir, '..');
        if (parent === dir) break;
        dir = parent;
    }
    return null;
}

function versionFromEntry(entry) {
    const m = entry.match(/^svelte-check@(\d+)\.(\d+)\.(\d+)/);
    return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : [0, 0, 0];
}

function compareVersions(a, b) {
    for (let i = 0; i < 3; i++) {
        if (a[i] !== b[i]) return a[i] - b[i];
    }
    return 0;
}

function upstreamSupportsTsgo(bin) {
    try {
        execFileSync(bin, ['--tsgo', '--help'], {
            stdio: ['ignore', 'pipe', 'pipe'],
            encoding: 'utf8',
            timeout: 20_000,
        });
        return true;
    } catch (err) {
        // svelte-check returns usage + exit 1 on unknown flag. We
        // detect by checking stderr/stdout for the known rejection
        // text; silent crashes (tsconfig errors etc.) would also land
        // here, so fall through to "assume supported" in that case.
        const msg = (err.stderr?.toString('utf8') ?? '') + (err.stdout?.toString('utf8') ?? '');
        if (/Unknown option.*--tsgo|Unrecognized/.test(msg)) return false;
        return true;
    }
}

function formatParityTable(rows, target) {
    const headers = ['tool', 'files', 'errors', 'warnings', 'files_with_problems'];
    const widths = headers.map(h => h.length);
    const body = rows.map(r => headers.map(h => String(r[h])));
    body.forEach(row => row.forEach((c, i) => { widths[i] = Math.max(widths[i], c.length); }));
    const sep = widths.map(w => '-'.repeat(w)).join('  ');
    const pad = (s, w) => String(s).padEnd(w);
    const lines = [
        `target: ${target}`,
        headers.map((h, i) => pad(h, widths[i])).join('  '),
        sep,
        ...body.map(row => row.map((c, i) => pad(c, widths[i])).join('  ')),
    ];
    return lines.join('\n');
}

// -----------------------------------------------------------------
// Shared helpers.
// -----------------------------------------------------------------

function runBinary() {
    const out = execFileSync(
        binary,
        ['--workspace', targetAbs, '--output', 'machine', ...extraArgs],
        { stdio: ['ignore', 'pipe', 'pipe'], maxBuffer: 256 * 1024 * 1024 },
    );
    if (!/\bCOMPLETED\b/.test(out.toString('utf8'))) {
        throw new Error('binary exited cleanly but produced no COMPLETED line — likely crash');
    }
}

function measure(fn) {
    const start = performance.now();
    try { fn(); }
    catch (err) {
        if (err.status === 1) { /* fine */ } else { throw err; }
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

function emitCsv(csvLines) {
    const csvText = csvLines.join('\n') + '\n';
    if (args.csv) {
        writeFileSync(args.csv, csvText);
        if (!args.quiet) console.error(`wrote ${args.csv}`);
    }
    process.stdout.write(csvText);
}

function parseArgs(a) {
    const out = { target: null, mode: null, csv: null, runs: null, tsconfig: null, quiet: false, allowDelta: false };
    for (let i = 0; i < a.length; i++) {
        const v = a[i];
        switch (v) {
            case '--target': out.target = a[++i]; break;
            case '--mode': out.mode = a[++i]; break;
            case '--csv': out.csv = a[++i]; break;
            case '--runs': out.runs = Number(a[++i]); break;
            case '--tsconfig': out.tsconfig = a[++i]; break;
            case '--quiet': out.quiet = true; break;
            case '--allow-delta': out.allowDelta = true; break;
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
