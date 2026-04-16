// Walk every `.v5` fixture under svelte2tsx's test corpus and run our
// binary against each. Pass criteria:
//
//   - Fixture NOT in baselines.json:
//         pass iff zero ERROR-severity diagnostics (clean fixture)
//   - Fixture IN baselines.json:
//         pass iff our error count ≤ baseline.max_errors
//
// The baseline list captures fixtures that are testing svelte2tsx's
// verbatim-emit behavior — they preserve user code character-for-character
// even when the user's code doesn't type-check. svelte2tsx's own
// `expectedv2.ts` for these fixtures contains TS errors. A "pass" for
// these fixtures means we're not introducing extra errors beyond what
// the user's code produces.
//
// Each fixture is a self-contained directory:
//   <fixture>.v5/
//     input.svelte       — Svelte 5 source (the SUT)
//     expectedv2.ts      — what svelte2tsx emits (reference; not consumed)
//
// We invoke our binary by writing a temporary per-fixture workspace
// containing the input.svelte under `src/` plus a minimal tsconfig.
//
// Env:
//   SVELTE_CHECK_BIN — absolute path to the svelte-check-native binary
//   SAMPLES_DIR      — absolute path to the .v5 samples directory
//   SHIM_TSCONFIG    — absolute path to the minimal tsconfig template
//   BASELINES        — absolute path to baselines.json

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
const BASELINES = process.env.BASELINES;
if (!BASELINES) {
    throw new Error('run.cjs: BASELINES env var required');
}

const baselines = (() => {
    try {
        const raw = JSON.parse(fs.readFileSync(BASELINES, 'utf-8'));
        return raw.verbatim_emit_fixtures || {};
    } catch (err) {
        throw new Error(`run.cjs: failed to read baselines from ${BASELINES}: ${err.message}`);
    }
})();

let passed = 0;
let passedBaseline = 0; // passed because within baseline budget
let failed = 0;
let skipped = 0;
const failures = [];

function runFixture(name, fixtureDir) {
    // Most fixtures use input.svelte; SvelteKit-targeted ones use
    // +page.svelte (or +layout.svelte) because the file name itself
    // signals the route role to svelte2tsx's auto-typing.
    const candidates = ['input.svelte', '+page.svelte', '+layout.svelte', '+page.ts'];
    const input = candidates
        .map((c) => path.join(fixtureDir, c))
        .find((p) => fs.existsSync(p));
    if (!input) {
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

        const baseline = baselines[name];
        if (baseline) {
            // Verbatim-emit fixture: pass iff our error count ≤ baseline.
            if (errors.length <= baseline.max_errors) {
                passedBaseline++;
            } else {
                failed++;
                failures.push({
                    name,
                    count: errors.length,
                    first: errors[0],
                    baseline: baseline.max_errors
                });
            }
        } else if (errors.length === 0) {
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

const total = passed + passedBaseline + failed;
const passLine = passedBaseline > 0
    ? `v5 fixtures: ${passed + passedBaseline}/${total} (${passed} clean, ${passedBaseline} within-baseline), ${failed} failed${skipped ? `, ${skipped} skipped` : ''}`
    : `v5 fixtures: ${passed} passed, ${failed} failed${skipped ? `, ${skipped} skipped` : ''}`;
console.log(passLine);
if (failures.length > 0) {
    console.log('\nFirst error per failing fixture (showing up to 30):');
    for (const f of failures.slice(0, 30)) {
        const e = f.first;
        const baselineNote = f.baseline !== undefined ? ` [over baseline ${f.baseline}]` : '';
        console.log(
            `  FAIL ${f.name} (${f.count} errors${baselineNote}): TS${e.code} ${e.message} @ ${e.filename}:${e.start.line + 1}:${e.start.character + 1}`
        );
    }
}
process.exit(failed > 0 ? 1 : 0);
