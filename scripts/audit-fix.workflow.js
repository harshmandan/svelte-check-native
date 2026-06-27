export const meta = {
  name: 'audit-fix-lows',
  description: 'Apply the verified low-severity audit fixes, one agent per file (no edit conflicts). Each agent reads the verdict MDs for its file and applies ONLY the documented fix.',
  phases: [{ title: 'Fix', detail: 'one agent per file applies its confirmed fixes' }],
}

// File→findings plan baked in by scripts/audit-fix-bake.mjs.
let PLAN = /*__PLAN__*/ null;
if (!PLAN) { PLAN = args || {}; if (typeof PLAN === 'string') { try { PLAN = JSON.parse(PLAN); } catch (e) { PLAN = {}; } } }
const FILES = Object.entries(PLAN);
log(`Applying low fixes across ${FILES.length} files`);

const SCHEMA = {
  type: 'object', additionalProperties: false,
  properties: {
    file: { type: 'string' },
    applied: { type: 'array', items: { type: 'string' }, description: 'finding ids actually changed' },
    skipped: { type: 'array', items: { type: 'string' }, description: 'finding ids skipped (already-fixed/stale/unsafe)' },
    notes: { type: 'string', description: 'one line: what changed + any skip reasons' },
    behavioral: { type: 'boolean', description: 'true if any change alters emit/lint OUTPUT (snapshots may need re-locking)' },
  },
  required: ['file', 'applied', 'skipped', 'notes', 'behavioral'],
}

const prompt = (file, findings) => `You are applying VERIFIED low-severity audit fixes to ONE file of svelte-check-native (a Rust reimplementation of svelte-check/svelte2tsx). You own this file exclusively; no other agent edits it.

FILE: ${file}

FINDINGS TO FIX (each already adversarially confirmed real + in-scope):
${findings.map((f) => `- ${f.id} @ ${f.location} [risk:${f.risk}]\n  FIX: ${f.fix}`).join('\n')}

PROCESS:
1. Read the file (and the verdict MD at notes/audit/active/verdicts/<id>.md if you need the full evidence/rationale).
2. For each finding, locate the cited code and apply EXACTLY the documented FIX — nothing more.
3. Comment/doc fixes: reword the comment as described; keep prose self-explanatory (do NOT reference internal round numbers, ticket/finding IDs, or bug numbers in the comment text — teach the reader directly). Brittle upstream "file:LINE" pins may be reduced to "file" if the FIX says so.
4. Dead-code fixes: delete exactly the named line(s)/field(s). Keep any explanatory comment the FIX says to keep.
5. Small behavioral/code fixes: apply precisely as specified.

HARD RULES:
- Apply ONLY what the FIX documents. Do not refactor, rename, or "improve" beyond it.
- If the cited code does NOT match the finding (already fixed by an earlier change, or stale), SKIP that finding and record it in "skipped" — do not invent a change.
- If a fix would need a design fixture, public-API change, or judgement beyond the documented FIX, SKIP it and note why.
- Keep the file compiling: your edit must be valid Rust. Re-read your edited region to confirm balanced braces and correct syntax. Do NOT run cargo (the orchestrator gates centrally).
- Match surrounding code style exactly.
- Set "behavioral": true if ANY applied change alters what the emit/lint stage OUTPUTS (so the orchestrator knows to re-lock snapshots). Pure comment/doc/dead-code edits are behavioral:false.

Return the JSON summary.`;

const results = await parallel(
  FILES.map(([file, findings]) => () => agent(prompt(file, findings), { label: `fix:${file.split('/').pop()}`, phase: 'Fix', schema: SCHEMA }))
);
const ok = results.filter(Boolean);
const applied = ok.reduce((a, r) => a + (r.applied?.length || 0), 0);
const skipped = ok.reduce((a, r) => a + (r.skipped?.length || 0), 0);
const behavioralFiles = ok.filter((r) => r.behavioral).map((r) => r.file);
log(`Done: ${applied} applied, ${skipped} skipped across ${ok.length} files. Behavioral: ${behavioralFiles.length} files.`);
return { applied, skipped, files: ok.length, behavioralFiles, perFile: ok };
