// Transpiles TypeScript the way svelte-check's fallback preprocessor
// does: `ts.transpileModule` from the workspace's own `typescript`,
// with that preprocessor's options.
//
//   <runtime> transpile.mjs <workspace>
//
// stdin:  a JSON array of { fileName, content }
// stdout: a JSON array with, for each, { code, mappings } (the printed
//         JavaScript and its source map's mappings), or null where the
//         transpile threw. `null` alone when the workspace has no
//         `typescript` that can transpile.

import { createRequire } from 'node:module'
import { join } from 'node:path'

let ts = null
try {
  ts = createRequire(join(process.argv[2], 'package.json'))('typescript')
} catch {}

let input = ''
process.stdin.setEncoding('utf8')
for await (const chunk of process.stdin) input += chunk

if (typeof ts?.transpileModule !== 'function') {
  process.stdout.write('null')
} else {
  const printed = JSON.parse(input).map(({ fileName, content }) => {
    try {
      const { outputText, sourceMapText } = ts.transpileModule(content, {
        fileName,
        compilerOptions: {
          module: ts.ModuleKind.ESNext,
          target: ts.ScriptTarget.ESNext,
          sourceMap: true,
          verbatimModuleSyntax: true,
        },
      })
      return { code: outputText, mappings: JSON.parse(sourceMapText).mappings }
    } catch {
      return null
    }
  })
  process.stdout.write(JSON.stringify(printed))
}
