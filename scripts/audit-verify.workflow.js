export const meta = {
  name: 'audit-verify-batch',
  description: 'Adversarially verify a BATCH of audit findings against code + upstream; each agent writes a durable verdict JSON to notes/audit/active/verdicts/. Resumable: re-run skips findings already verdicted.',
  phases: [{ title: 'Verify', detail: 'one skeptic per finding → writes its verdict' }],
}

// Batch is injected by scripts/audit-verify-batch.mjs (placeholder
// replaced with {findings, verdictsDir, upstreamMap}). Falls back to
// `args` when invoked with explicit args instead.
let A = /*__BATCH__*/ null;
if (!A) {
  A = args;
  if (typeof A === 'string') { try { A = JSON.parse(A); } catch (e) { A = {}; } }
}
A = A || {};
const FINDINGS = A.findings || [];
const UPSTREAM = A.upstreamMap || {};
const DIR = A.verdictsDir;
log(`Verifying ${FINDINGS.length} findings → ${DIR}/`);

const SCOPE = `STRICT SCOPE — never CONFIRM a finding that asks us to:
- implement LSP/editor features, watch mode, tsc support (tsgo-only), formatting, or CSS lint beyond the vendor-prefix carve-out;
- be STRICTER than upstream (firing a diagnostic upstream does NOT fire is a BUG, so REJECT such findings);
- re-implement TypeScript's own checks ("tsgo is trusted").
A byte-scanner that is a DOCUMENTED, sanctioned perf exception (template_refs / store / the const-tag void scan note) is acceptable — judge its CORRECTNESS, don't confirm "it exists" as a bug.`;

const SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    id: { type: 'string' },
    verdict: { type: 'string', enum: ['confirmed', 'rejected', 'adjusted'] },
    real: { type: 'boolean', description: 'is there a genuine, in-scope defect?' },
    severity: { type: 'string', enum: ['high', 'medium', 'low', 'none'] },
    evidence: { type: 'string', description: 'code/upstream evidence for the verdict (cite lines)' },
    concrete_fix: { type: 'string', description: 'if real: the precise fix; else empty' },
    risk: { type: 'string', enum: ['trivial', 'low', 'moderate', 'high'], description: 'fix regression risk' },
  },
  required: ['id', 'verdict', 'real', 'severity', 'evidence', 'concrete_fix', 'risk'],
}

const prompt = (f) => `You are an ADVERSARIAL VERIFIER hardening svelte-check-native (a Rust reimplementation of svelte-check/svelte2tsx, tsgo-powered) toward a flawless upstream-alternative. A single-agent review raised the finding below; decide whether it is a GENUINE, in-scope defect.

FINDING ${f.id} [${f.crate} · ${f.dimension}]
title: ${f.title}
location: ${f.location}
detail: ${f.detail}
suggested_fix: ${f.suggested_fix}
upstream_ref: ${f.upstream_ref}

DO THIS:
1. Read the cited code at the location (and surrounding context).
2. Read the upstream counterpart for crate "${f.crate}": ${UPSTREAM[f.crate] || 'n/a'} — find the specific function/logic and compare.
3. Decide: is the defect REAL, correctly characterized, IN SCOPE, and at the right severity? Reproduce mentally (or note the exact input that triggers it).

${SCOPE}

Default to REJECT when you cannot independently confirm from the code, when it's out-of-scope, when it asks us to be stricter than upstream, or when the cited behavior is actually correct. CONFIRM (or ADJUST severity) only with concrete code/upstream evidence; give the precise fix and a regression-risk estimate.

Then WRITE your verdict to: ${DIR}/${f.id}.md — frontmatter:
---
id: ${f.id}
verdict: <confirmed|rejected|adjusted>
real: <true|false>
severity: <high|medium|low|none>
risk: <trivial|low|moderate|high>
location: ${f.location}
crate: ${f.crate}
---
then a short "## Evidence" and (if real) "## Fix" section.

Return the JSON verdict.`;

const results = await parallel(
  FINDINGS.map((f) => () => agent(prompt(f), { label: `verify:${f.id}`, phase: 'Verify', schema: SCHEMA }))
);
const ok = results.filter(Boolean);
const confirmed = ok.filter((v) => v.real && v.verdict !== 'rejected').length;
log(`Batch done: ${ok.length}/${FINDINGS.length} verdicts, ${confirmed} real.`);
return { batch: FINDINGS.length, verdicts: ok };
