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
    if (c === '/' && regexCanStart(s, i)) {
        // A regex literal: to the unescaped `/` outside a class.
        let j = i + 1;
        let cls = false;
        while (j < s.length && s[j] !== '\n' && (cls || s[j] !== '/')) {
            if (s[j] === '\\') j++;
            else if (s[j] === '[') cls = true;
            else if (s[j] === ']') cls = false;
            j++;
        }
        if (s[j] === '/') {
            j++;
            while (/[a-z]/.test(s[j] ?? '')) j++;
            return j;
        }
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

/**
 * Whether a `/` at `i` starts a regex literal rather than dividing:
 * true after an operator, an opening bracket, a separator or a keyword
 * that takes an expression, false after a value (a name, a number, a
 * closing bracket).
 */
function regexCanStart(s, i) {
    let j = i - 1;
    while (j >= 0 && /\s/.test(s[j])) j--;
    if (j < 0) return true;
    const p = s[j];
    if ('(,=:[!&|?{};+-*%<>~^'.includes(p)) return true;
    if (/[\w$]/.test(p)) {
        let k = j;
        while (k >= 0 && /[\w$]/.test(s[k])) k--;
        return /^(?:return|typeof|case|do|else|in|of|void|yield|await|delete|instanceof|new)$/.test(s.slice(k + 1, j + 1));
    }
    return false;
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
            .replace(/__sveltets_\$\$bindings\b|__svn_\$\$bindings\b/g, 'H.$$$$bindings')
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
            .replace(/\bsvelteHTML\.mapElementTag\b/g, 'H.mapelementtag')
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
    // Closing parentheses of doubled pairs already dropped.
    const dropped = new Set();
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
        // Doubled parentheses, `((x))`, are one pair.
        if (c === '(' && s[i + 1] === '(' && matchClose(s, i + 1) + 1 === matchClose(s, i)) {
            dropped.add(matchClose(s, i));
            i++;
            continue;
        }
        if (dropped.has(i)) {
            i++;
            continue;
        }
        if (c === '(' && (out.endsWith(':') || out.endsWith('...'))) {
            const close = matchClose(s, i);
            let after = close + 1;
            while (dropped.has(after)) after++;
            const next = s[after];
            // Not around a comma sequence: bare, `"x":a, b` gives the
            // object a second member, and parenthesised it is one value.
            const inner = s.slice(i + 1, close);
            const one_value = splitTop(stripOuterParens(inner)).length === 1;
            if (one_value && (next === ',' || next === '}' || next === undefined)) {
                out += unwrapValues(stripOuterParens(inner));
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

/**
 * `const` declarations inside the template's body and its snippets'
 * bodies (each an `async () => {…}`): `{@const}` tags and the names an
 * `{#await}` branch binds. The construct is the pattern, with any type
 * annotation, and the initializer. Synthesised locals (`$$…`, `__svn_…`)
 * and declarations other kinds already cover (`skip`, by offset) are
 * left out.
 */
function templateConsts(original, text, skip) {
    const out = [];
    const seen = new Set();
    const body = /\basync\s*\(\s*\)\s*=>\s*\{/g;
    let r;
    while ((r = body.exec(text))) {
        const open = r.index + r[0].length - 1;
        const close = matchClose(text, open);
        const decl = /\bconst\s+/g;
        decl.lastIndex = open;
        let m;
        while ((m = decl.exec(text)) && m.index < close) {
            if (seen.has(m.index) || skip.has(m.index)) continue;
            seen.add(m.index);
            // The declaration runs to the `;` that ends it.
            let i = m.index + m[0].length;
            let eq = -1;
            while (i < close) {
                const s = skipNonCode(text, i);
                if (s >= 0) {
                    i = s;
                    continue;
                }
                const c = text[i];
                if ('([{'.includes(c)) {
                    i = matchClose(text, i) + 1;
                    continue;
                }
                if (c === ';') break;
                // A declaration without `;` ends at a line break unless
                // the next line continues the expression.
                if (c === '\n' && eq >= 0) {
                    const next = text.slice(i).match(/^\s*(\S)/);
                    if (!next || !'.?:&|+-*/%,=<>'.includes(next[1])) break;
                }
                if (c === '=' && eq < 0 && !'=>'.includes(text[i + 1]) && !'=!<>'.includes(text[i - 1])) eq = i;
                i++;
            }
            if (eq < 0) continue;
            const pattern = original.slice(m.index + m[0].length, eq);
            const init = original.slice(eq + 1, i);
            if (/\$\$|__svn|__sveltets/.test(pattern)) continue;
            if (/^\s*(?:new\b|__sveltets_2_ensureComponent\(|__svn_ensure_component\()/.test(init)) continue;
            out.push({ kind: 'const', at: m.index, label: '', text: `${normalize(pattern)}=${normalize(init)}` });
        }
    }
    return out;
}

/**
 * A render-result value as the type it gives its key. The two sides
 * spell the same type several ways: a JSDoc cast `/** @type {T} *\/
 * ({})`, `{} as any as T`, `undefined as any as (T)`, `{a: a} as T`
 * (the cast decides the type, not the value object), and a bare `""`
 * (widened to `string` in a returned object). Each becomes `T<T>`, with
 * `;` and `,` between type members read alike. A value object cast to a
 * type keeps the object when it is more than names (`{...ensureRight
 * Props<…>(…)} as T`), since that part is checked too.
 */
function canonResultValue(v) {
    v = stripOuterParens(v);
    const typeOf = (t) =>
        'T<' +
        stripOuterParens(t)
            .replace(/;([}\]])/g, '$1')
            .replace(/;/g, ',') +
        '>';
    if (v.startsWith('@type{')) {
        const close = matchClose(v, 5);
        return typeOf(v.slice(6, close));
    }
    if (v === '""') return 'T<string>';
    if (v === '{}') return 'T<{}>';
    let obj = '';
    let rest = v;
    if (v.startsWith('undefinedas')) rest = v.slice('undefined'.length);
    else if (v.startsWith('{')) {
        const close = matchClose(v, 0);
        obj = v.slice(0, close + 1);
        rest = v.slice(close + 1);
    }
    if (!rest.startsWith('as')) return v;
    rest = rest.slice(2);
    if (rest.startsWith('anyas')) rest = rest.slice(5);
    else if (rest.startsWith('unknownas')) rest = rest.slice(9);
    const names = /^\{(?:[\w$]+:[\w$]+(?:,[\w$]+:[\w$]+)*)?\}$/;
    return (obj && !names.test(obj) ? obj : '') + typeOf(rest);
}

/**
 * `T<{a?: X, b: Y}>` → `T<{a?,b}>`: the names a props or exports type
 * declares and which are optional, sorted. The member types are left
 * out: one side writes `typeof a` where the other writes the type it
 * resolves to, which only the type checker can compare. Anything that
 * is not a plain type literal is returned as it is.
 */
function memberNames(value) {
    const m = /^T<\{([\s\S]*)\}>$/.exec(value);
    if (!m) return value;
    const members = [];
    for (const piece of splitTop(m[1])) {
        if (/^(?:[\w$]+|"[^"]*")\??:/.test(piece) || members.length === 0) members.push(piece);
        else members[members.length - 1] += ',' + piece;
    }
    const names = members.filter(Boolean).map((p) => /^((?:[\w$]+|"[^"]*")\??):/.exec(p)?.[1] ?? p);
    return `T<{${names.sort().join(',')}}>`;
}

/**
 * The render function's result, one construct per key: `props`,
 * `slots`, `exports`, `bindings`. `events` is left out: svelte2tsx
 * widens it outside the result (`__sveltets_2_with_any_event`), we
 * inside, so the two never agree as text.
 *
 * A Svelte 4 component's props are a value object upstream (`{a: a}`),
 * made partly optional by the default export's `__sveltets_2_partial(
 * ['a'], …)`, and a type on our side (`{a?: typeof a}`). The value
 * object is read as that type: each `key: name` is `key: typeof name`,
 * optional when the partial list names it.
 */
function renderResult(original, text) {
    const out = [];
    const partial = /(?:__sveltets_2_partial|__sveltets_2_partial_with_any)\(\s*\[([^\]]*)\]/.exec(text);
    const optional = new Set(
        partial ? [...partial[1].matchAll(/['"]([^'"]+)['"]/g)].map((m) => m[1]) : []
    );
    const ret = /\breturn\s*\{\s*props\s*:/g;
    let m;
    while ((m = ret.exec(text))) {
        const open = m.index + m[0].indexOf('{');
        const close = matchClose(text, open);
        // Split on the commas between keys: a comma inside a type's
        // angle brackets (`as Record<string, never>`) splits too, so a
        // piece that does not start a key belongs to the one before.
        const parts = [];
        for (const piece of splitTop(original.slice(open + 1, close))) {
            if (/^\s*\w+\s*:/.test(piece) || parts.length === 0) parts.push(piece);
            else parts[parts.length - 1] += ',' + piece;
        }
        for (const part of parts) {
            const k = /^\s*(\w+)\s*:/.exec(part);
            if (!k || !['props', 'slots', 'exports', 'bindings'].includes(k[1])) continue;
            let value = unwrapValues(normalize(part.slice(k[0].length)));
            // A member may carry a JSDoc type (`/** @type {T} */ a: a`),
            // which becomes the member's type.
            const member = /^(?:@type\{([\s\S]*)\})?([\w$]+):([\w$]+)$/;
            const members = value.startsWith('{') && value.endsWith('}') ? splitTop(value.slice(1, -1)).filter(Boolean) : null;
            if (k[1] === 'props' && members && members.every((p) => member.test(p))) {
                value = `T<{${members
                    .map((p) => {
                        const [, type, key, name] = member.exec(p);
                        return `${key}${optional.has(key) ? '?' : ''}:${type ?? `typeof${name}`}`;
                    })
                    .join(',')}}>`;
            } else {
                value = canonResultValue(value);
            }
            if (k[1] === 'props' || k[1] === 'exports') value = memberNames(value);
            out.push({ kind: 'result', at: m.index, label: k[1], text: value });
        }
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
    const snippetDecls = snippets(original, text);
    for (const s of snippetDecls) found.push(s);
    // Directive helpers: the arguments decide what TypeScript checks
    // (the action / transition / animation function against the
    // element, a slot's name and the props it exposes, a get/set
    // binding's pair, a style directive's value).
    parenContent(/(?:__sveltets_2_ensureAction|__svn_ensure_action)\(/g, 'action');
    parenContent(/(?:__sveltets_2_ensureTransition|__svn_ensure_transition)\(/g, 'transition');
    parenContent(/(?:__sveltets_2_ensureAnimation|__svn_ensure_animation)\(/g, 'animation');
    parenContent(/(?:__sveltets_createSlot|__svn_create_slot)\(/g, 'slot');
    parenContent(/(?:__sveltets_2_get_set_binding|__svn_get_set_binding)\(/g, 'getset');
    parenContent(/(?:__sveltets_2_ensureType|__svn_ensure_type)\(/g, 'style');
    for (const c of found) {
        if (['action', 'transition', 'animation', 'slot', 'getset', 'style'].includes(c.kind)) {
            c.text = unwrapValues(splitTop(c.text).map(stripOuterParens).join(','));
        }
    }
    // `bind:` on an element or component writes back to its target:
    // `() => target = __sveltets_2_any(null)` upstream,
    // `void (() => { target = __svn_any(null); })` ours.
    const assign = /=>\s*\{?\s*([^;{}=]+?)\s*=\s*(?:__sveltets_2_any|__svn_any)\(\s*null\s*\)/g;
    while ((m = assign.exec(text))) {
        found.push({ kind: 'bind', at: m.index, label: '', text: normalize(m[1]) });
    }
    // `bind:name` on a component checks `name` against the component's
    // bindable props: `instance.$$bindings = 'name'`.
    const bindings = /\.\$\$bindings\s*=\s*/g;
    while ((m = bindings.exec(text))) {
        const start = m.index + m[0].length;
        const end = skipNonCode(text, start);
        if (end < 0) continue;
        found.push({ kind: 'bindings', at: m.index, label: '', text: normalize(text.slice(start, end)) });
    }
    // A slot consumer's `let:` directives: the destructuring pattern
    // and the slot it reads, `const {…} = instance.$$slot_def.name`.
    const slotLet = /\bconst\s*\{/g;
    const slotLetDecls = new Set();
    while ((m = slotLet.exec(text))) {
        const open = m.index + m[0].length - 1;
        const close = matchClose(text, open);
        const rhs = /^\s*=\s*([\w$]+)\.\$\$slot_def\s*(\.\s*[\w$]+|\[[^\]]*\])/.exec(text.slice(close + 1));
        if (!rhs) continue;
        slotLetDecls.add(m.index);
        found.push({
            kind: 'slotlet',
            at: m.index,
            label: normalize(rhs[2]).replace(/^\["(.*)"\]$/, '.$1'),
            text: normalize(original.slice(open, close + 1)),
        });
    }
    for (const c of templateConsts(original, text, new Set([...snippetDecls.map((s) => s.at), ...slotLetDecls]))) {
        found.push(c);
    }
    for (const r of renderResult(original, text)) found.push(r);
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

const KINDS = [
    'element',
    'component',
    'iso',
    'each',
    'if',
    'await',
    'snippet',
    'render',
    'on',
    'action',
    'transition',
    'animation',
    'style',
    'bind',
    'getset',
    'bindings',
    'slot',
    'slotlet',
    'const',
    'result',
];

/** A snapshot's files: `// === rel ===` header → the emit after it. */
function sections(text) {
    const out = new Map();
    const heads = [...text.matchAll(/^\/\/ === (.+) ===$/gm)];
    heads.forEach((h, i) => {
        const end = i + 1 < heads.length ? heads[i + 1].index : text.length;
        out.set(h[1], text.slice(h.index + h[0].length, end));
    });
    return out;
}

/**
 * The files svelte2tsx rejected in a sample (`upstream.error.txt`, one
 * `rel: message` line each). svelte-check reports such a file's parse
 * error and leaves it out of type checking, so there is no emit to
 * compare; the emit_snapshots test checks that we leave it out too.
 */
function rejectedFiles(sampleDir) {
    const file = path.join(sampleDir, 'upstream.error.txt');
    if (!fs.existsSync(file)) return new Map();
    const out = new Map();
    for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
        const m = /^(.+?\.svelte): (.*)$/.exec(line);
        if (m) out.set(m[1], m[2]);
    }
    return out;
}

/** Pair two lists of one kind's constructs and report what differs. */
function compareKind(kind, xs, ys, base, out) {
    // Snippets are declarations; we hoist some to a different place
    // than svelte2tsx, so their order says nothing.
    if (kind === 'snippet') {
        const key = (c) => `${c.label}(${c.text})`;
        xs = [...xs].sort((p, q) => key(p).localeCompare(key(q)));
        ys = [...ys].sort((p, q) => key(p).localeCompare(key(q)));
    }
    const n = Math.max(xs.length, ys.length);
    for (let i = 0; i < n; i++) {
        const x = xs[i];
        const y = ys[i];
        const same =
            x &&
            y &&
            x.text === y.text &&
            // An element is its tag, which decides the type its
            // attributes are checked against, plus those attributes; a
            // component is the constructor it goes through plus its props.
            (!['component', 'snippet', 'element', 'slotlet'].includes(kind) || x.label === y.label);
        if (same) continue;
        out.push({
            ...base,
            kind,
            index: i,
            label: x && y && x.label !== y.label ? `${x.label} vs ${y.label}` : (x ?? y).label,
            ours: x ? x.text : null,
            upstream: y ? y.text : null,
        });
    }
}

function compareSample(corpus, name) {
    const sampleDir = path.join(SNAP, corpus, name);
    const ours = path.join(sampleDir, 'expected.emit.ts');
    const theirs = path.join(sampleDir, 'upstream.emit.ts');
    if (!fs.existsSync(ours)) return null;
    const rejected = rejectedFiles(sampleDir);
    if (!fs.existsSync(theirs) && rejected.size === 0) return null;
    const a = sections(fs.readFileSync(ours, 'utf8'));
    const b = fs.existsSync(theirs) ? sections(fs.readFileSync(theirs, 'utf8')) : new Map();
    const sample = `${corpus}/${name}`;
    const out = [];
    let constructs = 0;
    let templateConstructs = 0;
    for (const file of [...new Set([...a.keys(), ...b.keys(), ...rejected.keys()])].sort()) {
        if (rejected.has(file)) continue;
        const base = { sample, file };
        if (!a.has(file) || !b.has(file)) {
            // A component one side emits and the other does not.
            out.push({
                ...base,
                kind: 'file',
                index: 0,
                label: file,
                ours: a.has(file) ? 'emitted' : null,
                upstream: b.has(file) ? 'emitted' : null,
            });
            continue;
        }
        const xs = extract(a.get(file));
        const ys = extract(b.get(file));
        for (const c of ys) stats.kinds[c.kind] = (stats.kinds[c.kind] ?? 0) + 1;
        constructs += xs.length + ys.length;
        templateConstructs += [...xs, ...ys].filter((c) => c.kind !== 'result').length;
        for (const kind of KINDS) {
            const xk = xs.filter((c) => c.kind === kind);
            const yk = ys.filter((c) => c.kind === kind);
            // Upstream writes the interface inline only for generic
            // components; the others go through the shim's factory.
            if (kind === 'iso' && yk.length === 0) continue;
            if (kind === 'result') {
                // The render result's keys are written in a different
                // order, and a Svelte 4 install's result has fewer of
                // them: pair each key with its namesake.
                for (const key of [...new Set([...xk, ...yk].map((c) => c.label))].sort()) {
                    compareKind(
                        kind,
                        xk.filter((c) => c.label === key),
                        yk.filter((c) => c.label === key),
                        base,
                        out
                    );
                }
                continue;
            }
            compareKind(kind, xk, yk, base, out);
        }
    }
    // Every file rejected: nothing of this sample is type-checked.
    if ([...a.keys()].every((f) => rejected.has(f))) return null;
    if (constructs === 0) stats.empty.push(sample);
    if (templateConstructs === 0) stats.resultOnly.push(sample);
    return out;
}

// ------------------------------------------------------------- allowlist

function loadAllow() {
    if (flag('--all') || !fs.existsSync(ALLOW)) return [];
    return JSON.parse(fs.readFileSync(ALLOW, 'utf8')).entries.map((e) => ({ ...e, hits: 0 }));
}

const asList = (v) => (Array.isArray(v) ? v : [v]);

/**
 * Whether allowlist entry `e` accounts for divergence `d` completely.
 * `ours` / `upstream` name the differing parts (a string, or parallel
 * lists for several parts): with each `ours` part replaced by its
 * `upstream` part, our text must become upstream's exactly, so a change
 * anywhere else in the construct is still reported. `null` on a side
 * means that side has no construct, and the other side's text must then
 * equal the entry's exactly.
 */
function explains(e, d) {
    if (e.sample !== '*' && !asList(e.sample).includes(d.sample)) return false;
    if (e.file !== undefined && e.file !== d.file) return false;
    if (e.kind !== d.kind) return false;
    if (e.label !== undefined && e.label !== d.label) return false;
    if (e.ours === null || e.upstream === null) {
        return e.ours === d.ours && e.upstream === d.upstream;
    }
    if (d.ours === null || d.upstream === null) return false;
    const from = asList(e.ours);
    const to = asList(e.upstream);
    if (from.length !== to.length || from.some((f) => !f || !d.ours.includes(f))) return false;
    let text = d.ours;
    from.forEach((f, i) => {
        text = text.split(f).join(to[i]);
    });
    return text === d.upstream;
}

function allowed(d, allow) {
    const hits = allow.filter((e) => explains(e, d));
    for (const e of hits) e.hits++;
    if (hits.length === 0) return null;
    return hits.find((e) => e.status === 'bug') ?? hits[0];
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
const stats = { kinds: {}, empty: [], resultOnly: [] };
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
    console.log(`${stats.resultOnly.length} samples reduce to the render result alone`);
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
