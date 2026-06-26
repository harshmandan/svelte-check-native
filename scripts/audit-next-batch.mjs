// Select the next N files that DON'T yet have a review report on disk,
// and print the Workflow args JSON for scripts/audit-review.workflow.js.
//
// Resumable by construction: "done" = a reviews/<safe>.md exists, so a
// re-run after a crashed/limited session continues where it left off.
//
// Usage: node scripts/audit-next-batch.mjs <batchSize> [crateFilter]
import { readFileSync, existsSync, writeFileSync } from 'node:fs';
import { execSync } from 'node:child_process';

const N = parseInt(process.argv[2] || '20', 10);
const crateFilter = process.argv[3] || null;
const ROOT = 'notes/audit/active';
const REPORTS = `${ROOT}/reviews`;

const UPSTREAM = {
  parser:
    'language-tools/packages/svelte2tsx/src/utils/htmlxparser.ts + .../htmlxtojsx_v2/ (upstream parses via the svelte compiler; we hand-roll a template parser — judge correctness/completeness, not the choice to hand-roll)',
  analyze:
    'language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ (ExportedNames, ComponentEvents, Stores, slot, TemplateScope, ImplicitStoreValues, event-handler, Generics) + .../htmlxtojsx_v2/',
  emit:
    'language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/index.ts + .../htmlxtojsx_v2/nodes/* + .../svelte2tsx/{createRenderFunction,processInstanceScriptContent,addComponentExport,index}.ts',
  typecheck:
    'language-tools/packages/svelte-check/src/{incremental,index,tsgo}.ts',
  core: 'language-tools/packages/svelte2tsx/src/helpers/{sveltekit,files}.ts + language-tools/packages/svelte-check/src/utils.ts + TypeScript tsconfig semantics',
  'svn-lint':
    '.svelte-upstream/svelte/packages/svelte/src/compiler/phases/2-analyze/visitors/* + .svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*',
  cli: 'language-tools/packages/svelte-check/src/{options,writers,index}.ts',
  'svelte-compiler': '.svelte-upstream/svelte/packages/svelte/src/compiler/',
};

const manifest = JSON.parse(readFileSync(`${ROOT}/manifest.json`, 'utf8'));
const meta = JSON.parse(readFileSync(`${ROOT}/meta.json`, 'utf8'));
const sha = execSync('git rev-parse --short HEAD').toString().trim();
const now = execSync('TZ=Asia/Calcutta date "+%Y-%m-%dT%H:%M%z"').toString().trim();

const entries = Object.entries(manifest.files);
const pending = entries
  .filter(([p, m]) => !existsSync(`${REPORTS}/${m.safe}.md`))
  .filter(([p, m]) => !crateFilter || m.crate === crateFilter)
  .map(([p, m]) => ({ path: p, crate: m.crate, safe: m.safe }));

const done = entries.length - entries.filter(([p, m]) => !existsSync(`${REPORTS}/${m.safe}.md`)).length;
const batch = pending.slice(0, N);

const argsObj = { files: batch, reportsDir: REPORTS, upstreamMap: UPSTREAM, now, sha };
writeFileSync('/tmp/audit-batch-args.json', JSON.stringify(argsObj));

console.error(`progress: ${done}/${entries.length} reviewed, ${pending.length} pending`);
console.error(`this batch: ${batch.length} files${crateFilter ? ` (crate ${crateFilter})` : ''}`);
for (const f of batch) console.error(`  - ${f.path}`);
console.error(`args written to /tmp/audit-batch-args.json`);
// stdout: compact args (for piping/inspection)
console.log(JSON.stringify(argsObj));
