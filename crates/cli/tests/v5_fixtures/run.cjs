// Walk every `.v5` fixture under svelte2tsx's test corpus and run our
// binary against each. Pass criteria:
//
//   - Fixture NOT in baselines.json:
//         pass iff zero ERROR-severity diagnostics (clean fixture)
//   - Fixture IN baselines.json:
//         pass iff our error set EXACTLY matches baseline.expected_errors
//         (same count, same {code, line, column} per entry, optional
//         message_contains substring match). A count regression OR a
//         code/position shift both fail — the old `max_errors` cap would
//         silently tolerate an error-shape swap.
//
// The baseline list captures fixtures that are testing svelte2tsx's
// verbatim-emit behavior — they preserve user code character-for-character
// even when the user's code doesn't type-check. svelte2tsx's own
// `expectedv2.ts` for these fixtures contains TS errors. A "pass" for
// these fixtures means our overlay produces the SAME errors tsgo would
// produce on the user's code, in the same positions — catching any
// accidental emit regression that swaps one error for a different one.
//
// Migration mode: set `CAPTURE_BASELINES=1` to reconstruct the
// expected_errors lists from a live binary run. The runner writes
// updated `baselines.json` back to the same path in place, preserving
// each entry's `reason` field. Use this when an emit change deliberately
// shifts the expected errors, then review the diff.
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

const CAPTURE_BASELINES = process.env.CAPTURE_BASELINES === '1';

const baselinesRoot = (() => {
    try {
        return JSON.parse(fs.readFileSync(BASELINES, 'utf-8'));
    } catch (err) {
        throw new Error(`run.cjs: failed to read baselines from ${BASELINES}: ${err.message}`);
    }
})();
const baselines = baselinesRoot.verbatim_emit_fixtures || {};

// Stable sort for exact-shape comparison. file/line/column/code is the
// minimum key set — two errors with identical position + code would be
// a bug we'd want to see as a pair either way, so we don't break the tie
// with the message.
function sortErrors(a, b) {
    if (a.file !== b.file) return a.file < b.file ? -1 : 1;
    if (a.line !== b.line) return a.line - b.line;
    if (a.column !== b.column) return a.column - b.column;
    return a.code - b.code;
}

function errorShape(e) {
    return {
        file: (e.filename || '').replace(/\\/g, '/'),
        line: e.start.line,
        column: e.start.character,
        code: e.code
    };
}

// Compare actual errors (from machine-verbose JSON) against a baseline
// entry's `expected_errors` list. Returns null if they match, a failure
// reason string otherwise.
function compareExact(actual, expected) {
    const actualShapes = actual.map(errorShape).sort(sortErrors);
    const expectedSorted = [...expected].sort(sortErrors);
    if (actualShapes.length !== expectedSorted.length) {
        return `expected ${expectedSorted.length} errors, got ${actualShapes.length}`;
    }
    for (let i = 0; i < expectedSorted.length; i++) {
        const a = actualShapes[i];
        const e = expectedSorted[i];
        if (a.file !== e.file || a.line !== e.line || a.column !== e.column || a.code !== e.code) {
            return `error ${i} diverges — expected ${JSON.stringify(e)}, got ${JSON.stringify(a)}`;
        }
        if (e.message_contains) {
            const actualMsg = actual.find((x) => errorShape(x).file === a.file
                && x.start.line === a.line && x.start.character === a.column && x.code === a.code)?.message || '';
            if (!actualMsg.includes(e.message_contains)) {
                return `error ${i} message doesn't contain "${e.message_contains}" — got "${actualMsg}"`;
            }
        }
    }
    return null;
}

