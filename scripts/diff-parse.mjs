#!/usr/bin/env node
// diff-parse.mjs — diff the real `svelte/compiler` `parse()` (modern AST)
// against `crates/parser` for a .svelte file or a directory tree.
//
// The parse-layer counterpart of diff-emit.mjs: our parser is otherwise
// only tested against fixtures, never against the real Svelte parser on
// real code, so structural mis-parses can ship silently. This tool
// normalizes both parses to a comparable skeleton (kind/name/span tree)
// and reports every disagreement.
//
// Usage:
//   node scripts/diff-parse.mjs <path/to/File.svelte>
//       Side-by-side skeleton diff plus per-node divergence lines.
//
//   node scripts/diff-parse.mjs --dir <path>
//       Sweep every .svelte under <path> (node_modules skipped).
//       Summary: N files, N identical, N divergent (one line per file),
//       N upstream-parse-rejected, N allowlisted.
//
// Options:
//   --svelte <dir>     Directory whose node_modules provides `svelte`
//                      (also SVELTE_DIR env). Defaults to the newest
//                      svelte found in the target's workspace, then any
//                      bench/* install.
//   --dump-bin <path>  Path to the dump_parse example binary. Default:
//                      target/{release,debug}/examples/dump_parse.
//                      Build: cargo build --release -p svn-parser --example dump_parse
//   --allow <path>     Allowlist JSON (default scripts/diff-parse-allow.json).
//   --no-allow         Ignore the allowlist (show everything).
//   --max-lines <n>    Divergence lines printed per file (default 20).
//
// What is compared (the common structural core — NOT full trees):
//   - Section layout: module/instance script spans, style span,
//     svelte:options span (upstream hoists options out of the fragment;
//     we re-hoist ours to match).
//   - Elements/components/svelte:* by tag name + span. Upstream's
//     RegularElement/Component/TitleElement/SlotElement/Svelte* kinds
//     all normalize to their tag name.
//   - Blocks: kind (if/each/await/key/snippet) + span; each-block
//     context/index/key presence; await pending/then/catch/value/error
//     presence. Upstream nests `{:else if}` as an alternate holding an
//     `elseif: true` IfBlock; both sides flatten branch bodies into one
//     child list so the chain shape itself isn't compared.
//   - Tags: expression/html/render/const/debug/declaration spans.
//   - Comments and text runs: span-only.
//   - Attributes: kind + name + span. Directive prefixes normalize
//     (in:/out:/transition: all → transition, matching upstream's
//     single TransitionDirective). Attribute VALUES are not compared
//     (shape-divergent by design; the attribute span is the signal).
//     In-tag JS comments are dropped (upstream drops them from the AST).
//   - The whole-file TS-mode flag: upstream derives it from the first
//     lang= regex match over the source; we resolve per script section.
//     Disagreement emits a `ts-mode-flag:` line (allowlisted — decided
//     divergence, see scripts/diff-parse-allow.json).
//
// Spans are byte offsets on both sides and must agree exactly — that is
// the core signal. Expression/text CONTENTS are never compared.
//
// There is deliberately no --update-allow: allowlist entries are
// hand-written with reasons, same philosophy as bench/.parity-exceptions.json.
// A sweep warns about entries that no longer match anything.

import { readFileSync, existsSync, readdirSync, statSync, writeFileSync, mkdtempSync, rmSync } from 'node:fs';
import { resolve, dirname, join, relative } from 'node:path';
import { spawnSync } from 'node:child_process';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';

const args = process.argv.slice(2);
if (args.length === 0 || args.includes('--help') || args.includes('-h')) {
    console.error(
        'Usage: diff-parse.mjs <File.svelte> | --dir <path> [--svelte DIR] [--dump-bin PATH] [--allow PATH] [--no-allow] [--max-lines N]',
    );
    process.exit(args.length === 0 ? 2 : 0);
}

function flag(name) {
    return args.includes(name);
}
function value(name) {
    const i = args.indexOf(name);
    return i === -1 ? null : args[i + 1];
}

const repoRoot = resolve(dirname(new URL(import.meta.url).pathname), '..');
const dirMode = flag('--dir');
const target = resolve(dirMode ? value('--dir') : args[0]);
if (!existsSync(target)) {
    console.error(`Not found: ${target}`);
    process.exit(2);
}
const maxLines = parseInt(value('--max-lines') ?? '20', 10);

