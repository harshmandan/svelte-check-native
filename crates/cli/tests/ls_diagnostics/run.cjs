// LS diagnostic-fixture runner. Iterates upstream's
// language-tools/.../diagnostics/fixtures/<name>/ and asserts that our
// binary's diagnostics match `expected_svelte_5.json` (preferred) or
// `expectedv2.json` on (file, line, character, code).
//
// Lossy compare per notes/PARITY_TESTING_PLAN.md P1 — message + range.end
// not asserted. Upgrades to strict are gated by STRICT_LS_DIAGNOSTICS=1.
//
// Env:
//   SVELTE_CHECK_BIN          absolute path to the binary
//   FIXTURES_DIR              upstream fixtures dir (read-only)
//   STRICT_LS_DIAGNOSTICS=1   also assert message text + range.end (not yet)
//
// Each fixture is run with --workspace <fixture> and --tsconfig <fixture>/
// tsconfig.json (falling back to <fixtures-root>/tsconfig.json). Skip-list
// is the explicit "future parity bug" list — every entry should map to an
// open ROADMAP item.

'use strict';

const { execFileSync } = require('child_process');
const { readdirSync, readFileSync, statSync, rmSync, existsSync, copyFileSync } = require('fs');
const path = require('path');

const BIN = process.env.SVELTE_CHECK_BIN;
if (!BIN) throw new Error('run.cjs: SVELTE_CHECK_BIN env var required');
const FIXTURES_DIR = process.env.FIXTURES_DIR;
if (!FIXTURES_DIR) throw new Error('run.cjs: FIXTURES_DIR env var required');

const SHARED_TSCONFIG = path.join(FIXTURES_DIR, 'tsconfig.json');

// Defang the agentic env-flag so the binary doesn't auto-coerce to the
// human output format and starve our parser. Same defense as
// bug_fixtures/run.cjs.
const CHILD_ENV = { ...process.env, CLAUDECODE: '', GEMINI_CLI: '', CODEX_CI: '' };

// Fixtures the upstream root tsconfig explicitly excludes — these run with
// project-specific harness setup we don't replicate. Round-Parity #2:
// `svelte-native` was previously here, but its expectations file is empty
// (`[]`) and our binary produces no `input.svelte` diagnostics for it, so
// it passes strict matching trivially. Removed — kept node16 and
// project-reference because their special module-resolution / project-
// references harnesses aren't yet replicated.
const UPSTREAM_EXCLUDED = new Set([
    'node16',
    'project-reference',
]);

