export const meta = {
  name: 'audit-review-batch',
  description: 'Review a BATCH of files against upstream; each agent writes a durable per-file Markdown report to notes/audit/active/reviews/ (survives session loss). Resumable: re-run skips files already reported.',
  phases: [{ title: 'Review', detail: 'one agent per file → writes its own MD report' }],
}

// args: { files:[{path,crate,safe}], reportsDir, upstreamMap, now, sha }
let A = args;
if (typeof A === 'string') {
  try { A = JSON.parse(A); } catch (e) { A = {}; }
}
A = A || {};
log(`args received: type=${typeof args}, files=${(A.files || []).length}, reportsDir=${A.reportsDir || '?'}`);
const FILES = A.files || [];
const UPSTREAM = A.upstreamMap || {};
const REPORTS = A.reportsDir;
const NOW = A.now;
const SHA = A.sha;

const SCOPE = `STRICT SCOPE — DELIBERATE non-goals; never report as gaps/incompleteness:
LSP/editor features (hover, completion, go-to-def, rename, code actions, on-type diagnostics),
watch mode, tsc support (this project is tsgo-ONLY by design), formatting, and CSS lint beyond
the vendor-prefix carve-out. PHILOSOPHY: parity with upstream's CLI type-checker surface — same
errors/warnings/counts/exit codes. Being STRICTER than upstream is a BUG. "tsgo is trusted" — we
never re-implement TypeScript's own checks. Synthesized overlay names use the __svn_ prefix.`;

const SUMMARY_SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    file: { type: 'string' },
    report_written: { type: 'boolean' },
    grade: { type: 'string', enum: ['A', 'B', 'C', 'D', 'F'] },
    findings_count: { type: 'integer' },
    top_severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low', 'none'] },
    architecture_alignment: { type: 'string', enum: ['aligned', 'deviates-better', 'deviates-should-mimic', 'n/a'] },
  },
  required: ['file', 'report_written', 'grade', 'findings_count', 'top_severity', 'architecture_alignment'],
}

const prompt = (f) => `You are auditing ONE file of svelte-check-native (a Rust reimplementation of sveltejs/language-tools' svelte-check + svelte2tsx, tsgo-powered) against UPSTREAM, for submission as a FLAWLESS drop-in alternative to the official repo. Produce a durable Markdown review report.

FILE UNDER REVIEW: ${f.path} (crate "${f.crate}"). Read the ENTIRE file.
UPSTREAM SOURCE OF TRUTH for crate "${f.crate}": ${UPSTREAM[f.crate] || 'n/a'}
Find and READ the upstream counterpart(s) of THIS file's concern (Glob/Grep/Read; upstream is TypeScript). Compare directly.

Review across ALL of these goals:
1. CORRECTNESS / PARITY / BUGS — wrong logic, off-by-one, byte-vs-char offset errors, broken Unicode, panics, unwrap()/expect() on fallible paths w/o an invariant, missing error handling, mishandled oxc AST, edge cases (empty/nested/malformed input, delimiters inside strings/comments), and behavioral divergence vs upstream (IN SCOPE).
2. COMPLETENESS vs the upstream counterpart.
3. DATA STRUCTURES — Vec where a Set/Map is right, repeated O(n) scans, missing capacity pre-alloc (rule #5), needless clone()/String where borrow/&str/SmolStr/Cow fits.
4. PARSING/READING APPROACH — rule #1 forbids char-level scanners for embedded JS/TS (must walk the oxc AST). Flag hand-rolled JS/TS byte scanning. (Template-structure scanning in the parser crate is fine.) Note: template_refs.rs's JS tokenizer is a DOCUMENTED, sanctioned perf exception — judge its correctness, don't flag its existence.
5. CODE QUALITY / IDIOM / flawlessness; unwrap()/expect() lacking an invariant comment.
6. FILE ARCHITECTURE vs upstream (NEW GOAL) — which upstream file(s) does this file correspond to? Is the file's existence, NAME, and concern-boundary aligned with how upstream organizes the same logic? Do we split/merge concerns differently? Where we DEVIATE: is ours clearly BETTER (justify) or should we mimic upstream's layout? Flag logic that lives here but belongs in a sibling module to match upstream's structure. (One-to-one isn't required — we skip LSP/tsc and do some things differently — but the organization should be SIMILAR unless deviating is clearly better.)

${SCOPE}

Substantiate every finding by reading actual code; cite exact lines + the upstream reference. Do NOT pad with speculation — a clean file should report few/no findings.

NOW WRITE your report to this exact path using the Write tool: ${REPORTS}/${f.safe}.md
Use EXACTLY this structure (machine-consolidated later):

---
file: ${f.path}
crate: ${f.crate}
upstream_ref: <upstream file(s) compared against>
reviewed_at: ${NOW}
source_sha: ${SHA}
grade: <A|B|C|D|F>
---

# Review: \`${f.path}\`

## Summary
<2-3 sentences on overall health>

## Findings
<for each finding:>
### [<critical|high|medium|low> · <bug|completeness|data-structure|parsing|quality>] <title>
- **location:** ${f.path}:<line(s)>
- **upstream_ref:** <upstream file:line or "n/a">
- **detail:** <what's wrong AND why, vs upstream/correctness>
- **suggested_fix:** <concrete fix>
- **confidence:** <high|medium|low>
<if none: write "No findings.">

## File architecture
- **corresponds_to:** <upstream file(s), or "no direct counterpart">
- **alignment:** <aligned | deviates-better | deviates-should-mimic | n/a>
- **notes:** <how our file maps to upstream's organization; any logic that should move to mirror upstream; whether a deviation is justified>

After writing the file, return the JSON summary.`;

const results = await parallel(
  FILES.map((f) => () =>
    agent(prompt(f), { label: `review:${f.path}`, phase: 'Review', schema: SUMMARY_SCHEMA })
  )
);

const ok = results.filter(Boolean);
log(`Batch complete: ${ok.length}/${FILES.length} reports written to ${REPORTS}/`);
return {
  batch_size: FILES.length,
  reports_written: ok.filter((r) => r.report_written).length,
  summaries: ok,
};
