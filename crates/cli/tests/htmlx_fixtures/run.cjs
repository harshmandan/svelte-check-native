// Walk every sample under `htmlx2jsx/samples/` (modulo the skip list)
// and run our binary against each. Same harness shape as run.cjs in
// v5_fixtures — Node spawns the Rust binary with a per-fixture
// temporary workspace, parses the machine-verbose diagnostics, and
// compares the ERROR count to the sample's baseline.
//
// Why a baseline (not zero-errors)?
// Upstream's htmlx2jsx samples are STRING-EMIT tests: they compare a
// transformed .svelte against a checked-in `expectedv2.js`. Most
// samples reference undeclared identifiers (no surrounding <script>
// that declares `items`, `foo`, `Component`, etc.) because upstream
// isn't type-checking — they're asserting on the emit shape. For us
// those same samples DO go through tsgo, which reports TS2304
// `Cannot find name` on every undeclared identifier. Per-sample
// baselines let us catch regressions while ignoring that noise floor.
//
// Env:
//   SVELTE_CHECK_BIN — absolute path to our binary
//   SAMPLES_DIR      — absolute path to the htmlx2jsx/samples dir
//   SHIM_TSCONFIG    — base tsconfig extended by per-fixture tsconfigs
//   BASELINES        — baselines.json path (read-only here)
//   SKIP_LIST        — skip.json listing Svelte 4 samples to ignore

'use strict';

const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const BIN = process.env.SVELTE_CHECK_BIN;
if (!BIN) throw new Error('run.cjs: SVELTE_CHECK_BIN required');
const SAMPLES_DIR = process.env.SAMPLES_DIR;
if (!SAMPLES_DIR) throw new Error('run.cjs: SAMPLES_DIR required');
const SHIM_TSCONFIG = process.env.SHIM_TSCONFIG;
if (!SHIM_TSCONFIG) throw new Error('run.cjs: SHIM_TSCONFIG required');
const BASELINES = process.env.BASELINES;
if (!BASELINES) throw new Error('run.cjs: BASELINES required');
const SKIP_LIST = process.env.SKIP_LIST;
if (!SKIP_LIST) throw new Error('run.cjs: SKIP_LIST required');

const baselineMap = (() => {
    try {
        const raw = JSON.parse(fs.readFileSync(BASELINES, 'utf-8'));
        return raw.samples || {};
    } catch (err) {
        throw new Error(`run.cjs: failed to read baselines: ${err.message}`);
    }
})();
const skipSet = (() => {
    try {
        const raw = JSON.parse(fs.readFileSync(SKIP_LIST, 'utf-8'));
        return new Set(raw.skip || []);
    } catch (err) {
        throw new Error(`run.cjs: failed to read skip list: ${err.message}`);
    }
})();

let clean = 0;
let withinBaseline = 0;
let failed = 0;
let skipped = 0;
const observedCounts = {};
const failures = [];

function runFixture(name, fixtureDir) {
    const input = path.join(fixtureDir, 'input.svelte');
    if (!fs.existsSync(input)) {
        skipped++;
        return;
    }

    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), `svn-htmlx-${name}-`));
    try {
        const srcDir = path.join(workspace, 'src');
        fs.mkdirSync(srcDir, { recursive: true });
        const inputCopy = path.join(srcDir, 'input.svelte');
        fs.copyFileSync(input, inputCopy);

        const tsconfigPath = path.join(workspace, 'tsconfig.json');
        fs.writeFileSync(
            tsconfigPath,
            JSON.stringify({ extends: SHIM_TSCONFIG, include: ['src/**/*'] }, null, 2)
        );

        let stdout = '';
        try {
            stdout = execFileSync(
                BIN,
                ['--workspace', workspace, '--tsconfig', tsconfigPath, '--output', 'machine-verbose'],
                { encoding: 'utf-8', timeout: 60_000 }
            );
        } catch (err) {
            stdout = err.stdout || '';
        }

        const errors = [];
        for (const line of stdout.split('\n')) {
            const jsonStart = line.indexOf('{');
            if (jsonStart === -1) continue;
            try {
                const entry = JSON.parse(line.slice(jsonStart));
                if (entry.type === 'ERROR') errors.push(entry);
            } catch {
                /* not JSON */
            }
        }

        observedCounts[name] = errors.length;
        const baseline = baselineMap[name];
        const allowed = baseline !== undefined ? baseline.max_errors : 0;
        if (errors.length === 0) {
            clean++;
        } else if (errors.length <= allowed) {
            withinBaseline++;
        } else {
            failed++;
            failures.push({ name, count: errors.length, allowed, first: errors[0] });
        }
    } finally {
        fs.rmSync(workspace, { recursive: true, force: true });
    }
}

const entries = fs.readdirSync(SAMPLES_DIR).sort();
for (const entry of entries) {
    if (entry.startsWith('_')) continue;
    if (skipSet.has(entry)) {
        skipped++;
        continue;
    }
    const dir = path.join(SAMPLES_DIR, entry);
    let stat;
    try {
        stat = fs.statSync(dir);
    } catch {
        continue;
    }
    if (!stat.isDirectory()) continue;
    runFixture(entry, dir);
}

const total = clean + withinBaseline + failed;
console.log(
    `htmlx fixtures: ${clean + withinBaseline}/${total} (${clean} clean, ${withinBaseline} within-baseline), ${failed} failed${skipped ? `, ${skipped} skipped` : ''}`
);
if (failures.length > 0) {
    console.log('\nFirst error per failing fixture (showing up to 40):');
    for (const f of failures.slice(0, 40)) {
        const e = f.first;
        const loc = e && e.filename ? `${e.filename}:${(e.start?.line ?? 0) + 1}:${(e.start?.character ?? 0) + 1}` : '?';
        console.log(
            `  FAIL ${f.name} (${f.count} errors, baseline ${f.allowed}): TS${e?.code ?? '?'} ${e?.message ?? '?'} @ ${loc}`
        );
    }
}

// Always emit a JSON snapshot of observed counts so baseline updates
// are one-step: copy observed_counts.json → baselines.json.samples.
const snapshotPath = path.join(path.dirname(BASELINES), 'observed_counts.json');
fs.writeFileSync(
    snapshotPath,
    JSON.stringify(
        {
            _doc: 'Auto-generated by run.cjs on every invocation. NOT a baseline — copy entries into baselines.json.samples to accept them.',
            observed: Object.fromEntries(
                Object.entries(observedCounts).sort(([a], [b]) => a.localeCompare(b))
            )
        },
        null,
        2
    )
);

process.exit(failed > 0 ? 1 : 0);
