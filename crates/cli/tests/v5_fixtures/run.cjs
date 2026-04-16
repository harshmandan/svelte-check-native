// Walk every `.v5` fixture under svelte2tsx's test corpus and assert
// our binary type-checks each input.svelte cleanly (zero errors).
//
// Each fixture is a self-contained directory:
//   <fixture>.v5/
//     input.svelte       — Svelte 5 source (the system under test)
//     expectedv2.ts      — what svelte2tsx emits (reference; not consumed)
//
// We invoke our binary in single-file mode by writing a temporary
// per-fixture workspace that contains:
//   - the input.svelte under a `src/` directory
//   - a minimal tsconfig.json
// and running the full type-check pipeline against it.
//
// The fixture passes when the binary reports zero ERROR-severity
// diagnostics. The bar is intentionally lenient: a fixture is a
// known-good Svelte 5 component, so any error we report against it is
// a fidelity gap.
//
// Env:
//   SVELTE_CHECK_BIN — absolute path to the svelte-check-native binary
//   SAMPLES_DIR      — absolute path to the .v5 samples directory
//   SHIM_TSCONFIG    — absolute path to a minimal tsconfig.json template
//                      that fixtures inherit via `extends`. Keeps each
//                      fixture's tsconfig tiny and consistent.

'use strict';

const { execFileSync } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const BIN = process.env.SVELTE_CHECK_BIN;
if (!BIN) {
    throw new Error('run.cjs: SVELTE_CHECK_BIN env var required');
}
const SAMPLES_DIR = process.env.SAMPLES_DIR;
if (!SAMPLES_DIR) {
    throw new Error('run.cjs: SAMPLES_DIR env var required');
}
const SHIM_TSCONFIG = process.env.SHIM_TSCONFIG;
if (!SHIM_TSCONFIG) {
    throw new Error('run.cjs: SHIM_TSCONFIG env var required');
}

let passed = 0;
let failed = 0;
let skipped = 0;
const failures = [];

function runFixture(name, fixtureDir) {
    const input = path.join(fixtureDir, 'input.svelte');
    if (!fs.existsSync(input)) {
        skipped++;
        return;
    }

    // Create a per-fixture workspace under a fresh temp directory so the
    // .svelte-check cache doesn't pollute the shared samples tree.
    const workspace = fs.mkdtempSync(path.join(os.tmpdir(), `svn-v5-${name}-`));
    try {
        const srcDir = path.join(workspace, 'src');
        fs.mkdirSync(srcDir, { recursive: true });
        const inputCopy = path.join(srcDir, path.basename(input));
        fs.copyFileSync(input, inputCopy);

        const tsconfigPath = path.join(workspace, 'tsconfig.json');
        const tsconfig = {
            extends: SHIM_TSCONFIG,
            include: ['src/**/*']
        };
        fs.writeFileSync(tsconfigPath, JSON.stringify(tsconfig, null, 2));

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

        if (errors.length === 0) {
            passed++;
        } else {
            failed++;
            failures.push({ name, count: errors.length, first: errors[0] });
        }
    } finally {
        fs.rmSync(workspace, { recursive: true, force: true });
    }
}

const entries = fs.readdirSync(SAMPLES_DIR).sort();
for (const entry of entries) {
    if (!entry.endsWith('.v5')) continue;
    const dir = path.join(SAMPLES_DIR, entry);
    if (!fs.statSync(dir).isDirectory()) continue;
    runFixture(entry, dir);
}

console.log(`v5 fixtures: ${passed} passed, ${failed} failed${skipped ? `, ${skipped} skipped` : ''}`);
if (failures.length > 0) {
    console.log('\nFirst error per failing fixture (showing up to 10):');
    for (const f of failures.slice(0, 10)) {
        const e = f.first;
        console.log(
            `  FAIL ${f.name} (${f.count} errors): TS${e.code} ${e.message} @ ${e.filename}:${e.start.line + 1}:${e.start.character + 1}`
        );
    }
}
process.exit(failed > 0 ? 1 : 0);