// The binary forces `--output machine` when it sees CLAUDECODE / GEMINI_CLI /
// CODEX_CI set to `"1"` in its environment (see crates/cli/src/main.rs). This
// runner parses `machine-verbose` JSON from stdout, so inheriting those vars
// from an agentic parent shell would make every diagnostic invisible and
// every fixture falsely "pass". Blank them out (not delete — the binary only
// rejects literal `"1"`, so `""` is the minimal override).
const CHILD_ENV = {
    ...process.env,
    CLAUDECODE: '',
    GEMINI_CLI: '',
    CODEX_CI: ''
};

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
        let crashed = false;
        let crashErr = null;
        try {
            stdout = execFileSync(
                BIN,
                ['--workspace', workspace, '--tsconfig', tsconfigPath, '--output', 'machine-verbose'],
                { encoding: 'utf-8', timeout: 60_000, env: CHILD_ENV }
            );
        } catch (err) {
            stdout = err.stdout || '';
            if (!stdout) {
                crashed = true;
                crashErr = err;
            }
        }

        if (crashed) {
            // No stdout means the binary crashed — surface it instead
            // of letting an empty errors[] falsely register as 'pass'.
            // `return` here triggers the surrounding try/finally
            // workspace cleanup before bubbling out of runFixture().
            failed++;
            failures.push({
                name,
                count: 0,
                first: null,
                crash: `signal=${crashErr?.signal} status=${crashErr?.status} msg=${crashErr?.message}`,
            });
            return;
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
        if (CAPTURE_BASELINES && baseline) {
            // Migration mode: record the actual errors as the new
            // `expected_errors`. Preserve the reason field; caller
            // will `git diff` to confirm the intended delta.
            baseline.expected_errors = errors.map(errorShape).sort(sortErrors);
            delete baseline.max_errors;
            passedBaseline++;
        } else if (baseline) {
            // Verbatim-emit fixture: pass iff our error set EXACTLY
            // matches baseline.expected_errors.
            const expected = baseline.expected_errors || [];
            const mismatch = compareExact(errors, expected);
            if (mismatch === null) {
                passedBaseline++;
            } else {
                failed++;
                failures.push({
                    name,
                    count: errors.length,
                    first: errors[0],
                    baselineMismatch: mismatch
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

// We accept fixtures from two layouts:
//   - upstream svelte2tsx samples — many non-v5 dirs share the SAMPLES_DIR;
//     we filter to .v5 only to skip Svelte 4 fixtures
//   - our own v5-stores layout — every dir is a fixture, except `_shared/`
//     and other underscore-prefixed metadata dirs
//
// Heuristic: if any .v5 dir exists, behave as upstream-mode (filter to .v5).
// Otherwise local-mode (every dir except _-prefixed ones).
const entries = fs.readdirSync(SAMPLES_DIR).sort();
const hasV5 = entries.some((e) => e.endsWith('.v5'));
const isFixtureDir = (entry) => {
    if (entry.startsWith('_')) return false;
    if (hasV5 && !entry.endsWith('.v5')) return false;
    return true;
};
for (const entry of entries) {
    if (!isFixtureDir(entry)) continue;
    const dir = path.join(SAMPLES_DIR, entry);
    if (!fs.statSync(dir).isDirectory()) continue;
    runFixture(entry, dir);
}

if (CAPTURE_BASELINES) {
    // Write updated baselines back — preserves the _doc + reason fields
    // via the in-place edit we did during runFixture's capture branch.
    fs.writeFileSync(BASELINES, JSON.stringify(baselinesRoot, null, 4) + '\n');
    console.log(`CAPTURE_BASELINES: wrote ${Object.keys(baselines).length} entries to ${BASELINES}`);
    process.exit(0);
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
        const detail = f.baselineMismatch ? ` [${f.baselineMismatch}]` : '';
        const errDetail = e
            ? `TS${e.code} ${e.message} @ ${e.filename}:${e.start.line + 1}:${e.start.character + 1}`
            : '<no errors>';
        console.log(`  FAIL ${f.name} (${f.count} errors)${detail}: ${errDetail}`);
    }
}
process.exit(failed > 0 ? 1 : 0);
