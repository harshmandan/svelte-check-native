// Select the next N findings (from /tmp/medium-worklist.json) that don't
// yet have a verdict on disk, and print the Workflow args JSON for
// scripts/audit-verify.workflow.js. Resumable by filesystem state.
//
// Usage: node scripts/audit-verify-batch.mjs <batchSize> [worklistPath]
import { readFileSync, existsSync, writeFileSync } from 'node:fs';

const N = parseInt(process.argv[2] || '30', 10);
const WORKLIST = process.argv[3] || '/tmp/medium-worklist.json';
const DIR = 'notes/audit/active/verdicts';

const UPSTREAM = {
  parser: 'language-tools/packages/svelte2tsx/src/utils/htmlxparser.ts + .../htmlxtojsx_v2/',
  analyze: 'language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/* + .../htmlxtojsx_v2/',
  emit: 'language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/* + .../svelte2tsx/{createRenderFunction,processInstanceScriptContent,addComponentExport}.ts',
  typecheck: 'language-tools/packages/svelte-check/src/{incremental,index,tsgo}.ts',
  core: 'language-tools/packages/svelte2tsx/src/helpers/{sveltekit,files}.ts + svelte-check/src/utils.ts + TypeScript tsconfig semantics',
  'svn-lint': '.svelte-upstream/svelte/packages/svelte/src/compiler/phases/2-analyze/visitors/* + messages/compile-warnings/*',
  cli: 'language-tools/packages/svelte-check/src/{options,writers,index}.ts',
  'svelte-compiler': '.svelte-upstream/svelte/packages/svelte/src/compiler/',
};

const work = JSON.parse(readFileSync(WORKLIST, 'utf8'));
const pending = work.filter((f) => !existsSync(`${DIR}/${f.id}.md`));
const batch = pending.slice(0, N);
const argsObj = { findings: batch, verdictsDir: DIR, upstreamMap: UPSTREAM };
// Bake the batch into a ready-to-run script (avoids huge inline args).
const tmpl = readFileSync('scripts/audit-verify.workflow.js', 'utf8');
// Function replacer: avoids String.replace's `$&`/`$'` special patterns,
// which would corrupt the bake (findings contain `$props`, `$'`, etc.).
const baked = tmpl.replace('/*__BATCH__*/ null', () => JSON.stringify(argsObj));
writeFileSync('/tmp/audit-verify-run.js', baked);

console.error(`progress: ${work.length - pending.length}/${work.length} verdicted, ${pending.length} pending`);
console.error(`this batch: ${batch.length} (${batch.map((f) => f.id).join(',')})`);
console.error('baked → /tmp/audit-verify-run.js');
console.log('/tmp/audit-verify-run.js');
