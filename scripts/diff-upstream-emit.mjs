#!/usr/bin/env node
// diff-upstream-emit.mjs — compare our emit snapshots for upstream's
// svelte2tsx / htmlx2jsx samples against upstream's own expected output.
//
// `crates/cli/tests/emit_snapshots` locks our `--emit-ts` output against
// itself, so a wrong shape accepted once stays green. Upstream ships the
// expected output for the same inputs; this tool reads both and reports
// where the TYPED pieces differ. The two files are laid out differently
// (render-function wrapper, helper names, indentation), so whole files
// are not compared. Instead each side is reduced to a list of constructs
// that decide what TypeScript checks, paired in source order:
//
//   element   `svelteHTML.createElement("tag", {…})` — tag + attribute object
//   component `new C({ target, props: {…} })` — the props object
//   iso       the generic `$$IsomorphicComponent` interface body
//   each      the `for (… of …)` header of `{#each}`
//   if        the condition of `{#if}` / `{:else if}`
//   await     the awaited expression of `{#await}`
//   snippet   a snippet's name and parameters (compared as a set)
//   render    the argument of `{@render}`
//   on        the event name and handler of a component's `$on(…)`
//
// Not compared: the render function's result and the non-generic default
// export. We fold svelte2tsx's factory wrappers (`with_any_event`,
// `partial`) into the render result instead of applying them around it,
// so the two only agree at the level of the resulting types, which a
// text comparison cannot see.
//
// Also not compared, because the two sides write them in shapes a text
// comparison cannot pair: `bind:` assignments and `$$bindings`, actions,
// transitions and animations, `{@const}`, `{@html}` and `{#key}`, bare
// `{expression}` statements, slot definitions, and which stretches are
// wrapped in ignore comments (all comments are dropped). A difference
// there shows only in the diagnostics, which the fixtures check.
//
// Each construct is normalised before comparing: comments and whitespace
// outside strings removed, trailing commas dropped, helper names mapped
// to one spelling (`__sveltets_2_ensureComponent` and
// `__svn_ensure_component` both become `H.ensurecomponent`), and
// synthesised locals renamed (`$$_tnenopmoC0C` / `__svn_C_0` → `$C`).
//
// The upstream side is `upstream.emit.ts`, written next to each snapshot
// by scripts/gen-upstream-emit.mjs: svelte2tsx at the submodule's version,
// called as `svelte-check --tsgo` calls it. Samples svelte2tsx rejects
// (`upstream.error.txt`) are skipped: svelte-check reports the parse
// error and type-checks nothing.
//
// Usage:
//   node scripts/diff-upstream-emit.mjs              report, exit 1 on any
//                                                   divergence not allowed
//   node scripts/diff-upstream-emit.mjs --all        ignore the allowlist
//   node scripts/diff-upstream-emit.mjs --sample <corpus/name>
//                                                   one sample, verbose
//   node scripts/diff-upstream-emit.mjs --kinds      count divergences per kind
//   node scripts/diff-upstream-emit.mjs --json       machine-readable list
//
// Allowlist: scripts/diff-upstream-emit-allow.json — hand-written entries
// with reasons. Each entry matches by `sample` (exact, or `*`), `kind`,
// and optional `ours` / `upstream` substrings of the normalised text.
// Entries that match nothing are reported as stale (exit 1), so the list
// only ever shrinks toward real decisions.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const SNAP = path.join(ROOT, 'crates/cli/tests/emit_snapshots');
const ALLOW = path.join(ROOT, 'scripts/diff-upstream-emit-allow.json');
const CORPORA = { svelte2tsx: 'ts', htmlx2jsx: 'js', bugs: 'ts' };

const args = process.argv.slice(2);
const flag = (f) => args.includes(f);
const opt = (f) => {
    const i = args.indexOf(f);
    return i >= 0 ? args[i + 1] : undefined;
};

// ---------------------------------------------------------------- scanning