// Fixtures we deliberately skip with a one-line reason. Treat each as a
// future parity bug; the count should monotonically decrease as overlay-
// shape and lint coverage close gaps. Buckets:
//
//   bucket=overlay-positions: identical (file, code) MULTISET as upstream
//     in spirit, but counts diverge because our overlay produces a
//     different number of TS errors per source position. E.g. a single
//     bind: site fans out to 2 errors in upstream's overlay vs 1 in ours
//     (or vice versa) — both flag the same bug, just with different
//     redundancy. Resolves when overlay shape converges.
//
//   bucket=missing-code: TS code our typecheck doesn't surface in this
//     scenario. Examples: 6385/6387 (deprecation hints, severity≠ERROR),
//     6133/7044 (unused-decl / implicit-any noisy hints we filter),
//     18047 (possibly-null narrowing on a path tsgo handles differently),
//     -1 (svelte-compiler parser error reported by upstream's
//     svelte plugin, not by tsgo). Resolves when those code paths land.
//
//   bucket=alt-language: <script lang="coffee"> / lang="pug" — upstream's
//     LS skips type-checking these; we feed them to oxc and explode.
//
//   bucket=svelte-shim-resolution: TS2307 "cannot find module 'svelte'"
//     fires in our overlay because the LS fixtures dir doesn't have a
//     `svelte` package installed; upstream's LS resolves the module via
//     the in-process svelte-language-server importPackage path. Whole-
//     workspace mode would need a synthetic install.
//
//   bucket=position-drift: same (file, code) MULTISET but column /
//     line drifts because our overlay's reverse mapping lands the
//     diagnostic on a different anchor than upstream's. Surfaced
//     when R-Parity #1 switched the default match from (file, code)
//     to (file, line, character, code). Resolves when the overlay's
//     line/col anchor converges with upstream's.
const SKIP_LIST = {
    // Upstream-excluded — see UPSTREAM_EXCLUDED above; recorded here too
    // so the printed scoreboard surfaces them.
    'node16': 'upstream-root tsconfig excludes — node16 module-resolution mode',
    'project-reference': 'upstream-root tsconfig excludes — project-references mode',

    // bucket=alt-language
    'pug': 'alt-language: <script lang="pug"> support gap — TS2339 leak',

    // bucket=missing-code
    '$$events': 'missing-code: 6385/6387 deprecation hints not surfaced',
    'deprecated-unused-hints': 'missing-code: 6133/6385 unused/deprecated hints filtered',
    '$bindable-reassign.v5': 'missing-code: 6133 unused-declaration hint filtered',
    'const-tag': 'missing-code: 6133 unused-declaration hint filtered (5×)',
    'parser-error': 'missing-code: -1 svelte-compiler parser error path differs',
    'svelte-element-error': 'missing-code: -1 svelte-compiler parser error path differs',
    'unInitialized': 'missing-code: 2454 used-before-assignment narrowing differs (3×)',
    'bind-this': 'missing-code: 2322/2454/6133 mix not all surfaced',
    'undeclared-component': 'missing-code: 2304 cannot-find-name on auto-imported components',
    'ignore-generated-code': 'missing-code: 2304 in injected blocks not surfaced',
    'if-control-flow': 'missing-code: 18047 narrowing diff vs upstream',
    'typechecks-js-with-ts-check': 'missing-code: 2339 in @ts-check-only file not surfaced',

    // bucket=svelte-shim-resolution / structural
    '$$events-usage': 'shim-resolution: 2307 cannot-find-module svelte + 7006 implicit-any cascade',
    '$$props-usage': 'shim-resolution: 2307 cannot-find-module svelte',
    '$$slots-usage': 'shim-resolution: 2307 cannot-find-module svelte + 2339/2367 mismatch',
    'custom-types': '7006 implicit-any leak (×4) plus 2353/7044 missing — overlay differs',

    // bucket=overlay-positions: same code categories, different counts
    '$$props-invalid3': 'tsgo-divergence: tsgo emits inner TS2741 (missing-prop) where tsc/upstream LS emits TS2345 wrap',
    '$$slots': 'overlay-counts: 2322/2345/2353 multiset diverges',
    '$store-wrong-usage': 'overlay-counts: 2769 fires 6× upstream, 0× ours',
    'accessors-customElement-configs': 'overlay-counts: 2322 extra in our overlay',
    'actions-animations-transitions-typechecks': 'overlay-counts: 2345 expected 2×, ours 0×',
    'bindings': 'overlay-counts: 2322/2339/2353 multiset diverges',
    'component-invalid': 'overlay-counts: 2322/2345 missing, 2353/2554 extra',
    // svelte-native expects the svelteNative.JSX namespace switch (jsxFactory:
    // "svelteNative" + svelteOptions.namespace = "svelteNative.JSX") so element
    // attribute checks degrade to the catch-all `[name: string]: { [name: string]: any }`
    // shape. We always emit `svelteHTML.createElement(...)`; once R-Conv #7
    // tightened the fallback `HTMLAttributes` interface, the per-element strict
    // shape now fires 2353 on `<label horizontalAlignment=…>` etc. which the
    // svelte-native fixture intends to permit.
    'svelte-native': 'namespace-handling: requires svelteOptions.namespace=svelteNative.JSX',
    'generics': 'overlay-counts: 2322/2367 multiset diverges',
    'getters': 'overlay-counts: 2367 vs 2749 mismatch',
    'import-precedence': 'overlay-counts: extra 2614 (×2)',
    'snippet-js.v5': 'overlay-counts: 2345/2367 missing',
    'strictEvents': 'overlay-counts: 2345 missing',
    'svelte-element': 'overlay-counts: 2353 missing, 2322/2741/7006 extra',

    // bucket=position-drift was here. All three entries
    // (`$store-bind`, `script-boolean-not-assignable-to-string`,
    // `modulescript-boolean-not-assignable-to-string`) unblocked
    // by R-Conv #1 (component-bind name anchor) and R-Conv #2
    // (single-line script body TokenMapEntry).
};

let passed = 0;
let failed = 0;
let skipped = 0;
const skipReasons = [];
const failureReasons = [];

function diagnosticKey(d) {
    return `${d.file}:${d.line}:${d.character} [${d.code}]`;
}

function diagnosticCodeKey(d) {
    return `${d.file} [${d.code}]`;
}

// Round-Parity #1: default match is now `(file, line, character,
// code)` — STRICT position parity, matching the plan in
// notes/PARITY_TESTING_PLAN.md. Pre-fix the suite compared
// `(file, code)` only; the printed PASS line meant "we fired the
// right categories of error in the right files" but said nothing
// about line/character drift. A position regression could land
// silently as long as the (file, code) multiset stayed unchanged.
//
// Fixtures whose positions don't yet match upstream's reverse-
// mapped output are added to SKIP_LIST with a `bucket=position-drift`
// reason. Those entries are now visible work, not invisible
// passes.
//
// LOOSE mode (LS_DIAGNOSTICS_LOOSE=1) keeps the legacy `(file,
// code)` comparison for cases where the local environment can't
// reach byte-perfect positions yet (CI cold-cache scenarios, etc.).
const LOOSE = process.env.LS_DIAGNOSTICS_LOOSE === '1';
const compareKey = LOOSE ? diagnosticCodeKey : diagnosticKey;
function makeMultiset(list, keyFn) {
    const map = new Map();
    for (const d of list) {
        const k = keyFn(d);
        map.set(k, (map.get(k) ?? 0) + 1);
    }
    return map;
}

