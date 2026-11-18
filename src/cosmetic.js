// Procedural cosmetic filters: hide (or remove, or restyle) elements that a plain CSS selector
// can't describe — `.card:has-text(Sponsored)`, `.ad-label:upward(2)`, `div:matches-css(position:
// fixed)`, `.slot:remove()`. Written for Ironswift (MIT); the syntax is the one uBlock Origin
// and Adblock Plus filter lists use.
//
// This is the body of `function(rules) {…}`; `rules` is a list of selector strings.
'use strict';
var win = window;
var doc = document;
var setT = win.setTimeout;

function pattern(s) {
  s = String(s === undefined ? '' : s).trim();
  if ((s.charAt(0) === '"' && s.charAt(s.length - 1) === '"') || (s.charAt(0) === "'" && s.charAt(s.length - 1) === "'")) s = s.slice(1, -1);
  var m = /^\/(.+)\/([imsu]*)$/.exec(s);
  if (m) { try { return new RegExp(m[1], m[2]); } catch (e) { return /(?!)/; } }
  return new RegExp(s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
}

var OPS = ['has-text', 'contains', '-abp-contains', 'has', '-abp-has', 'if', 'if-not', 'not', 'upward', 'nth-ancestor', 'matches-css',
  'matches-css-before', 'matches-css-after', '-abp-properties', 'min-text-length', 'matches-attr', 'matches-path', 'matches-media',
  'matches-prop', 'xpath', 'watch-attr', 'remove', 'remove-attr', 'remove-class', 'style', 'others', 'shadow'];
var ACTIONS = { remove: 1, 'remove-attr': 1, 'remove-class': 1, style: 1 };

// Find the closing parenthesis for an opening one at `i`, skipping quoted and escaped text.
function closing(s, i) {
  var depth = 0, quote = null;
  for (var j = i; j < s.length; j++) {
    var c = s.charAt(j);
    if (c === '\\') { j++; continue; }
    if (quote) { if (c === quote) quote = null; continue; }
    if (c === '"' || c === "'") { quote = c; continue; }
    if (c === '(') depth++;
    else if (c === ')') { depth--; if (depth === 0) return j; }
  }
  return -1;
}

// The next `:op(` at bracket depth 0, as {at, op, argStart, end}.
function nextOp(s, from) {
  var depth = 0, quote = null;
  for (var j = from; j < s.length; j++) {
    var c = s.charAt(j);
    if (c === '\\') { j++; continue; }
    if (quote) { if (c === quote) quote = null; continue; }
    if (c === '"' || c === "'") { quote = c; continue; }
    if (c === '(' || c === '[') depth++;
    else if (c === ')' || c === ']') depth--;
    else if (c === ':' && depth === 0) {
      var m = /^:([-a-z]+)\(/.exec(s.slice(j));
      if (m && OPS.indexOf(m[1]) >= 0) {
        // `:has(` and `:not(` with plain CSS inside are left to the style engine.
        var open = j + m[0].length - 1;
        var end = closing(s, open);
        if (end < 0) return null;
        var arg = s.slice(open + 1, end);
        if ((m[1] === 'has' || m[1] === 'not' || m[1] === 'is') && !isProcedural(arg)) continue;
        return { at: j, op: m[1], arg: arg, end: end + 1 };
      }
    }
  }
  return null;
}

function isProcedural(s) {
  for (var i = 0; i < OPS.length; i++) if (OPS[i] !== 'has' && OPS[i] !== 'not' && s.indexOf(':' + OPS[i] + '(') >= 0) return true;
  return false;
}

// Parse a selector into steps: {css}, {op, arg}, ending with an optional action.
function parse(sel) {
  var steps = [];
  var pos = 0;
  for (;;) {
    var n = nextOp(sel, pos);
    var raw = sel.slice(pos, n ? n.at : sel.length);
    var css = raw.trim();
    // After an operator, `:has-text(x) .y` means descendants (a leading space).
    if (css && steps.length && /^\s/.test(raw) && !/^[>+~]/.test(css)) css = ' ' + css;
    if (css) steps.push({ css: css });
    if (!n) break;
    steps.push({ op: n.op, arg: n.arg });
    pos = n.end;
  }
  var action = null;
  var last = steps[steps.length - 1];
  if (last && ACTIONS[last.op]) { action = last; steps.pop(); }
  return { steps: steps, action: action };
}

var cache = {};
function compiled(sel) {
  if (!cache[sel]) cache[sel] = parse(sel);
  return cache[sel];
}

function toArray(list) { return Array.prototype.slice.call(list || []); }

// Elements related to `node` by a CSS continuation like `> span`, `+ .x`, ` .y`, `.z`.
function relative(node, css) {
  css = css.trim();
  var c = css.charAt(0);
  try {
    if (c === '>') {
      var rest = css.slice(1).trim();
      // `> a b`: children matching the first compound, then descend.
      var parts = rest.split(/\s+(?![^[(]*[\])])/);
      var first = parts.shift();
      var kids = toArray(node.children).filter(function(k) { return k.matches(first); });
      if (parts.length === 0) return kids;
      var out = [];
      kids.forEach(function(k) { out = out.concat(relative(k, parts.join(' '))); });
      return out;
    }
    if (c === '+') {
      var s = node.nextElementSibling;
      return s && s.matches(css.slice(1).trim()) ? [s] : [];
    }
    if (c === '~') {
      var res = [], sib = node.nextElementSibling, q = css.slice(1).trim();
      while (sib) { if (sib.matches(q)) res.push(sib); sib = sib.nextElementSibling; }
      return res;
    }
    return toArray(node.querySelectorAll(css));
  } catch (e) {
    return [];
  }
}

function textOf(el) { return el.textContent || ''; }

// Elements (from `nodes`, or the document) that a parsed selector selects.
function run(p, root) {
  var nodes = null;
  for (var i = 0; i < p.steps.length; i++) {
    var st = p.steps[i];
    if (st.css !== undefined) {
      if (nodes === null) {
        var c = st.css.charAt(0);
        if (root && (c === '>' || c === '+' || c === '~')) nodes = relative(root, st.css);
        else { try { nodes = toArray((root || doc).querySelectorAll(st.css)); } catch (e) { return []; } }
      } else {
        var out = [];
        var cc = st.css.charAt(0);
        nodes.forEach(function(n) {
          if (cc === '>' || cc === '+' || cc === '~' || cc === ' ') out = out.concat(relative(n, st.css));
          else {
            // A compound directly after an operator: filter, or a descendant step if spaced.
            try { if (n.matches(st.css)) out.push(n); } catch (e) {}
          }
        });
        nodes = out;
      }
      continue;
    }
    if (nodes === null) nodes = toArray((root || doc).querySelectorAll('*'));
    nodes = applyOp(st.op, st.arg, nodes, root);
    if (nodes.length === 0) break;
  }
  if (nodes === null) return [];
  // No duplicates.
  return nodes.filter(function(n, k) { return nodes.indexOf(n) === k; });
}

function cssMatcher(arg) {
  var i = arg.indexOf(':');
  if (i < 0) return null;
  var prop = arg.slice(0, i).trim();
  var val = arg.slice(i + 1).trim();
  var re = /^\/.+\/[imsu]*$/.test(val) ? pattern(val) : new RegExp('^' + val.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*') + '$');
  return { prop: prop, re: re };
}

function applyOp(op, arg, nodes, root) {
  var re, n, m, sub;
  switch (op) {
    case 'has-text': case 'contains': case '-abp-contains':
      re = pattern(arg);
      return nodes.filter(function(el) { return re.test(textOf(el)); });
    case 'has': case '-abp-has': case 'if':
      sub = compiled(arg.trim());
      return nodes.filter(function(el) { return run(sub, el).length > 0; });
    case 'if-not':
      sub = compiled(arg.trim());
      return nodes.filter(function(el) { return run(sub, el).length === 0; });
    case 'not':
      sub = compiled(arg.trim());
      var excluded = run(sub, null);
      return nodes.filter(function(el) { return excluded.indexOf(el) < 0; });
    case 'upward':
      n = Number(arg);
      return nodes.map(function(el) {
        if (n >= 1 && n <= 256) { var a = el; for (var k = 0; k < n && a; k++) a = a.parentElement; return a; }
        var p = el.parentElement;
        try { return p ? p.closest(arg) : null; } catch (e) { return null; }
      }).filter(Boolean);
    case 'nth-ancestor':
      n = Number(arg);
      return nodes.map(function(el) { var a = el; for (var k = 0; k < n && a; k++) a = a.parentElement; return a; }).filter(Boolean);
    case 'matches-css': case 'matches-css-before': case 'matches-css-after': case '-abp-properties':
      m = cssMatcher(arg);
      if (!m) return [];
      var pseudo = op === 'matches-css-before' ? '::before' : op === 'matches-css-after' ? '::after' : null;
      return nodes.filter(function(el) {
        var cs;
        try { cs = pseudo ? win.getComputedStyle(el, pseudo) : win.getComputedStyle(el); } catch (e) { return false; }
        return cs && m.re.test(cs.getPropertyValue(m.prop));
      });
    case 'min-text-length':
      n = Number(arg);
      return nodes.filter(function(el) { return textOf(el).length >= n; });
    case 'matches-attr':
      var eq = arg.indexOf('=');
      var nameRe = pattern(eq < 0 ? arg : arg.slice(0, eq));
      var valRe = eq < 0 ? null : pattern(arg.slice(eq + 1));
      return nodes.filter(function(el) {
        var attrs = el.attributes || [];
        for (var k = 0; k < attrs.length; k++) {
          if (nameRe.test(attrs[k].name) && (!valRe || valRe.test(attrs[k].value))) return true;
        }
        return false;
      });
    case 'matches-path':
      re = pattern(arg);
      return re.test(location.pathname + location.search) ? nodes : [];
    case 'matches-media':
      try { return win.matchMedia(arg).matches ? nodes : []; } catch (e) { return []; }
    case 'matches-prop':
      var e2 = arg.indexOf('=');
      var chain = (e2 < 0 ? arg : arg.slice(0, e2)).split('.');
      var pv = e2 < 0 ? null : pattern(arg.slice(e2 + 1));
      return nodes.filter(function(el) {
        var v = el;
        for (var k = 0; k < chain.length; k++) { if (v === null || v === undefined) return false; v = v[chain[k]]; }
        return pv ? pv.test(String(v)) : v !== undefined;
      });
    case 'xpath':
      if (typeof doc.evaluate !== 'function') return [];
      var out = [];
      nodes.forEach(function(el) {
        try {
          var r = doc.evaluate(arg, el, null, 7, null);
          for (var k = 0; k < r.snapshotLength; k++) { var x = r.snapshotItem(k); if (x && x.nodeType === 1) out.push(x); }
        } catch (e) {}
      });
      return out;
    case 'watch-attr':
      return nodes;
    default:
      // :others(), :shadow() and anything unknown select nothing (never too much).
      return [];
  }
}

// Declarations from :style(), with no url() and every one !important.
function styleDecls(arg) {
  if (/[{}<\\]|url\(|expression\(/i.test(arg)) return null;
  return arg.split(';').map(function(d) { return d.trim(); }).filter(Boolean).map(function(d) {
    var i = d.indexOf(':');
    return i > 0 ? [d.slice(0, i).trim(), d.slice(i + 1).replace(/!important\s*$/, '').trim()] : null;
  }).filter(Boolean);
}

function apply(el, action) {
  if (!action) {
    if (el.style.getPropertyValue('display') !== 'none') el.style.setProperty('display', 'none', 'important');
    return;
  }
  switch (action.op) {
    case 'remove':
      if (el.parentNode) el.parentNode.removeChild(el);
      return;
    case 'remove-attr':
      String(action.arg).split('|').forEach(function(a) { a = a.trim(); if (a && !/^on/i.test(a)) el.removeAttribute(a); });
      return;
    case 'remove-class':
      String(action.arg).split('|').forEach(function(c) { if (c.trim()) el.classList.remove(c.trim()); });
      return;
    case 'style':
      var decls = styleDecls(action.arg);
      if (!decls) return;
      decls.forEach(function(d) { el.style.setProperty(d[0], d[1], 'important'); });
      return;
  }
}

function pass() {
  for (var i = 0; i < rules.length; i++) {
    var p;
    try { p = compiled(rules[i]); } catch (e) { continue; }
    var els;
    try { els = run(p, null); } catch (e) { continue; }
    for (var k = 0; k < els.length; k++) { try { apply(els[k], p.action); } catch (e) {} }
  }
}

// Run when the page has its content, and again as it changes (at most every 150 ms).
var pending = false;
function schedule() {
  if (pending) return;
  pending = true;
  setT.call(win, function() { pending = false; pass(); }, 150);
}
function start() {
  pass();
  if (typeof MutationObserver === 'function') {
    var opts = { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'id', 'hidden'] };
    // Only "something changed" is needed (prelude.js): no full record per change on the page.
    if (typeof __moSignalOnly !== 'undefined') opts[__moSignalOnly] = true;
    new MutationObserver(schedule).observe(doc.documentElement || doc, opts);
  }
}
if (doc.readyState === 'loading') doc.addEventListener('DOMContentLoaded', start, { once: true });
else start();
win.addEventListener('load', schedule, { once: true });