/** Index just past the string/template/comment starting at `i`, or -1. */
function skipNonCode(s, i) {
    const c = s[i];
    if (c === '/' && s[i + 1] === '/') {
        const e = s.indexOf('\n', i);
        return e < 0 ? s.length : e;
    }
    if (c === '/' && s[i + 1] === '*') {
        const e = s.indexOf('*/', i + 2);
        return e < 0 ? s.length : e + 2;
    }
    if (c === '"' || c === "'") {
        let j = i + 1;
        while (j < s.length && s[j] !== c) j += s[j] === '\\' ? 2 : 1;
        return j + 1;
    }
    if (c === '`') {
        let j = i + 1;
        while (j < s.length && s[j] !== '`') {
            if (s[j] === '\\') j += 2;
            else if (s[j] === '$' && s[j + 1] === '{') j = matchClose(s, j + 1) + 1;
            else j++;
        }
        return j + 1;
    }
    return -1;
}

/** Index of the bracket closing the one at `open`. */
function matchClose(s, open) {
    const pairs = { '(': ')', '[': ']', '{': '}' };
    const stack = [pairs[s[open]]];
    let i = open + 1;
    while (i < s.length && stack.length) {
        const skip = skipNonCode(s, i);
        if (skip >= 0) {
            i = skip;
            continue;
        }
        const c = s[i];
        if (pairs[c]) stack.push(pairs[c]);
        else if (c === stack[stack.length - 1]) stack.pop();
        i++;
    }
    return i - 1;
}

/** Split `s` on top-level commas. */
function splitTop(s) {
    const out = [];
    let depth = 0;
    let last = 0;
    let i = 0;
    while (i < s.length) {
        const skip = skipNonCode(s, i);
        if (skip >= 0) {
            i = skip;
            continue;
        }
        const c = s[i];
        if ('([{'.includes(c)) depth++;
        else if (')]}'.includes(c)) depth--;
        else if (c === ',' && depth === 0) {
            out.push(s.slice(last, i));
            last = i + 1;
        }
        i++;
    }
    out.push(s.slice(last));
    return out;
}

/** Value text of top-level key `key` in object literal text `{…}`. */
function objectKey(obj, key) {
    const inner = obj.trim().replace(/^\{/, '').replace(/\}$/, '');
    for (const part of splitTop(inner)) {
        const m = part.match(/^\s*([\w$]+)\s*:/);
        if (m && m[1] === key) return part.slice(m[0].length);
    }
    return undefined;
}

// ----------------------------------------------------------- normalising

// Helper names that differ beyond spelling: our name → upstream's.
const HELPER_ALIASES = {
    eachitems: 'ensurearray',
};

function canonHelper(name) {
    const c = name.toLowerCase().replace(/_/g, '');
    return 'H.' + (HELPER_ALIASES[c] ?? c);
}

/**
 * One spelling for a string literal whose value is fixed: `"a"`, `'a'`
 * and the substitution-free template `` `a` `` all have the literal type
 * `"a"`. Templates with `${…}` are kept as they are.
 */