function diffMultisets(expected, actual) {
    const missing = [];
    const extra = [];
    const keys = new Set([...expected.keys(), ...actual.keys()]);
    for (const k of keys) {
        const exp = expected.get(k) ?? 0;
        const act = actual.get(k) ?? 0;
        if (act < exp) missing.push(`${k} (×${exp - act})`);
        else if (act > exp) extra.push(`${k} (×${act - exp})`);
    }
    return { missing: missing.sort(), extra: extra.sort() };
}

function loadExpected(fixtureDir) {
    const v5 = path.join(fixtureDir, 'expected_svelte_5.json');
    const v2 = path.join(fixtureDir, 'expectedv2.json');
    const file = existsSync(v5) ? v5 : (existsSync(v2) ? v2 : null);
    if (!file) return null;
    let raw;
    try {
        raw = JSON.parse(readFileSync(file, 'utf-8'));
    } catch (err) {
        return { error: `parse error in ${file}: ${err.message}` };
    }
    // Normalize each entry to the shape we'll compare on. Filenames in
    // upstream's expected files are unqualified (the LS opens one document
    // by URI, so the diagnostic comes back keyed only by that document).
    // Our binary emits paths relative to --workspace, which IS the fixture
    // dir, so basenames line up.
    const list = raw.map(d => ({
        file: d.filename ? path.basename(d.filename) : 'input.svelte',
        line: d.range?.start?.line ?? -1,
        character: d.range?.start?.character ?? -1,
        code: d.code,
    }));
    return { list };
}

function runBinary(fixtureDir) {
    rmSync(path.join(fixtureDir, '.svelte-check'), { recursive: true, force: true });
    rmSync(path.join(fixtureDir, '.svelte-kit'), { recursive: true, force: true });

    const fixtureTsconfig = path.join(fixtureDir, 'tsconfig.json');
    const tsconfig = existsSync(fixtureTsconfig) ? fixtureTsconfig : SHARED_TSCONFIG;

    const args = [
        '--workspace', fixtureDir,
        '--tsconfig', tsconfig,
        '--output', 'machine-verbose',
    ];

    let stdout = '';
    let crashed = null;
    try {
        stdout = execFileSync(BIN, args, {
            encoding: 'utf-8',
            timeout: 60_000,
            env: CHILD_ENV,
            maxBuffer: 64 * 1024 * 1024,
        });
    } catch (err) {
        stdout = err.stdout || '';
        if (!stdout) crashed = err;
    }
    if (crashed) {
        return {
            crash: `signal=${crashed.signal} status=${crashed.status} msg=${crashed.message}`,
            diagnostics: [],
        };
    }
    const diagnostics = [];
    for (const line of stdout.split('\n')) {
        const idx = line.indexOf('{');
        if (idx < 0) continue;
        let entry;
        try {
            entry = JSON.parse(line.slice(idx));
        } catch {
            continue;
        }
        if (entry.type !== 'ERROR' && entry.type !== 'WARNING') continue;
        const fname = String(entry.filename || '').replace(/\\/g, '/');
        const base = path.basename(fname);
        // Upstream's LS test asserts diagnostics for ONE opened document
        // (`input.svelte`, see test-utils.ts createSnapshotTester) — sibling
        // .svelte files only enter as imports, never as the diagnostic
        // target. Generated shim files / cache dirs / node_modules entries
        // also don't enter the expected JSON. Our binary reports workspace-
        // wide, so we trim to input.svelte only for an apples-to-apples
        // compare.
        if (base !== 'input.svelte') continue;
        if (
            fname.includes('.svelte-check/')
            || fname.includes('.svelte-kit/')
            || fname.includes('node_modules/')
        ) continue;
        // svn-lint warnings have string codes (`element_invalid_self_closing_tag`
        // etc.); upstream's LS test doesn't run our linter, so drop them
        // from the comparison. Numeric codes are the TS diagnostic codes
        // upstream's expectedv2.json keys on.
        if (typeof entry.code !== 'number') continue;
        diagnostics.push({
            file: base,
            line: entry.start?.line ?? -1,
            character: entry.start?.character ?? -1,
            code: entry.code,
        });
    }
    return { diagnostics };
}