// ---- svelte/compiler resolution -----------------------------------------

function svelteVersionAt(dir) {
    const pkg = join(dir, 'node_modules', 'svelte', 'package.json');
    if (!existsSync(pkg)) return null;
    try {
        return JSON.parse(readFileSync(pkg, 'utf8')).version ?? null;
    } catch {
        return null;
    }
}

function cmpSemver(a, b) {
    const p = (v) => String(v).split('-')[0].split('.').map((n) => parseInt(n, 10) || 0);
    const [av, bv] = [p(a), p(b)];
    for (let i = 0; i < 3; i++) if (av[i] !== bv[i]) return av[i] - bv[i];
    return 0;
}

function findSvelteBase() {
    const explicit = value('--svelte') ?? process.env.SVELTE_DIR ?? null;
    if (explicit) {
        const dir = resolve(explicit);
        if (!svelteVersionAt(dir)) {
            console.error(`No node_modules/svelte under ${dir}`);
            process.exit(2);
        }
        return dir;
    }
    const candidates = [];
    // Walk up from the target for a workspace install.
    let dir = dirMode ? target : dirname(target);
    while (dir.length > 1) {
        if (svelteVersionAt(dir)) candidates.push(dir);
        dir = dirname(dir);
    }
    // Any bench workspace (newest svelte wins — the reference parser
    // should be the most current grammar available).
    const bench = join(repoRoot, 'bench');
    if (existsSync(bench)) {
        for (const entry of readdirSync(bench)) {
            const d = join(bench, entry);
            if (svelteVersionAt(d)) candidates.push(d);
        }
    }
    if (candidates.length === 0) {
        console.error('No svelte install found (target workspace or bench/*). Pass --svelte <dir>.');
        process.exit(2);
    }
    candidates.sort((a, b) => cmpSemver(svelteVersionAt(b), svelteVersionAt(a)));
    return candidates[0];
}

const svelteBase = findSvelteBase();
const req = createRequire(join(svelteBase, 'noop.js'));
const compiler = req('svelte/compiler');
console.error(`reference: svelte ${compiler.VERSION} (from ${svelteBase})`);

// ---- our dump binary ----------------------------------------------------

function findDumpBin() {
    const explicit = value('--dump-bin');
    if (explicit) return resolve(explicit);
    for (const profile of ['release', 'debug']) {
        const p = join(repoRoot, 'target', profile, 'examples', 'dump_parse');
        if (existsSync(p)) return p;
    }
    console.error(
        'dump_parse binary missing. Build it first:\n  cargo build --release -p svn-parser --example dump_parse',
    );
    process.exit(2);
}
const dumpBin = findDumpBin();

/** Run dump_parse over files (batched), return Map<file, dumpObject>. */
function runDump(files) {
    const out = new Map();
    const BATCH = 400;
    for (let i = 0; i < files.length; i += BATCH) {
        const batch = files.slice(i, i + BATCH);
        const r = spawnSync(dumpBin, batch, { encoding: 'utf8', maxBuffer: 1024 * 1024 * 1024 });
        if (r.status !== 0 && !r.stdout) {
            console.error(`dump_parse failed: ${r.stderr}`);
            process.exit(2);
        }
        for (const line of r.stdout.split('\n')) {
            if (!line.trim()) continue;
            let obj;
            try {
                obj = JSON.parse(line);
            } catch {
                console.error(`unparseable dump line: ${line.slice(0, 200)}`);
                continue;
            }
            out.set(obj.file, obj);
        }
    }
    return out;
}

// ---- upstream TS-mode flag (whole-file regex, parse/index.js) -----------