function canonString(lit) {
    const q = lit[0];
    if (q === '`' && /(^|[^\\])\$\{/.test(lit)) return lit;
    const body = lit.slice(1, -1);
    let value;
    try {
        value = q === '`' ? body.replace(/\\`/g, '`') : JSON.parse('"' + body.replace(/\\'/g, "'").replace(/(^|[^\\])"/g, '$1\\"') + '"');
    } catch {
        return lit;
    }
    if (q === '`') {
        try {
            value = JSON.parse('"' + value.replace(/"/g, '\\"').replace(/\n/g, '\\n') + '"');
        } catch {
            return lit;
        }
    }
    return JSON.stringify(value);
}

/**
 * One name for each synthesised identifier. Runs before whitespace is
 * removed, so `a of b` cannot run together into one name.
 */
function renameSynthesised(text) {
    return (
        text
            .replace(/__sveltets_2_([A-Za-z0-9_]+)/g, (_, n) => canonHelper(n))
            .replace(/__svn_C_[A-Za-z0-9_]+/g, '$C')
            .replace(/__svn_inst_[A-Za-z0-9_]+/g, '$I')
            // svelte2tsx names a component's constructor local after the
            // reversed tag name plus its depth (`$$_tnenopmoC0C`) and the
            // instance the same without the trailing `C` (`$$_tnenopmoC0`).
            .replace(/\$\$_[A-Za-z0-9_]*?\d+C\b/g, '$C')
            .replace(/\$\$_[A-Za-z][A-Za-z0-9_]*?\d+\b/g, '$I')
            .replace(/__svn_Render_[0-9a-f]+|__sveltets_Render/g, '$R')
            .replace(/\$\$render_[0-9a-f]+/g, '$$$$render')
            // The placeholder item of an `{#each}` without `as`.
            .replace(/\$\$each_item|__svn_each_unused/g, '$EACH')
            // The iterable's temp when it shares the item's name.
            .replace(/\$\$_each\b|__svn_each_arr\b/g, '$EACHARR')
            .replace(/__svn_([A-Za-z0-9_]+)/g, (_, n) => canonHelper(n))
            .replace(/\$\$action_\d+/g, 'H.action')
            .replace(/H\.action\d+/g, 'H.action')
    );
}

function normalize(text) {
    text = renameSynthesised(text);
    // svelte2tsx ends a component's block with the bare tag name
    // (`… } Component}`), a reference for editor navigation with no
    // effect on types. Taken out while the white space is still there:
    // without it `return x }` reads as one name before the brace. A tag
    // name is capitalised, dotted or namespaced.
    text = text.replace(
        /([})]);?(\s*)(?:[A-Z][\w$.]*|[a-z_$][\w$]*[.:][\w$.:]*)\s*\}/g,
        '$1$2}'
    );
    let s = '';
    let i = 0;
    while (i < text.length) {
        const c = text[i];
        if (c === '/' && (text[i + 1] === '/' || text[i + 1] === '*')) {
            const end = skipNonCode(text, i);
            // A JSDoc cast `/** @type {T} */ (x)` types `x` as `T`; keep
            // the type as a marker so the cast can be compared below.
            const cast = /^\/\*\*\s*@type\s*\{([\s\S]*)\}\s*\*\/$/.exec(text.slice(i, end));
            if (cast) s += `@type{${normalize(cast[1])}}`;
            i = end;
            continue;
        }
        const skip = skipNonCode(text, i);
        if (skip >= 0) {
            s += canonString(text.slice(i, skip));
            i = skip;
            continue;
        }
        if (!/\s/.test(c)) s += c;
        i++;
    }
    return (
        s
            .replace(/,([}\])])/g, '$1')
            // A quoted key that is a plain identifier is the same key bare.
            .replace(/([{,])"([A-Za-z_$][\w$]*)":/g, '$1$2:')
            // Snippet and slot bodies: ours return `__svn_snippet_return()`,
            // upstream `__sveltets_2_any(0)`; both are `any`.
            .replace(/H\.snippetreturn\(\)/g, 'H.any(0)')
            .replace(/=>\{returnH\.any\(0\);?\}/g, '=>H.any(0)')
            .replace(/;\}/g, '}')
    );
}

/** `((x))` → `x`: parentheses around a whole expression. */
function stripOuterParens(s) {
    while (s.startsWith('(') && matchClose(s, 0) === s.length - 1) s = s.slice(1, -1);
    return s;
}

/**
 * Drop parentheses that wrap a whole property value in an object
 * literal (`{"x":(a?b:c)}` → `{"x":a?b:c}`), at every depth. We
 * parenthesise attribute values; upstream writes them bare. TypeScript
 * types both the same (contextual types pass through parentheses).
 */