// Round-Parity #2: collect "stale skip" entries — fixtures listed
// in SKIP_LIST or UPSTREAM_EXCLUDED that actually pass strict
// matching today. They become drift the same way bench.mjs's stale
// allowlist entries do (R-Parity #3): a SKIP entry whose underlying
// divergence got fixed must be removed, otherwise the next reviewer
// thinks the fixture still needs work.
const staleSkips = [];

function runFixture(name, fixtureDir) {
    const userSkipped = name in SKIP_LIST || UPSTREAM_EXCLUDED.has(name);

    const expected = loadExpected(fixtureDir);
    if (!expected) {
        if (userSkipped) {
            // No expectations file is its own kind of skip — record
            // under the shared "skipped" bucket without flagging the
            // SKIP_LIST entry as stale.
            skipped++;
            skipReasons.push(`  SKIP: ${name} — ${
                name in SKIP_LIST
                    ? SKIP_LIST[name]
                    : 'upstream-root tsconfig excludes it'
            }`);
            return;
        }
        skipped++;
        skipReasons.push(`  SKIP: ${name} — no expectedv2.json / expected_svelte_5.json`);
        return;
    }
    if (expected.error) {
        failed++;
        failureReasons.push(`  FAIL: ${name} — ${expected.error}`);
        return;
    }

    if (userSkipped) {
        // Run the fixture anyway so we can detect stale skips.
        // Keep the slot in the SKIP bucket for the scoreboard, but
        // if we find a clean match we'll flag it as drift.
        const { diagnostics, crash } = runBinary(fixtureDir);
        if (!crash) {
            const expMs = makeMultiset(expected.list, compareKey);
            const actMs = makeMultiset(diagnostics, compareKey);
            const { missing, extra } = diffMultisets(expMs, actMs);
            if (missing.length === 0 && extra.length === 0) {
                staleSkips.push(name);
            }
        }
        skipped++;
        skipReasons.push(`  SKIP: ${name} — ${
            name in SKIP_LIST
                ? SKIP_LIST[name]
                : 'upstream-root tsconfig excludes it'
        }`);
        return;
    }

    const { diagnostics, crash } = runBinary(fixtureDir);
    if (crash) {
        failed++;
        failureReasons.push(`  FAIL: ${name} — binary crashed (${crash})`);
        return;
    }

    // Round-Parity #1: default-strict on `(file, line, character,
    // code)`. `LS_DIAGNOSTICS_LOOSE=1` falls back to the legacy
    // `(file, code)` multiset comparison.
    const expMs = makeMultiset(expected.list, compareKey);
    const actMs = makeMultiset(diagnostics, compareKey);
    const { missing, extra } = diffMultisets(expMs, actMs);

    if (missing.length === 0 && extra.length === 0) {
        passed++;
        console.log(`  PASS: ${name}`);
    } else {
        failed++;
        const lines = [`  FAIL: ${name}`];
        if (missing.length) {
            lines.push(`    missing (${missing.length}):`);
            for (const k of missing) lines.push(`      ${k}`);
        }
        if (extra.length) {
            lines.push(`    extra (${extra.length}):`);
            for (const k of extra) lines.push(`      ${k}`);
        }
        failureReasons.push(lines.join('\n'));
    }
}

console.log('svelte-check-native LS diagnostic fixtures\n');

const entries = readdirSync(FIXTURES_DIR).sort();
for (const entry of entries) {
    if (entry.startsWith('_') || entry === 'tsconfig.json') continue;
    const dir = path.join(FIXTURES_DIR, entry);
    try {
        if (!statSync(dir).isDirectory()) continue;
    } catch {
        continue;
    }
    runFixture(entry, dir);
}

if (skipReasons.length) {
    console.log('\nskipped:');
    for (const line of skipReasons) console.log(line);
}
if (failureReasons.length) {
    console.log('\nfailures:');
    for (const block of failureReasons) console.log(block);
}

// Round-Parity #2: stale skips count as failures. A SKIP_LIST entry
// (or UPSTREAM_EXCLUDED member) whose fixture now matches expected
// strictly must be removed from the list — otherwise the scoreboard
// shows an artificial work item that's already been done. Same
// discipline as bench.mjs's stale-allowlist treatment in #3.
let stalePenalty = 0;
if (staleSkips.length) {
    console.log('\nstale skips (now passing — remove from SKIP_LIST / UPSTREAM_EXCLUDED):');
    for (const name of staleSkips) {
        console.log(`  STALE: ${name}`);
    }
    stalePenalty = staleSkips.length;
}

console.log(
    `\n${passed} passed, ${failed + stalePenalty} failed${skipped ? `, ${skipped} skipped` : ''}` +
    (stalePenalty ? ` (incl. ${stalePenalty} stale skip${stalePenalty === 1 ? '' : 's'})` : '')
);
process.exit(failed + stalePenalty > 0 ? 1 : 0);
