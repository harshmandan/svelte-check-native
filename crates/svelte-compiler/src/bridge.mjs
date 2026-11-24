// svelte-check-native compiler-warnings bridge
//
// Persistent subprocess that imports the user's `svelte/compiler` and
// returns warnings + parse errors via line-delimited JSON over stdio.
//
// Protocol — one JSON object per line in each direction:
//   request:  {"id": N, "filename": "...", "source": "..."}
//   response: {"id": N, "warnings": [...], "error": "<optional>"}
//
// A request with `"rejection": true` is answered by `rejection()` below
// instead: {"id": N, "compileError": <error|null>, "parseError": <error|null>}.
//
// Each warning has `{code, message, start: {line, column}, end: {line,
// column}}` where line is 1-based, column is 0-based (matching
// svelte/compiler's locate-character output).
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
// svelte >= 5 publishes a CJS bundle at `./compiler/index.js`. Node's
// ESM-import-of-CJS shows only `default` on the namespace (the whole
// module.exports object), while Bun synthesizes named exports. Both
// runtimes expose the same function as `mod.default.compile` though,
// so prefer the named export and fall back through the default. If
// neither yields a function the bridge is in a bad state and we fail
// loudly rather than letting every request report "compile is not a
// function" per file.
const _svelteMod = await import(pathToFileURL(svelteCompilerPath).href)
const compile = _svelteMod.compile ?? _svelteMod.default?.compile
if (typeof compile !== 'function') {
  console.error(
    'bridge: no `compile` export found in ' + svelteCompilerPath +
      ' (keys: ' + Object.keys(_svelteMod).join(', ') + ')',
  )
  process.exit(2)
}

// Optional: absolute path to the user's svelte.config.{js,mjs,cjs}
// (argv[3]) and statically read compilerOptions JSON (argv[4]): the
// inline options of a `svelte()` / `sveltekit()` Vite plugin, or the
// options of a config the runtime can't import. Loaded once at startup.
// The JSON is deep-merged over the imported config's options, objects
// key by key and anything else replaced, as vite-plugin-svelte merges
// its inline options over `svelte.config.*`.
const svelteConfigPath = process.argv[3] || ''
const staticCompilerOptionsJson = process.argv[4] || ''

function isPlainObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function deepMerge(under, over) {
  if (!isPlainObject(under) || !isPlainObject(over)) return over
  const out = { ...under }
  for (const [key, value] of Object.entries(over)) {
    out[key] = key in out ? deepMerge(out[key], value) : value
  }
  return out
}

let userCompilerOptions = {}
if (svelteConfigPath) {
  try {
    const mod = await import(pathToFileURL(svelteConfigPath).href)
    // svelte.config.js is conventionally ESM with a default export;
    // tolerate CJS-style { compilerOptions } at the module root too.
    const cfg = mod.default ?? mod
    if (cfg && typeof cfg === 'object' && cfg.compilerOptions) {
      userCompilerOptions = cfg.compilerOptions
    }
  } catch (e) {
    console.error(
      'bridge: failed to load svelte.config (' + svelteConfigPath + '): ' +
        (e?.message || String(e)) + ' — proceeding with defaults',
    )
  }
}
if (staticCompilerOptionsJson) {
  try {
    const parsed = JSON.parse(staticCompilerOptionsJson)
    if (isPlainObject(parsed)) {
      userCompilerOptions = deepMerge(userCompilerOptions, parsed)
    }
  } catch (e) {
    console.error(
      'bridge: failed to parse static compilerOptions JSON: ' +
        (e?.message || String(e)) + ' — proceeding with defaults',
    )
  }
}

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
  if (req.rejection) {
    process.stdout.write(JSON.stringify({ id: req.id, ...rejection(req) }) + '\n')
    return
  }
  try {
    const result = compile(req.source, {
      // User compilerOptions first so our explicit overrides win.
      ...userCompilerOptions,
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
    start: w.start || { line: 1, column: 0 },
    end: w.end || w.start || { line: 1, column: 0 },
  }
}

// What svelte-check's default engine reports for a component svelte2tsx
// can't convert. Its Svelte pass compiles the component with the config's
// `compilerOptions` exactly as given (`compileError`), and its TypeScript
// pass reports svelte2tsx's own failure (`parseError`): the compiler's
// `parse()` run on the source with script and style contents blanked out,
// as svelte2tsx's `parseHtmlx` does it. Either is null when it doesn't
// throw. An error without a position counts as at the start of the file.
function rejection(req) {
  const asError = (e) => {
    const start = e?.start ?? { line: 1, column: 0 }
    return {
      code: e?.code,
      message: e?.message || String(e),
      severity: 'error',
      start,
      end: e?.end ?? start,
    }
  }
  let compileError = null
  try {
    compile(req.source, { ...userCompilerOptions, filename: req.filename })
  } catch (e) {
    compileError = asError(e)
  }
  let parseError = null
  try {
    const svelte5Plus = Number(String(_svelteMod.VERSION ?? _svelteMod.default?.VERSION)[0]) > 4
    const parse = _svelteMod.parse ?? _svelteMod.default?.parse
    const blanked = blankVerbatimContent(req.source, findVerbatimElements(req.source))
    parse(blanked, svelte5Plus ? { loose: false } : undefined)
  } catch (e) {
    parseError = asError(e)
  }
  return { compileError, parseError }
}

// svelte2tsx's `htmlxparser.ts`, kept as it is there.
const scriptRegex =
  /(<!--[^]*?-->)|(<script((?:\s+[^=>'"\/\s]+=(?:"[^"]*"|'[^']*'|[^>\s]+)|\s+[^=>'"\/\s]+)*\s*)>)([\S\s]*?)<\/script>/g
const styleRegex =
  /(<!--[^]*?-->)|(<style((?:\s+[^=>'"\/\s]+=(?:"[^"]*"|'[^']*'|[^>\s]+)|\s+[^=>'"\/\s]+)*\s*)>)([\S\s]*?)<\/style>/g

function toVerbatimContent(match) {
  const content = match[4] || ''
  const start = match.index + match[2].length
  return { containerStart: match.index, start, end: start + content.length }
}

function findNextVerbatimElement(htmlx, from) {
  let best = null
  for (const exp of [scriptRegex, styleRegex]) {
    exp.lastIndex = from
    let match = null
    while ((match = exp.exec(htmlx)) != null) {
      if (match[0].startsWith('<!--')) {
        continue
      }
      if (!best || match.index < best.node.containerStart) {
        best = { node: toVerbatimContent(match), nextIndex: exp.lastIndex }
      }
      break
    }
  }
  return best
}

function findVerbatimElements(htmlx) {
  const tags = []
  let from = 0
  let next
  while ((next = findNextVerbatimElement(htmlx, from)) != null) {
    tags.push(next.node)
    from = next.nextIndex
  }
  return tags
}

function blankVerbatimContent(htmlx, verbatimElements) {
  let output = htmlx
  for (const content of verbatimElements) {
    output =
      output.substring(0, content.start) +
      output
        .substring(content.start, content.end)
        .replace(/[^\n]/g, ' ')
        .replace(/[^\n][^\n][^\n][^\n]\n/g, '/**/\n') +
      output.substring(content.end)
  }
  return output
}