function unwrapValues(s) {
    let out = '';
    let i = 0;
    while (i < s.length) {
        const skip = skipNonCode(s, i);
        if (skip >= 0) {
            out += s.slice(i, skip);
            i = skip;
            continue;
        }
        const c = s[i];
        // A value starts after `:` that follows a key; a parenthesised
        // value runs to its matching `)` and is followed by `,` or `}`.
        if (c === '(' && (out.endsWith(':') || out.endsWith('...'))) {
            const close = matchClose(s, i);
            const next = s[close + 1];
            // Not around a comma sequence: bare, `"x":a, b` gives the
            // object a second member, and parenthesised it is one value.
            const inner = s.slice(i + 1, close);
            const one_value = splitTop(stripOuterParens(inner)).length === 1;
            if (one_value && (next === ',' || next === '}' || next === undefined)) {
                out += unwrapValues(inner);
                i = close + 1;
                continue;
            }
        }
        out += c;
        i++;
    }
    return out;
}

// ------------------------------------------------------------ extracting

/** `text` with every comment blanked to spaces (same offsets). */
function blankComments(text) {
    let out = '';
    let i = 0;
    while (i < text.length) {
        const c = text[i];
        const skip = skipNonCode(text, i);
        if (skip >= 0) {
            const chunk = text.slice(i, skip);
            out += c === '/' ? chunk.replace(/[^\n]/g, ' ') : chunk;
            i = skip;
            continue;
        }
        out += c;
        i++;
    }
    return out;
}

