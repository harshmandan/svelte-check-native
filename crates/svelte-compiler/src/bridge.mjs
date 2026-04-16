// svelte-check-native compiler-warnings bridge
//
// Persistent subprocess that imports the user's `svelte/compiler` and
// returns warnings + parse errors via line-delimited JSON over stdio.
//
// Protocol — one JSON object per line in each direction:
//   request:  {"id": N, "filename": "...", "source": "..."}
//   response: {"id": N, "warnings": [...], "error": "<optional>"}
//
// Each warning has `{code, message, start: {line, column}, end: {line,
// column}}`. Lines and columns are 1-based, matching svelte/compiler's
// own format.
//
// The bridge is started once per `svelte-check-native` run; the Rust
// side keeps the subprocess open and reuses it across all .svelte files
// to avoid the import-once / module-eval cost on every file.

// `svelte/compiler` is loaded via dynamic import using the user-provided
// absolute path (passed as the first CLI argument). ESM lookup-by-name
// from a temp-dir bridge would otherwise fail — node's ES module
// resolution doesn't honor NODE_PATH and bun's cache layout doesn't
// support self-references for some transitive deps. A direct
// file-URL import sidesteps both quirks.
import { createInterface } from 'node:readline'
import { pathToFileURL } from 'node:url'

const svelteCompilerPath = process.argv[2]
if (!svelteCompilerPath) {
  console.error('bridge: missing svelte/compiler path argument')
  process.exit(2)
}
const { compile } = await import(pathToFileURL(svelteCompilerPath).href)

const rl = createInterface({ input: process.stdin, crlfDelay: Infinity })

rl.on('line', (line) => {
  if (!line) return
  let req
  try {
    req = JSON.parse(line)
  } catch {
    // Garbage on stdin — emit a generic error so the host knows.
    process.stdout.write(
      JSON.stringify({ id: -1, warnings: [], error: 'malformed request' }) + '\n',
    )
    return
  }
  try {
    const result = compile(req.source, {
      filename: req.filename,
      // Skip codegen — we only care about diagnostics.
      generate: false,
      // Match the dev-time experience the user actually runs.
      dev: true,
    })
    const warnings = (result.warnings || []).map(serializeWarning)
    process.stdout.write(JSON.stringify({ id: req.id, warnings }) + '\n')
  } catch (e) {
    // Compiler threw — usually a parse error. Emit it as a single
    // warning entry with code='parse-error' so it surfaces in the
    // diagnostic stream rather than getting silently swallowed.
    const start = e?.start ?? null
    const end = e?.end ?? start
    const warnings = []
    if (start) {
      warnings.push({
        code: e?.code || 'compile_error',
        message: e?.message || String(e),
        severity: 'error',
        start,
        end: end || start,
      })
    }
    process.stdout.write(
      JSON.stringify({
        id: req.id,
        warnings,
        error: warnings.length === 0 ? String(e?.message || e) : undefined,
      }) + '\n',
    )
  }
})

function serializeWarning(w) {
  return {
    code: w.code || 'unknown',
    message: w.message || '',
    severity: 'warning',
    start: w.start || { line: 1, column: 1 },
    end: w.end || w.start || { line: 1, column: 1 },
  }
}
