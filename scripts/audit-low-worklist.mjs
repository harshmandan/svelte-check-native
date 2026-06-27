// Build an ID'd, signal-ordered worklist of the low-severity audit
// findings for the adversarial-verify pass. Reads findings.json, filters
// severity=low, assigns stable l### IDs (by original order so reruns are
// deterministic), and orders by dimension signal — highest-yield first
// (bug/correctness/parity/completeness), doc-style `quality` last.
//
// Usage: node scripts/audit-low-worklist.mjs  → writes /tmp/low-worklist.json
import { readFileSync, writeFileSync } from 'node:fs';

const f = JSON.parse(readFileSync('notes/audit/active/findings.json', 'utf8'));
const arr = Array.isArray(f) ? f : f.findings || [];
const low = arr.filter((x) => (x.severity || '').toLowerCase() === 'low');

// Stable IDs in source order (l001..), independent of the later sort.
low.forEach((x, i) => {
  x.id = `l${String(i + 1).padStart(3, '0')}`;
});

const PRIORITY = {
  correctness: 0,
  bug: 1,
  parity: 2,
  completeness: 3,
  'data-structure': 4,
  parsing: 5,
  architecture: 6,
  quality: 7,
};
const rank = (d) => (d in PRIORITY ? PRIORITY[d] : 8);

const ordered = [...low].sort((a, b) => rank(a.dimension) - rank(b.dimension));

const worklist = ordered.map((x) => ({
  id: x.id,
  crate: x.crate,
  dimension: x.dimension,
  title: x.title,
  location: x.location,
  detail: x.detail,
  suggested_fix: x.suggested_fix || '',
  upstream_ref: x.upstream_ref || '',
}));

writeFileSync('/tmp/low-worklist.json', JSON.stringify(worklist, null, 0));

const counts = {};
for (const x of worklist) counts[x.dimension] = (counts[x.dimension] || 0) + 1;
console.error(`low worklist: ${worklist.length} findings → /tmp/low-worklist.json`);
console.error(`order: ${Object.entries(counts).map(([d, n]) => `${d}=${n}`).join(' ')}`);