// Mirrors svelte/compiler phases/1-parse/index.js `regex_lang_attribute`
// + the `<s` filter loop + `match[2] === 'ts'`.
const regexLangAttribute =
    /<!--[^]*?-->|<script\s+(?:[^>]*|(?:[^=>'"/]+=(?:"[^"]*"|'[^']*'|[^>\s]+)\s+)*)lang=(["'])?([^"' >]+)\1[^>]*>/g;

function upstreamTsFlag(source) {
    regexLangAttribute.lastIndex = 0;
    let m;
    do m = regexLangAttribute.exec(source);
    while (m && m[0][1] !== 's');
    regexLangAttribute.lastIndex = 0;
    return m?.[2] === 'ts';
}

// ---- normalization: common skeleton -------------------------------------
//
// Node: { k, name?, tag?, s, e, flags?, attrs?, children? }
//
// Upstream spans are UTF-16 code-unit indices (JS string offsets); our
// spans are UTF-8 byte offsets. TO_BYTE (set per file in normUpstream)
// converts upstream's indices so both sides compare in bytes.

let TO_BYTE = (i) => i;

function makeU16ToByte(source) {
    if (!/[^\x00-\x7F]/.test(source)) return (i) => i;
    const map = new Uint32Array(source.length + 1);
    let byte = 0;
    let u = 0;
    for (const ch of source) {
        // for..of iterates code points; ch.length is 1 or 2 UTF-16 units.
        const cp = ch.codePointAt(0);
        const bytes = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
        for (let k = 0; k < ch.length; k++) map[u + k] = byte;
        u += ch.length;
        byte += bytes;
    }
    map[u] = byte;
    return (i) => (i >= 0 && i <= u ? map[i] : byte);
}

const TAG_KINDS = {
    ExpressionTag: 'expression',
    HtmlTag: 'at_html',
    ConstTag: 'at_const',
    DebugTag: 'at_debug',
    RenderTag: 'at_render',
};

const DIRECTIVE_KINDS = {
    BindDirective: 'bind',
    OnDirective: 'on',
    UseDirective: 'use',
    ClassDirective: 'class',
    StyleDirective: 'style',
    TransitionDirective: 'transition',
    AnimateDirective: 'animate',
    LetDirective: 'let',
};

function flattenUpstreamIf(n) {
    // `{:else if}` nests as alternate=[IfBlock{elseif:true}]; flatten the
    // chain's branch bodies into one list (matching our flattened arms).
    const children = [...n.consequent.nodes];
    if (n.alternate) {
        const alt = n.alternate.nodes;
        if (alt.length === 1 && alt[0].type === 'IfBlock' && alt[0].elseif) {
            children.push(...flattenUpstreamIf(alt[0]));
        } else {
            children.push(...alt);
        }
    }
    return children;
}

function normUpstreamAttrs(attrs) {
    const out = [];
    for (const a of attrs ?? []) {
        const s = TO_BYTE(a.start);
        const e = TO_BYTE(a.end);
        if (a.type === 'Attribute') {
            out.push({ k: 'attribute', name: a.name, s, e });
        } else if (a.type === 'SpreadAttribute') {
            out.push({ k: 'spread', s, e });
        } else if (a.type === 'AttachTag') {
            out.push({ k: 'attach', s, e });
        } else if (a.type in DIRECTIVE_KINDS) {
            out.push({ k: 'directive', tag: DIRECTIVE_KINDS[a.type], name: a.name, s, e });
        } else {
            out.push({ k: `?${a.type}`, s, e });
        }
    }
    return out;
}

function normUpstreamNode(n) {
    const base = { s: TO_BYTE(n.start), e: TO_BYTE(n.end) };
    switch (n.type) {
        case 'Text':
            return { k: 'text', ...base };
        case 'Comment':
            return { k: 'comment', ...base };
        case 'ExpressionTag':
        case 'HtmlTag':
        case 'ConstTag':
        case 'DebugTag':
        case 'RenderTag':
            return { k: 'tag', tag: TAG_KINDS[n.type], ...base };
        case 'AttachTag':
            return { k: 'tag', tag: 'at_attach', ...base };
        case 'DeclarationTag':
            return { k: 'tag', tag: n.declaration?.kind === 'let' ? 'decl_let' : 'decl_const', ...base };
        case 'IfBlock':
            return { k: 'if', ...base, children: flattenUpstreamIf(n).map(normUpstreamNode) };
        case 'EachBlock':
            return {
                k: 'each',
                ...base,
                flags: `ctx=${!!n.context} idx=${n.index != null} key=${!!n.key}`,
                children: [...n.body.nodes, ...(n.fallback?.nodes ?? [])].map(normUpstreamNode),
            };
        case 'AwaitBlock':
            return {
                k: 'await',
                ...base,
                flags:
                    `pending=${!!n.pending} then=${!!n.then} catch=${!!n.catch} ` +
                    `value=${!!n.value} error=${!!n.error}`,
                children: [
                    ...(n.pending?.nodes ?? []),
                    ...(n.then?.nodes ?? []),
                    ...(n.catch?.nodes ?? []),
                ].map(normUpstreamNode),
            };
        case 'KeyBlock':
            return { k: 'key', ...base, children: n.fragment.nodes.map(normUpstreamNode) };
        case 'SnippetBlock':
            return {
                k: 'snippet',
                name: n.expression?.name,
                ...base,
                children: n.body.nodes.map(normUpstreamNode),
            };
        default:
            // Element-likes all carry .name (RegularElement, Component,
            // TitleElement, SlotElement, SvelteElement, SvelteComponent,
            // SvelteSelf, SvelteWindow, SvelteDocument, SvelteBody,
            // SvelteHead, SvelteFragment, SvelteBoundary).
            if (n.fragment && typeof n.name === 'string') {
                return {
                    k: 'el',
                    name: n.name,
                    ...base,
                    attrs: normUpstreamAttrs(n.attributes),
                    children: n.fragment.nodes.map(normUpstreamNode),
                };
            }
            return { k: `?${n.type}`, ...base };
    }
}

function normUpstream(source) {
    let ast;
    try {
        ast = compiler.parse(source, { modern: true });
    } catch (err) {
        return { reject: { code: err?.code ?? 'unknown', message: String(err?.message ?? err).split('\n')[0] } };
    }
    // Upstream parse() strips a leading BOM (compiler/index.js
    // remove_bom), so its offsets are relative to the post-BOM string.
    // Rebase them onto original-file byte offsets by mapping over the
    // BOM-less source and adding the BOM's 3 bytes back.
    const bom = source.charCodeAt(0) === 0xfeff ? 3 : 0;
    const map = makeU16ToByte(bom ? source.slice(1) : source);
    TO_BYTE = bom ? (i) => map(i) + bom : map;
    const span = (x) => (x ? { s: TO_BYTE(x.start), e: TO_BYTE(x.end) } : null);
    return {
        sections: {
            module: span(ast.module),
            instance: span(ast.instance),
            style: span(ast.css),
            options: span(ast.options),
        },
        ts: upstreamTsFlag(source),
        nodes: ast.fragment.nodes.map(normUpstreamNode),
    };
}

// ---- normalization: our dump --------------------------------------------

const OUR_TAG_KINDS = {
    expression: 'expression',
    at_const: 'at_const',
    decl_const: 'decl_const',
    decl_let: 'decl_let',
    at_html: 'at_html',
    at_render: 'at_render',
    at_debug: 'at_debug',
    at_tag: 'at_attach',
};

function normOurAttrs(attrs) {
    const out = [];
    for (const a of attrs ?? []) {
        if (a.kind === 'comment') continue; // upstream drops in-tag comments
        if (a.kind === 'directive') {
            const dir = a.dir === 'in' || a.dir === 'out' ? 'transition' : a.dir;
            out.push({ k: 'directive', tag: dir, name: a.name, s: a.start, e: a.end });
        } else {
            out.push({ k: a.kind, name: a.name, s: a.start, e: a.end });
        }
    }
    return out;
}

function normOurNode(n) {
    const base = { s: n.start, e: n.end };
    switch (n.kind) {
        case 'text':
            return { k: 'text', ...base };
        case 'comment':
            return { k: 'comment', ...base };
        case 'interpolation':
            return { k: 'tag', tag: OUR_TAG_KINDS[n.tag] ?? `?${n.tag}`, ...base };
        case 'element':
        case 'component':
        case 'svelte_element': {
            // Upstream extracts `this` off <svelte:component>/<svelte:element>
            // into node.expression / node.tag; it never appears in their
            // attribute list. We keep it as a regular attribute — drop it
            // for comparison.
            let attrs = n.attrs;
            if (n.name === 'svelte:component' || n.name === 'svelte:element') {
                attrs = attrs.filter((a) => !(a.kind === 'attribute' && a.name === 'this'));
            }
            return {
                k: 'el',
                name: n.name,
                ...base,
                attrs: normOurAttrs(attrs),
                children: n.children.map(normOurNode),
            };
        }
        case 'if':
            return {
                k: 'if',
                ...base,
                children: [
                    ...n.consequent,
                    ...n.elseif.flat(),
                    ...(n.alternate ?? []),
                ].map(normOurNode),
            };
        case 'each':
            return {
                k: 'each',
                ...base,
                flags: `ctx=${n.has_context} idx=${n.has_index} key=${n.has_key}`,
                children: [...n.body, ...(n.alternate ?? [])].map(normOurNode),
            };
        case 'await':
            return {
                k: 'await',
                ...base,
                flags:
                    `pending=${n.pending != null} then=${n.then != null} catch=${n.catch != null} ` +
                    `value=${n.then?.has_context ?? false} error=${n.catch?.has_context ?? false}`,
                children: [
                    ...(n.pending ?? []),
                    ...(n.then?.body ?? []),
                    ...(n.catch?.body ?? []),
                ].map(normOurNode),
            };
        case 'key':
            return { k: 'key', ...base, children: n.body.map(normOurNode) };
        case 'snippet':
            return { k: 'snippet', name: n.name, ...base, children: n.body.map(normOurNode) };
        default:
            return { k: `?${n.kind}`, ...base };
    }
}

function normOurs(dump, source) {
    // Hoist <svelte:options> out of the template (upstream removes it
    // from the fragment and stores it on the root).
    let options = null;
    let nodes = [];
    for (const n of dump.template) {
        if (n.kind === 'svelte_element' && n.name === 'svelte:options' && options === null) {
            options = { s: n.start, e: n.end };
            continue;
        }
        nodes.push(n);
    }
    // Upstream strips a leading BOM and runs `template.trimEnd()` BEFORE
    // parsing (compiler/index.js remove_bom + parse/index.js), so neither
    // a BOM nor trailing whitespace at EOF ever reaches its fragment.
    // Mirror both on our top-level list: clamp/drop leading text inside
    // the BOM and trailing text past the trimmed boundary. Boundaries are
    // computed in BYTES (our span unit); JS trimEnd matches upstream's
    // whitespace set exactly because upstream trims the same JS string.
    const bomLen = source.charCodeAt(0) === 0xfeff ? 3 : 0;
    if (bomLen && nodes.length > 0) {
        const first = nodes[0];
        if (first.kind === 'text' && first.end <= bomLen) nodes.shift();
        else if (first.kind === 'text' && first.start < bomLen) {
            nodes[0] = { ...first, start: bomLen };
        }
    }
    const trimmedLen = Buffer.byteLength(source.trimEnd(), 'utf8');
    while (nodes.length > 0) {
        const last = nodes[nodes.length - 1];
        if (last.kind === 'text' && last.start >= trimmedLen) {
            nodes.pop();
            continue;
        }
        if (last.kind === 'text' && last.end > trimmedLen) {
            nodes = nodes.slice(0, -1).concat([{ ...last, end: trimmedLen }]);
        }
        break;
    }
    const span = (x) => (x ? { s: x.start, e: x.end } : null);
    const ts = (dump.instance_script ?? dump.module_script)?.lang === 'ts';
    return {
        sections: {
            module: span(dump.module_script),
            instance: span(dump.instance_script),
            style: span(dump.style),
            options,
        },
        ts,
        errors: dump.errors,
        nodes: nodes.map(normOurNode),
    };
}

// ---- comparison ---------------------------------------------------------

function sig(n) {
    const label = n.k + (n.name ? `(${n.name})` : '') + (n.tag ? `<${n.tag}>` : '');
    const flags = n.flags ? ` [${n.flags}]` : '';
    return `${label}@${n.s}-${n.e}${flags}`;
}

function compareLists(path, ours, ups, out) {
    let i = 0;
    let j = 0;
    while (i < ours.length && j < ups.length) {
        const a = ours[i];
        const b = ups[j];
        if (a.s === b.s) {
            if (sig(a) !== sig(b)) {
                out.push(`${path}: ours=${sig(a)} upstream=${sig(b)}`);
            }
            if (a.k === b.k) {
                if (a.attrs || b.attrs) compareLists(`${path}/${sig(a)}:attrs`, a.attrs ?? [], b.attrs ?? [], out);
                if (a.children || b.children)
                    compareLists(`${path}/${sig(a)}`, a.children ?? [], b.children ?? [], out);
            }
            i++;
            j++;
        } else if (a.s < b.s) {
            out.push(`${path}: ours-only ${sig(a)}`);
            i++;
        } else {
            out.push(`${path}: upstream-only ${sig(b)}`);
            j++;
        }
    }
    for (; i < ours.length; i++) out.push(`${path}: ours-only ${sig(ours[i])}`);
    for (; j < ups.length; j++) out.push(`${path}: upstream-only ${sig(ups[j])}`);
}

function compareFile(ours, ups) {
    const lines = [];
    for (const key of ['module', 'instance', 'style', 'options']) {
        const a = ours.sections[key];
        const b = ups.sections[key];
        const fmt = (x) => (x ? `${x.s}-${x.e}` : 'absent');
        if (fmt(a) !== fmt(b)) lines.push(`section ${key}: ours=${fmt(a)} upstream=${fmt(b)}`);
    }
    if (ours.ts !== ups.ts) lines.push(`ts-mode-flag: ours=${ours.ts} upstream=${ups.ts}`);
    for (const err of ours.errors) {
        lines.push(`ours-parse-error: ${err.code}@${err.start}-${err.end}`);
    }
    compareLists('/', ours.nodes, ups.nodes, lines);
    return lines;
}

// ---- allowlist ----------------------------------------------------------

const allowPath = value('--allow') ?? join(repoRoot, 'scripts', 'diff-parse-allow.json');
let allowEntries = [];
if (!flag('--no-allow') && existsSync(allowPath)) {
    const raw = JSON.parse(readFileSync(allowPath, 'utf8'));
    allowEntries = (raw.allow ?? []).map((entry) => ({
        pattern: new RegExp(entry.pattern),
        files: entry.files ? new RegExp(entry.files) : null,
        reason: entry.reason ?? '',
        hits: 0,
    }));
}

function allowMatch(relFile, line) {
    for (const entry of allowEntries) {
        if (entry.files && !entry.files.test(relFile)) continue;
        if (entry.pattern.test(line)) {
            entry.hits++;
            return true;
        }
    }
    return false;
}

// ---- tree serialization (single-file mode) ------------------------------

function writeTree(nodes, indent, out) {
    for (const n of nodes) {
        out.push('  '.repeat(indent) + sig(n));
        if (n.attrs) for (const a of n.attrs) out.push('  '.repeat(indent + 1) + '@' + sig(a));
        if (n.children) writeTree(n.children, indent + 1, out);
    }
}

function serialize(norm) {
    const out = [];
    for (const key of ['module', 'instance', 'style', 'options']) {
        const s = norm.sections[key];
        out.push(`section ${key}: ${s ? `${s.s}-${s.e}` : 'absent'}`);
    }
    out.push(`ts-mode: ${norm.ts}`);
    writeTree(norm.nodes, 0, out);
    return out.join('\n') + '\n';
}

// ---- modes --------------------------------------------------------------

function collectSvelteFiles(root, out) {
    for (const entry of readdirSync(root)) {
        // Keep dot-dirs like .svelte-kit — generated .svelte files are
        // real checker inputs. Only dependency trees and VCS are skipped.
        if (entry === 'node_modules' || entry === '.git') continue;
        const p = join(root, entry);
        const st = statSync(p, { throwIfNoEntry: false });
        if (!st) continue;
        if (st.isDirectory()) collectSvelteFiles(p, out);
        else if (entry.endsWith('.svelte')) out.push(p);
    }
    return out;
}

if (!dirMode) {
    const source = readFileSync(target, 'utf8');
    const ups = normUpstream(source);
    const dump = runDump([target]).get(target);
    if (!dump || dump.read_error) {
        console.error(`dump_parse could not read ${target}: ${dump?.read_error ?? 'no output'}`);
        process.exit(2);
    }
    const ours = normOurs(dump, source);
    if (ups.reject) {
        console.log(`upstream-parse-reject: ${ups.reject.code} — ${ups.reject.message}`);
        console.log(`ours: ${ours.errors.length ? JSON.stringify(ours.errors) : 'parsed without fatal errors'}`);
        process.exit(1);
    }
    const lines = compareFile(ours, ups);
    const tmp = mkdtempSync(join(tmpdir(), 'svn-diff-parse-'));
    const uFile = join(tmp, 'upstream.txt');
    const oFile = join(tmp, 'ours.txt');
    writeFileSync(uFile, serialize(ups));
    writeFileSync(oFile, serialize(ours));
    spawnSync('diff', ['-u', '--color=always', uFile, oFile], {
        stdio: ['ignore', 'inherit', 'inherit'],
    });
    rmSync(tmp, { recursive: true, force: true });
    if (lines.length === 0) {
        console.log('IDENTICAL (normalized skeletons agree)');
        process.exit(0);
    }
    console.log(`\n${lines.length} divergence(s):`);
    const rel = relative(repoRoot, target);
    for (const line of lines) {
        console.log(`  ${allowMatch(rel, line) ? '[allowed] ' : ''}${line}`);
    }
    process.exit(lines.every((l) => allowMatch(rel, l)) ? 0 : 1);
}

// --dir sweep
const files = collectSvelteFiles(target, []).sort();
console.error(`sweeping ${files.length} .svelte files under ${target}`);
const dumps = runDump(files);

let identical = 0;
let divergent = 0;
let allowlisted = 0;
const allowlistedFiles = [];
let upstreamRejected = 0;
let bothRejected = 0;
const rejectBuckets = new Map();
const divergentFiles = [];

for (const file of files) {
    const rel = relative(repoRoot, file);
    let source;
    try {
        source = readFileSync(file, 'utf8');
    } catch {
        continue;
    }
    const dump = dumps.get(file);
    if (!dump || dump.read_error) {
        divergent++;
        divergentFiles.push({ rel, lines: [`dump failed: ${dump?.read_error ?? 'no output'}`] });
        continue;
    }
    const ups = normUpstream(source);
    const ours = normOurs(dump, source);
    if (ups.reject) {
        if (ours.errors.length > 0) bothRejected++;
        else upstreamRejected++;
        const key = ups.reject.code;
        rejectBuckets.set(key, (rejectBuckets.get(key) ?? []).concat(rel));
        continue;
    }
    const lines = compareFile(ours, ups);
    if (lines.length === 0) {
        identical++;
    } else if (lines.every((l) => allowMatch(rel, l))) {
        allowlisted++;
        allowlistedFiles.push({ rel, lines });
    } else {
        divergent++;
        divergentFiles.push({ rel, lines: lines.filter((l) => !allowMatch(rel, l)) });
    }
}

console.log(`\n=== diff-parse sweep: ${files.length} files ===`);
console.log(`identical:         ${identical}`);
console.log(`allowlisted:       ${allowlisted}`);
console.log(`divergent:         ${divergent}`);
console.log(
    `upstream-rejected: ${upstreamRejected} (ours tolerated; invalid embedded JS/TS or pre-${compiler.VERSION} syntax)`,
);
console.log(`both-rejected:     ${bothRejected}`);

if (allowlistedFiles.length > 0) {
    console.log('\nallowlisted files:');
    for (const { rel, lines } of allowlistedFiles) {
        console.log(`  ${rel}: ${lines[0]}${lines.length > 1 ? ` (+${lines.length - 1} more)` : ''}`);
    }
}

if (rejectBuckets.size > 0) {
    console.log('\nupstream reject buckets:');
    for (const [code, list] of [...rejectBuckets.entries()].sort((a, b) => b[1].length - a[1].length)) {
        console.log(`  ${code}: ${list.length}`);
        for (const f of list.slice(0, 5)) console.log(`    ${f}`);
        if (list.length > 5) console.log(`    ... +${list.length - 5} more`);
    }
}

if (divergentFiles.length > 0) {
    console.log('\ndivergent files:');
    for (const { rel, lines } of divergentFiles) {
        console.log(`  ${rel} (${lines.length}):`);
        for (const line of lines.slice(0, maxLines)) console.log(`    ${line}`);
        if (lines.length > maxLines) console.log(`    ... +${lines.length - maxLines} more`);
    }
}

for (const entry of allowEntries) {
    if (entry.hits === 0) {
        console.log(`\nWARNING: stale allowlist entry (matched nothing): ${entry.pattern}`);
    }
}

process.exit(divergent > 0 ? 1 : 0);
