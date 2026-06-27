// Aggregate the per-finding verdict MDs in notes/audit/active/verdicts/
// into a single triaged fix list. Parses the YAML-ish frontmatter
// (verdict/real/severity/risk/location/crate) and the ## Evidence / ## Fix
// body sections. Prints a prioritized worklist: confirmed-real findings
// first (by severity then risk), then adjusted, then rejected (collapsed).
//
// Usage: node scripts/audit-triage.mjs
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';

const DIR = 'notes/audit/active/verdicts';
const files = readdirSync(DIR).filter((f) => f.endsWith('.md'));

const fm = (body, key) => {
  const m = body.match(new RegExp(`^${key}:\\s*(.+)$`, 'm'));
  return m ? m[1].trim() : '';
};
const section = (body, name) => {
  const m = body.match(new RegExp(`##\\s*${name}\\s*\\n([\\s\\S]*?)(?=\\n##\\s|$)`));
  return m ? m[1].trim() : '';
};

const verdicts = files.map((f) => {
  const raw = readFileSync(`${DIR}/${f}`, 'utf8');
  const fmBlock = raw.match(/^---\n([\s\S]*?)\n---/);
  const head = fmBlock ? fmBlock[1] : raw;
  return {
    id: fm(head, 'id') || f.replace('.md', ''),
    verdict: fm(head, 'verdict'),
    real: fm(head, 'real') === 'true',
    severity: fm(head, 'severity') || 'none',
    risk: fm(head, 'risk') || 'unknown',
    crate: fm(head, 'crate'),
    location: fm(head, 'location'),
    evidence: section(raw, 'Evidence'),
    fix: section(raw, 'Fix'),
  };
});

const sevRank = { high: 0, medium: 1, low: 2, none: 3 };
const riskRank = { trivial: 0, low: 1, moderate: 2, high: 3, unknown: 4 };

const real = verdicts
  .filter((v) => v.real && v.verdict !== 'rejected')
  .sort((a, b) => sevRank[a.severity] - sevRank[b.severity] || riskRank[a.risk] - riskRank[b.risk]);
const rejected = verdicts.filter((v) => !v.real || v.verdict === 'rejected');

let out = `# Medium-findings triage (consolidated verdicts)\n\n`;
out += `Total verdicts: ${verdicts.length} · confirmed-real: ${real.length} · rejected/not-real: ${rejected.length}\n\n`;
out += `## Confirmed-real (fix order: severity → risk)\n\n`;
for (const v of real) {
  out += `### ${v.id} — ${v.severity}/${v.risk} [${v.crate}]\n`;
  out += `- **where:** \`${v.location}\`\n`;
  if (v.fix) out += `- **fix:** ${v.fix.replace(/\n+/g, ' ')}\n`;
  if (v.evidence) out += `- **why:** ${v.evidence.replace(/\n+/g, ' ').slice(0, 400)}\n`;
  out += `\n`;
}
out += `## Rejected / not-real (${rejected.length})\n\n`;
for (const v of rejected) {
  out += `- **${v.id}** [${v.crate}] — ${v.verdict || 'rejected'}: ${(v.evidence || '').replace(/\n+/g, ' ').slice(0, 180)}\n`;
}

writeFileSync(`${DIR}/../TRIAGE.md`, out);
console.error(`triaged ${verdicts.length} verdicts → ${DIR}/../TRIAGE.md`);
console.error(`confirmed-real: ${real.length} | rejected: ${rejected.length}`);
console.log(JSON.stringify(real.map((v) => ({ id: v.id, sev: v.severity, risk: v.risk, crate: v.crate, loc: v.location })), null, 2));
