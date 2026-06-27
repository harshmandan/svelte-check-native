// Bake the file-grouped low-fix plan into a runnable workflow script.
// Reads /tmp/low-fix-plan.json (built inline earlier) and writes
// /tmp/low-fix-run.js with the plan injected. Optionally restrict to a
// crate prefix so fixes can land crate-by-crate.
//
// Usage: node scripts/audit-fix-bake.mjs [cratePrefix]
//   e.g. node scripts/audit-fix-bake.mjs crates/svn-lint
import { readFileSync, writeFileSync } from 'node:fs';

const PREFIX = process.argv[2] || '';
const plan = JSON.parse(readFileSync('/tmp/low-fix-plan.json', 'utf8'));
const filtered = {};
for (const [file, findings] of Object.entries(plan)) {
  if (PREFIX && !file.startsWith(PREFIX)) continue;
  // Keep only the fields the agent needs; drop bulky evidence.
  filtered[file] = findings.map((f) => ({ id: f.id, location: f.location, risk: f.risk, fix: f.fix }));
}
const tmpl = readFileSync('scripts/audit-fix.workflow.js', 'utf8');
const baked = tmpl.replace('/*__PLAN__*/ null', () => JSON.stringify(filtered));
writeFileSync('/tmp/low-fix-run.js', baked);

const nf = Object.keys(filtered).length;
const n = Object.values(filtered).reduce((a, b) => a + b.length, 0);
console.error(`baked ${n} findings across ${nf} files${PREFIX ? ` (prefix ${PREFIX})` : ''} → /tmp/low-fix-run.js`);
console.log('/tmp/low-fix-run.js');
