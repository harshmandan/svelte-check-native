// Bug-fixture runner. Iterates fixtures/bugs/<NN>-<slug>/ and asserts that
// `svelte-check-native <fixture>` produces exactly the diagnostics listed in
// the fixture's expected.json.
//
// Same assertion shape as upstream's test-sanity.js — same parse rules for
// machine-verbose output — so the habit of reading either is portable.
//
// Env:
//   SVELTE_CHECK_BIN — absolute path to the binary
//   FIXTURES_DIR     — absolute path to fixtures/bugs/
//
// expected.json shapes:
//   { "clean": true }
//       → run binary normally, assert zero ERRORs emitted (black-box)
//   { "errors": [{file,line,column,code}] }
//       → run binary normally, assert exact set of ERRORs (black-box)
//   { "emit_contains": ["..."], "emit_not_contains": ["..."] }
//       → run binary with `--emit-ts`; capture stdout as generated TS;
//         assert substring presence/absence on the emitted code

'use strict';

const { execFileSync } = require('child_process');
const { readdirSync, readFileSync, statSync, rmSync, existsSync } = require('fs');
const path = require('path');

const BIN = process.env.SVELTE_CHECK_BIN;
if (!BIN) {
    throw new Error('run.cjs: SVELTE_CHECK_BIN env var required');
}
const FIXTURES_DIR = process.env.FIXTURES_DIR;
if (!FIXTURES_DIR) {
    throw new Error('run.cjs: FIXTURES_DIR env var required');
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
let failed = 0;
let skipped = 0;

function sortKey(a, b) {
    if (a.file !== b.file) return a.file.localeCompare(b.file);
    if (a.line !== b.line) return a.line - b.line;
    if (a.column !== b.column) return a.column - b.column;
    return a.code - b.code;
}

function runFixture(name, fixtureDir) {
    const expectedPath = path.join(fixtureDir, 'expected.json');
    if (!existsSync(expectedPath)) {
        skipped++;
        console.log(`  SKIP: ${name} (no expected.json)`);
        return;
    }

    let expected;
    try {
        expected = JSON.parse(readFileSync(expectedPath, 'utf-8'));
    } catch (err) {
        failed++;
        console.log(`  FAIL: ${name} (expected.json parse error: ${err.message})`);
        return;
    }

    // Wipe any leftover cache between runs for determinism.
    rmSync(path.join(fixtureDir, '.svelte-check'), { recursive: true, force: true });
    rmSync(path.join(fixtureDir, '.svelte-kit'), { recursive: true, force: true });

    const tsconfig = path.join(fixtureDir, 'tsconfig.json');
    const tsconfigExists = existsSync(tsconfig);
    const isEmitMode = Array.isArray(expected.emit_contains) || Array.isArray(expected.emit_not_contains);

    const issues = [];

    if (isEmitMode) {
        // Grey-box: run with --emit-ts, capture stdout as generated TS.
        // --emit-ts exits before tsconfig resolution, so the flag is
        // a no-op when present and harmless when absent.
        const args = ['--emit-ts', '--workspace', fixtureDir];
        if (tsconfigExists) {
            args.push('--tsconfig', tsconfig);
        }

        let emit = '';
        try {
            emit = execFileSync(BIN, args, { encoding: 'utf-8', timeout: 60_000, env: CHILD_ENV });
        } catch (err) {
            emit = err.stdout || '';
        }

        for (const needle of expected.emit_contains || []) {
            if (!emit.includes(needle)) {
                issues.push(`expected emit to contain ${JSON.stringify(needle)}`);
            }
        }
        for (const needle of expected.emit_not_contains || []) {
            if (emit.includes(needle)) {
                issues.push(`expected emit to NOT contain ${JSON.stringify(needle)}`);
            }
        }

        if (issues.length && process.env.DEBUG_EMIT) {
            issues.push(`captured emit (first 4000 chars):\n${emit.slice(0, 4000)}`);
        }
    } else {
        // Black-box: run normally, parse machine-verbose diagnostics.
        // Black-box mode requires a tsconfig; fixtures lacking one are
        // expected to be `--emit-ts`-only and should also lack
        // expected.json (which would route them through the SKIP branch
        // at the top of this function).
        const args = ['--workspace', fixtureDir, '--output', 'machine-verbose'];
        if (tsconfigExists) {
            args.push('--tsconfig', tsconfig);
        }

        let stdout = '';
        try {
            stdout = execFileSync(BIN, args, { encoding: 'utf-8', timeout: 60_000, env: CHILD_ENV });
        } catch (err) {
            stdout = err.stdout || '';
        }

        const actualErrors = [];
        for (const line of stdout.split('\n')) {
            const jsonStart = line.indexOf('{');
            if (jsonStart === -1) continue;
            let entry;
            try {
                entry = JSON.parse(line.slice(jsonStart));
            } catch {
                continue;
            }
            if (entry.type === 'ERROR') {
                actualErrors.push({
                    file: String(entry.filename || '').replace(/\\/g, '/'),
                    line: entry.start?.line ?? -1,
                    column: entry.start?.character ?? -1,
                    code: entry.code
                });
            }
        }

        if (expected.clean === true) {
            if (actualErrors.length !== 0) {
                issues.push(
                    `expected clean, got ${actualErrors.length} error(s):\n` +
                        JSON.stringify(actualErrors, null, 2)
                );
            }
        } else {
            const exp = [...(expected.errors || [])].sort(sortKey);
            const act = [...actualErrors].sort(sortKey);
            if (JSON.stringify(exp) !== JSON.stringify(act)) {
                issues.push(`expected:\n${JSON.stringify(exp, null, 2)}\nactual:\n${JSON.stringify(act, null, 2)}`);
            }
        }
    }

    if (issues.length) {
        failed++;
        console.log(`  FAIL: ${name}`);
        for (const line of issues) {
            for (const l of line.split('\n')) console.log(`        ${l}`);
        }
    } else {
        passed++;
        console.log(`  PASS: ${name}`);
    }
}

console.log('svelte-check-native bug fixtures\n');

const entries = readdirSync(FIXTURES_DIR).sort();
for (const entry of entries) {
    if (entry.startsWith('_')) continue; // skip _shared/, _templates/
    const dir = path.join(FIXTURES_DIR, entry);
    try {
        if (!statSync(dir).isDirectory()) continue;
    } catch {
        continue;
    }
    runFixture(entry, dir);
}

console.log(`\n${passed} passed, ${failed} failed${skipped ? `, ${skipped} skipped` : ''}`);
process.exit(failed > 0 ? 1 : 0);
