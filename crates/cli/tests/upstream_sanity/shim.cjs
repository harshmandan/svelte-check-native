// Node `--require` shim that redirects upstream svelte-check's
// `execFileSync('node', [dist/src/index.js, ...args])` invocation
// to our svelte-check-native binary.
//
// Loaded via:
//   node --require <path>/shim.cjs <path>/language-tools/.../test-sanity.js
//
// Env:
//   SVELTE_CHECK_BIN — absolute path to the svelte-check-native binary
//
// Why this exists:
// Upstream's test-sanity.js is a black-box subprocess test — it only cares
// what the binary prints to stdout. We intercept the spawn so their test file
// runs byte-for-byte unmodified against our Rust implementation. When they
// update their test, we bump the submodule and pick up the change for free.

'use strict';

const child_process = require('child_process');

const OUR_BIN = process.env.SVELTE_CHECK_BIN;
if (!OUR_BIN) {
    throw new Error(
        'shim.cjs: SVELTE_CHECK_BIN env var must point to the svelte-check-native binary'
    );
}

// Normalize backslashes so the match works on Windows.
const norm = (p) => String(p).replace(/\\/g, '/');

// Upstream calls execFileSync('node', [CLI, ...cliArgs]) where CLI ends in
// `/svelte-check/dist/src/index.js`.
function isUpstreamSvelteCheckCli(candidate) {
    return /\/svelte-check\/dist\/src\/index\.js$/.test(norm(candidate));
}

const realExecFileSync = child_process.execFileSync;

// The binary forces `--output machine` when it sees CLAUDECODE / GEMINI_CLI /
// CODEX_CI set to `"1"` in its environment (see crates/cli/src/main.rs).
// Upstream's test-sanity.js requests `--output machine-verbose` and parses
// JSON per line; inheriting those vars from an agentic parent shell would
// make every diagnostic invisible and every test falsely "pass". Blank them
// out when spawning our binary (not delete — the binary only rejects literal
// `"1"`, so `""` is the minimal override).
function blankAgentEnv(opts) {
    const base = (opts && opts.env) || process.env;
    return { ...opts, env: { ...base, CLAUDECODE: '', GEMINI_CLI: '', CODEX_CI: '' } };
}

child_process.execFileSync = function patchedExecFileSync(file, args, opts) {
    if (
        (file === 'node' || file === 'node.exe') &&
        Array.isArray(args) &&
        args.length > 0 &&
        isUpstreamSvelteCheckCli(args[0])
    ) {
        return realExecFileSync.call(this, OUR_BIN, args.slice(1), blankAgentEnv(opts));
    }
    return realExecFileSync.call(this, file, args, opts);
};