const SNIPPET_RETURN = /^ReturnType<import\(['"]svelte['"]\)\.Snippet>/;
const SNIPPET_JSDOC = /^\/\*\*\s*@returns\s*\{ReturnType<import\(['"]svelte['"]\)\.Snippet>\}\s*\*\//;

/**
 * Every snippet declaration: `const name = <T>(params): ReturnType<
 * import('svelte').Snippet> => …` in a TypeScript file, `const name =
 * /** @returns {ReturnType<import('svelte').Snippet>} *\/ (params) => …`
 * in a JavaScript one. The construct is the type parameters and the
 * parameter list (with any JSDoc types in it), plus which of the two
 * return annotations it carries; the label is the snippet's name.
 * `text` is `original` with comments blanked, so offsets agree.
 */
function snippets(original, text) {
    const out = [];
    const decl = /\bconst\s+([\w$]+)\s*=/g;
    let m;
    while ((m = decl.exec(text))) {
        let p = m.index + m[0].length;
        // Real white space only: the JSDoc is blanked in `text`.
        while (/\s/.test(original[p] ?? '')) p++;
        const jsdoc = SNIPPET_JSDOC.exec(original.slice(p));
        if (jsdoc) {
            p += jsdoc[0].length;
            while (/\s/.test(text[p] ?? '')) p++;
        }
        const start = p;
        if (text[p] === '<') {
            // Type parameters: `<` … `>` at bracket depth zero, strings
            // skipped (`<T extends "<">`), `=>` not a closing angle.
            let depth = 0;
            while (p < text.length) {
                const skip = skipNonCode(text, p);
                if (skip >= 0) {
                    p = skip;
                    continue;
                }
                const c = text[p];
                if ('([{'.includes(c)) p = matchClose(text, p);
                else if (c === '<') depth++;
                else if (c === '>' && text[p - 1] !== '=') depth--;
                p++;
                if (depth === 0) break;
            }
            while (/\s/.test(text[p] ?? '')) p++;
        }
        if (text[p] !== '(') continue;
        const close = matchClose(text, p);
        let q = close + 1;
        while (/\s/.test(text[q] ?? '')) q++;
        let form = null;
        if (jsdoc && text.startsWith('=>', q)) form = 'jsdoc';
        else if (!jsdoc && text[q] === ':') {
            q++;
            while (/\s/.test(text[q] ?? '')) q++;
            const ret = SNIPPET_RETURN.exec(text.slice(q));
            if (ret) {
                q += ret[0].length;
                while (/\s/.test(text[q] ?? '')) q++;
                if (text.startsWith('=>', q)) form = 'annotation';
            }
        }
        if (!form) continue;
        out.push({
            kind: 'snippet',
            at: m.index,
            label: m[1],
            text: `${form}:${normalize(original.slice(start, close + 1))}`,
        });
    }
    return out;
}

function extract(original) {
    // Match on code only (a comment can mention `createElement(` too);
    // spans read from the original, comments included, since a JSDoc
    // cast in a value is part of it.
    const text = blankComments(original);
    const found = [];
    let m;
    const el = /svelteHTML\.createElement\(/g;
    while ((m = el.exec(text))) {
        // Our `bind:this` / one-way binding reads build a throwaway
        // element to read from (`x = createElement("div", {})`, `(() => {
        // const __svn_el = createElement("div", {}); … })()`); upstream
        // reads the element's own local (`x = $$_div0`). The element type
        // comes from the tag alone, so these are not elements to compare.
        const before = original.slice(Math.max(0, m.index - 60), m.index);
        if (/=\s*$/.test(before) && !/const\s+\$\$_[\w$]+\s*=\s*$/.test(before)) continue;
        const open = m.index + m[0].length - 1;
        const close = matchClose(text, open);
        const [tag, attrs = '', ...rest] = splitTop(original.slice(open + 1, close));
        found.push({
            kind: 'element',
            at: m.index,
            // The tag: a string, or for `<svelte:element>` an expression,
            // which we parenthesise and upstream writes bare.
            label: stripOuterParens(normalize(tag)),
            text: unwrapValues(normalize(attrs + (rest.length ? ',' + rest.join(',') : ''))),
        });
    }
    const comp = /\bnew\s+([\w$]+)\s*\(\s*\{/g;
    while ((m = comp.exec(text))) {
        const open = m.index + m[0].length - 1;
        const close = matchClose(text, open);
        const props = objectKey(original.slice(open, close + 1), 'props');
        if (props === undefined) continue;
        // The component expression handed to ensureComponent for this local.
        const local = m[1].replace(/[$]/g, '\\$');
        // The nearest one before the `new`: svelte2tsx reuses a local's
        // name for every `<svelte:component>`.
        const before = original.slice(0, m.index).split('\n').slice(-6).join('\n');
        const decls = new RegExp(
            `const\\s+${local}\\s*=\\s*(?:__sveltets_2_ensureComponent|__svn_ensure_component)\\(`,
            'g'
        );
        let decl = null;
        for (let d; (d = decls.exec(before)); ) decl = d;
        let label = '';
        if (decl) {
            const o = decl.index + decl[0].length - 1;
            label = normalize(before.slice(o + 1, matchClose(before, o)));
        }
        found.push({ kind: 'component', at: m.index, label, text: unwrapValues(normalize(props)) });
    }
    // `<svelte:self>`: an untyped instance, `createComponentAny({…})`.
    const anyComp = /(?:__sveltets_2_createComponentAny|__svn_create_component_any)\(\s*\{/g;
    while ((m = anyComp.exec(text))) {
        const open = m.index + m[0].length - 1;
        const close = matchClose(text, open);
        found.push({
            kind: 'component',
            at: m.index,
            label: 'H.createcomponentany',
            text: unwrapValues(normalize(original.slice(open, close + 1))),
        });
    }
    // Template control flow and snippets. Each records the text that
    // TypeScript checks: the `for (… of …)` header of `{#each}`, the
    // condition of `{#if}` / `{:else if}`, the awaited expression of
    // `{#await}`, a snippet's name and parameters, the argument of
    // `{@render}`.
    const parenContent = (re, kind, label = () => '') => {
        let mm;
        while ((mm = re.exec(text))) {
            const open = mm.index + mm[0].lastIndexOf('(');
            const close = matchClose(text, open);
            found.push({
                kind,
                at: mm.index,
                label: label(mm),
                text: stripOuterParens(normalize(original.slice(open + 1, close))),
            });
        }
    };
    parenContent(/\bfor\s*\(\s*(?:let|const|var)\s/g, 'each');
    parenContent(/\bif\s*\(/g, 'if');
    parenContent(/\bawait\s*\(/g, 'await');
    parenContent(/(?:__sveltets_2_ensureSnippet|__svn_ensure_snippet)\(/g, 'render');
    // `on:event` on a component: the event's name and its handler, which
    // `$on` types against the component's events.
    parenContent(/\.\$on\(/g, 'on');
    // We parenthesise the handler; an argument is one value either way.
    for (const c of found) {
        if (c.kind === 'on') c.text = splitTop(c.text).map(stripOuterParens).join(',');
    }
    for (const s of snippets(original, text)) found.push(s);
    const iso = /interface \$\$IsomorphicComponent\s*\{/g;
    while ((m = iso.exec(text))) {
        const open = m.index + m[0].length - 1;
        const close = matchClose(text, open);
        const body = original.slice(open + 1, close);
        // svelte2tsx writes the interface inline only for a generic
        // component (`new <T>(…)`); a non-generic one goes through the
        // shim's factory, so ours has nothing to compare against.
        if (!/^\s*new\s*</.test(body)) continue;
        // One construct per member, keyed by its leading token.
        for (const line of body.split('\n').map((l) => l.trim()).filter(Boolean)) {
            const key = line.startsWith('new')
                ? 'new'
                : line.startsWith('z_$$bindings')
                  ? 'z_$$bindings'
                  : 'call';
            found.push({ kind: 'iso', at: m.index, label: key, text: normalize(line) });
        }
    }
    found.sort((a, b) => a.at - b.at);
    return found;
}

// --------------------------------------------------------------- pairing

function compareSample(corpus, name) {
    const ours = path.join(SNAP, corpus, name, 'expected.emit.ts');
    const theirs = path.join(SNAP, corpus, name, 'upstream.emit.ts');
    if (!fs.existsSync(theirs) || !fs.existsSync(ours)) return null;
    const a = extract(fs.readFileSync(ours, 'utf8'));
    const b = extract(fs.readFileSync(theirs, 'utf8'));
    for (const c of b) stats.kinds[c.kind] = (stats.kinds[c.kind] ?? 0) + 1;
    if (a.length === 0 && b.length === 0) stats.empty.push(`${corpus}/${name}`);
    const out = [];
    for (const kind of [
        'element',
        'component',
        'iso',
        'each',
        'if',
        'await',
        'snippet',
        'render',
        'on',
    ]) {
        let xs = a.filter((c) => c.kind === kind);
        let ys = b.filter((c) => c.kind === kind);
        // Snippets are declarations; we hoist some to a different place
        // than svelte2tsx, so their order says nothing.
        if (kind === 'snippet') {
            const key = (c) => `${c.label}(${c.text})`;
            xs = [...xs].sort((p, q) => key(p).localeCompare(key(q)));
            ys = [...ys].sort((p, q) => key(p).localeCompare(key(q)));
        }
        // Upstream writes the interface inline only for generic
        // components; the others go through the shim's factory.
        if (kind === 'iso' && ys.length === 0) continue;
        const n = Math.max(xs.length, ys.length);
        for (let i = 0; i < n; i++) {
            const x = xs[i];
            const y = ys[i];
            // A component is the constructor it goes through plus its props.
            const same =
                x &&
                y &&
                x.text === y.text &&
                // An element is its tag, which decides the type its
                // attributes are checked against, plus those attributes.
                (!['component', 'snippet', 'element'].includes(kind) || x.label === y.label);
            if (same) continue;
            out.push({
                sample: `${corpus}/${name}`,
                kind,
                index: i,
                label: x && y && x.label !== y.label ? `${x.label} vs ${y.label}` : (x ?? y).label,
                ours: x ? x.text : null,
                upstream: y ? y.text : null,
            });
        }
    }
    return out;
}

// ------------------------------------------------------------- allowlist

function loadAllow() {
    if (flag('--all') || !fs.existsSync(ALLOW)) return [];
    return JSON.parse(fs.readFileSync(ALLOW, 'utf8')).entries.map((e) => ({ ...e, hits: 0 }));
}

function allowed(d, allow) {
    for (const e of allow) {
        if (e.sample !== '*' && e.sample !== d.sample) continue;
        if (e.kind && e.kind !== d.kind) continue;
        if (e.label !== undefined && e.label !== d.label) continue;
        if (e.ours !== undefined && !(d.ours ?? '').includes(e.ours)) continue;
        if (e.upstream !== undefined && !(d.upstream ?? '').includes(e.upstream)) continue;
        e.hits++;
        return e;
    }
    return null;
}

// ------------------------------------------------------------------ main

const only = opt('--sample');
const allow = loadAllow();
const divergences = [];
const knownBugs = [];
const missing = [];
let compared = 0;
// Constructs found on the upstream side, per kind, and the samples
// where neither side has any (for `--stats`).
const stats = { kinds: {}, empty: [] };
for (const corpus of Object.keys(CORPORA)) {
    const dir = path.join(SNAP, corpus);
    if (!fs.existsSync(dir)) continue;
    for (const name of fs.readdirSync(dir).sort()) {
        if (only && only !== `${corpus}/${name}`) continue;
        const sampleDir = path.join(dir, name);
        if (
            fs.existsSync(path.join(sampleDir, 'expected.emit.ts')) &&
            !fs.existsSync(path.join(sampleDir, 'upstream.emit.ts')) &&
            !fs.existsSync(path.join(sampleDir, 'upstream.error.txt'))
        ) {
            missing.push(`${corpus}/${name}`);
            continue;
        }
        const ds = compareSample(corpus, name);
        if (ds === null) continue;
        compared++;
        for (const d of ds) {
            const entry = allowed(d, allow);
            if (!entry) divergences.push(d);
            else if (entry.status === 'bug') knownBugs.push(d);
        }
    }
}

const stale = only ? [] : allow.filter((e) => e.hits === 0);

if (flag('--json')) {
    console.log(JSON.stringify({ compared, divergences, knownBugs, stale }, null, 2));
} else if (flag('--stats')) {
    console.log(`${compared} samples compared; upstream constructs per kind:`);
    for (const [k, n] of Object.entries(stats.kinds).sort()) console.log(`  ${k}: ${n}`);
    console.log(`${stats.empty.length} samples reduce to no construct on either side`);
    if (flag('--verbose')) for (const s of stats.empty) console.log(`  ${s}`);
} else if (flag('--kinds')) {
    const by = {};
    for (const d of divergences) by[d.kind] = (by[d.kind] ?? 0) + 1;
    console.log(`${compared} samples compared`);
    for (const [k, n] of Object.entries(by)) console.log(`  ${k}: ${n}`);
} else {
    const width = only ? Infinity : 240;
    const cut = (s) => (s === null ? '(missing)' : s.length > width ? s.slice(0, width) + '…' : s);
    for (const d of divergences) {
        console.log(`${d.sample}  ${d.kind}#${d.index} ${d.label}`);
        console.log(`  ours:     ${cut(d.ours)}`);
        console.log(`  upstream: ${cut(d.upstream)}`);
    }
    const samples = new Set(divergences.map((d) => d.sample)).size;
    const bugs = allow.filter((e) => e.status === 'bug' && e.hits > 0);
    console.log(
        `\n${compared} samples compared, ${divergences.length} new divergences in ${samples} samples, ` +
            `${knownBugs.length} known divergences still open (${bugs.length} allowlisted bugs)`
    );
    for (const e of bugs) console.log(`  open: ${e.title ?? e.reason}`);
}
for (const m of missing) {
    console.error(`MISSING upstream reference for ${m}: run node scripts/gen-upstream-emit.mjs`);
}
for (const e of stale) {
    console.error(`STALE allowlist entry (matches nothing): ${JSON.stringify(e.reason ?? e)}`);
}
process.exitCode = divergences.length || stale.length || missing.length ? 1 : 0;
