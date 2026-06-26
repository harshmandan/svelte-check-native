// Consolidate all per-file review reports in notes/audit/active/reviews/
// into a ranked findings list + per-crate grade/architecture summary.
//
// Parses each report's frontmatter (file/crate/grade/upstream_ref), its
// "## Findings" section (### [severity · dimension] title + bullets), and
// its "## File architecture" section (corresponds_to/alignment/notes).
//
// Outputs:
//   notes/audit/active/findings.json   (structured, for verify/fix)
//   notes/audit/active/CONSOLIDATED.md (human report)
//
// Usage: node scripts/audit-consolidate.mjs
import { readFileSync, writeFileSync, readdirSync } from 'node:fs';

const DIR = 'notes/audit/active/reviews';
const sevOrder = { critical: 0, high: 1, medium: 2, low: 3 };

function parseFrontmatter(text) {
  const m = text.match(/^---\n([\s\S]*?)\n---/);
  const fm = {};
  if (m) for (const line of m[1].split('\n')) {
    const i = line.indexOf(':');
    if (i > 0) fm[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return fm;
}

function section(text, heading) {
  const re = new RegExp(`^##\\s+${heading}\\s*$`, 'm');
  const m = re.exec(text);
  if (!m) return '';
  const start = m.index + m[0].length;
  const next = /^##\s+/m.exec(text.slice(start));
  return text.slice(start, next ? start + next.index : undefined);
}

function bullet(block, key) {
  // matches "- **key:** value" (value may wrap to indented continuation lines)
  const re = new RegExp(`-\\s*\\*\\*${key}:?\\*\\*\\s*([^\\n]*(?:\\n(?!\\s*-\\s*\\*\\*|^#)[^\\n]*)*)`, 'i');
  const m = re.exec(block);
  return m ? m[1].replace(/\s+/g, ' ').trim() : '';
}

const files = readdirSync(DIR).filter((f) => f.endsWith('.md'));
const reports = [];
const allFindings = [];

for (const f of files) {
  const text = readFileSync(`${DIR}/${f}`, 'utf8');
  const fm = parseFrontmatter(text);
  const findingsBlock = section(text, 'Findings');
  const archBlock = section(text, 'File architecture');

  const findings = [];
  // split on "### [..]" headings
  const parts = findingsBlock.split(/^###\s+/m).slice(1);
  for (const p of parts) {
    const head = p.split('\n')[0];
    const hm = head.match(/\[\s*(critical|high|medium|low)\s*[·.|,-]\s*([a-z-]+)\s*\]\s*(.*)/i);
    const severity = hm ? hm[1].toLowerCase() : 'low';
    const dimension = hm ? hm[2].toLowerCase() : 'quality';
    const title = hm ? hm[3].trim() : head.trim();
    findings.push({
      file: fm.file || f,
      crate: fm.crate || '?',
      severity,
      dimension,
      title,
      location: bullet(p, 'location') || (fm.file || ''),
      detail: bullet(p, 'detail'),
      suggested_fix: bullet(p, 'suggested_fix') || bullet(p, 'suggested fix'),
      upstream_ref: bullet(p, 'upstream_ref') || bullet(p, 'upstream ref'),
      confidence: bullet(p, 'confidence') || 'medium',
      report: f,
    });
  }
  for (const x of findings) allFindings.push(x);

  reports.push({
    file: fm.file || f,
    crate: fm.crate || '?',
    grade: fm.grade || '?',
    upstream_ref: fm.upstream_ref || '',
    corresponds_to: bullet(archBlock, 'corresponds_to') || bullet(archBlock, 'corresponds to'),
    alignment: (bullet(archBlock, 'alignment') || '').toLowerCase(),
    arch_notes: bullet(archBlock, 'notes'),
    findings_count: findings.length,
    report: f,
  });
}

allFindings.sort(
  (a, b) => (sevOrder[a.severity] ?? 9) - (sevOrder[b.severity] ?? 9) || a.crate.localeCompare(b.crate)
);

writeFileSync(
  'notes/audit/active/findings.json',
  JSON.stringify({ reports, findings: allFindings }, null, 1)
);

// ---- human report ----
const bySev = {};
for (const x of allFindings) bySev[x.severity] = (bySev[x.severity] || 0) + 1;
const crates = [...new Set(reports.map((r) => r.crate))].sort();
const gradeNum = { A: 4, B: 3, C: 2, D: 1, F: 0 };

const out = [];
out.push('# Consolidated upstream audit', '');
out.push(`Reports: **${reports.length} files** · Findings: **${allFindings.length}** ` +
  `(critical ${bySev.critical || 0}, high ${bySev.high || 0}, medium ${bySev.medium || 0}, low ${bySev.low || 0})`, '');

out.push('## Per-crate summary', '', '| crate | files | avg grade | findings | should-mimic-upstream |', '|---|--:|:--|--:|--:|');
for (const c of crates) {
  const rs = reports.filter((r) => r.crate === c);
  const graded = rs.filter((r) => gradeNum[r.grade] != null);
  const avg = graded.length ? (graded.reduce((s, r) => s + gradeNum[r.grade], 0) / graded.length) : 0;
  const avgL = ['F', 'D', 'C', 'B', 'A'][Math.round(avg)] || '?';
  const fc = allFindings.filter((x) => x.crate === c).length;
  const mimic = rs.filter((r) => r.alignment === 'deviates-should-mimic').length;
  out.push(`| ${c} | ${rs.length} | ${avgL} (${avg.toFixed(2)}) | ${fc} | ${mimic} |`);
}
out.push('');

out.push('## File-architecture: files flagged "should mimic upstream"', '');
const mimic = reports.filter((r) => r.alignment === 'deviates-should-mimic');
if (!mimic.length) out.push('_None._', '');
for (const r of mimic) out.push(`- **${r.file}** → ${r.corresponds_to || '?'} — ${r.arch_notes || ''}`);
out.push('');

for (const sev of ['critical', 'high', 'medium']) {
  const items = allFindings.filter((x) => x.severity === sev);
  out.push(`## ${sev.toUpperCase()} findings (${items.length})`, '');
  for (const x of items) {
    out.push(`### [${x.dimension}] ${x.title}`);
    out.push(`- **where:** \`${x.location}\` (${x.crate})`);
    if (x.detail) out.push(`- **detail:** ${x.detail}`);
    if (x.suggested_fix) out.push(`- **fix:** ${x.suggested_fix}`);
    if (x.upstream_ref) out.push(`- **upstream:** ${x.upstream_ref}`);
    out.push(`- _report: ${x.report}, confidence: ${x.confidence}_`, '');
  }
}
out.push('## LOW findings', '', `_${(bySev.low || 0)} low-severity findings — see findings.json._`, '');

writeFileSync('notes/audit/active/CONSOLIDATED.md', out.join('\n'));
console.log(`consolidated ${reports.length} reports, ${allFindings.length} findings`);
console.log(`severities: ${JSON.stringify(bySev)}`);
console.log('wrote notes/audit/active/{findings.json,CONSOLIDATED.md}');
