// Scriptlets: small pieces of code filter lists run inside a page before its own scripts
// (`example.com##+js(set-constant, adBlockDetected, false)`), to defuse anti-adblock walls and
// ad scripts that a request filter can't reach. Written for Ironswift (MIT). The names and
// arguments follow the scriptlets uBlock Origin's filter lists use, so those lists work here.
//
// This is the body of `function(calls) {…}`; `calls` is a list of [name, arg, …]. A page gets
// the shared helpers and only the `def(…)` blocks its calls name (each block starts with a line
// `def('name alias…', …` and ends with a line `});`).
'use strict';
var win = window;
var doc = document;
var log = function() {};
// Saved before the page can replace them.
var safe = {
  defineProperty: Object.defineProperty,
  getOwnPropertyDescriptor: Object.getOwnPropertyDescriptor,
  RegExp: RegExp,
  JSON_parse: JSON.parse,
  JSON_stringify: JSON.stringify,
  setTimeout: win.setTimeout,
  setInterval: win.setInterval,
  apply: Reflect.apply,
  Proxy: Proxy,
};

// A text pattern: "" matches anything, "/re/flags" a regular expression, "!…" negates,
// anything else is a plain substring.
function pattern(s) {
  s = s === undefined || s === null ? '' : String(s);
  var not = false;
  if (s.charAt(0) === '!') { not = true; s = s.slice(1); }
  var re;
  if (s === '') re = /^/;
  else {
    var m = /^\/(.+)\/([gimsu]*)$/.exec(s);
    if (m) { try { re = new safe.RegExp(m[1], m[2].replace('g', '')); } catch (e) { re = /(?!)/; } }
    else re = new safe.RegExp(s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  }
  return { test: function(t) { return re.test(String(t)) !== not; }, empty: s === '' && !not };
}

// Arguments given as "name, value" pairs after the positional ones.
function extra(args, from) {
  var o = {};
  for (var i = from; i + 1 < args.length; i += 2) o[args[i]] = args[i + 1];
  return o;
}

function onReady(f) {
  if (doc.readyState === 'loading') doc.addEventListener('DOMContentLoaded', f, { once: true });
  else f();
}

// Run f now, when the page has loaded, and whenever it changes (throttled).
function watchDom(f, attrs) {
  var pending = false;
  var run = function() { pending = false; try { f(); } catch (e) { log(e); } };
  onReady(function() {
    run();
    if (typeof MutationObserver !== 'function') return;
    var mo = new MutationObserver(function() {
      if (pending) return;
      pending = true;
      safe.setTimeout.call(win, run, 50);
    });
    var opts = { childList: true, subtree: true };
    if (attrs) { opts.attributes = true; opts.attributeFilter = attrs; }
    mo.observe(doc.documentElement || doc, opts);
  });
  win.addEventListener('load', run, { once: true });
}

// Values a set-constant-like scriptlet may set.
function constant(v) {
  switch (v) {
    case 'undefined': return { v: undefined };
    case 'false': return { v: false };
    case 'true': return { v: true };
    case 'null': return { v: null };
    case "''": case '""': case 'emptyStr': return { v: '' };
    case '[]': case 'emptyArr': return { v: [] };
    case '{}': case 'emptyObj': return { v: {} };
    case 'noopFunc': return { v: function() {} };
    case 'trueFunc': return { v: function() { return true; } };
    case 'falseFunc': return { v: function() { return false; } };
    case 'throwFunc': return { v: function() { throw new TypeError('blocked'); } };
    case 'noopCallbackFunc': return { v: function() { return function() {}; } };
    case 'noopThisFunc': return { v: function() { return this; } };
    case 'noopPromiseResolve': return { v: function() { return Promise.resolve(); } };
    case 'noopPromiseReject': return { v: function() { return Promise.reject(); } };
    case 'yes': return { v: 'yes' };
    case 'no': return { v: 'no' };
    case 'NaN': return { v: NaN };
    case 'Infinity': return { v: Infinity };
    case '-1': return { v: -1 };
    case '': return null;
  }
  if (/^-?\d+(\.\d+)?$/.test(v)) {
    var n = Number(v);
    if (Math.abs(n) <= 0x7FFF) return { v: n };
    return null;
  }
  if (/^'[^']*'$/.test(v) || /^"[^"]*"$/.test(v)) return { v: v.slice(1, -1) };
  return null;
}

// Define `chain` (a.b.c) on window, so that reading or writing its last part calls the given
// trap — even when a.b doesn't exist yet: each missing link is watched until the page makes it.
function trapChain(chain, makeDesc) {
  var parts = chain.split('.');
  function at(owner, i) {
    if (owner === null || (typeof owner !== 'object' && typeof owner !== 'function')) return;
    var prop = parts[i];
    if (i === parts.length - 1) {
      var d = makeDesc(owner, prop);
      if (!d) return;
      try { safe.defineProperty(owner, prop, d); } catch (e) { log(e); }
      return;
    }
    var cur = owner[prop];
    if (cur !== undefined && cur !== null && (typeof cur === 'object' || typeof cur === 'function')) {
      at(cur, i + 1);
      return;
    }
    // Wait for the page to create this link.
    var desc = safe.getOwnPropertyDescriptor(owner, prop);
    if (desc && desc.configurable === false) return;
    var value;
    try {
      safe.defineProperty(owner, prop, {
        configurable: true,
        enumerable: true,
        get: function() { return value; },
        set: function(v) {
          value = v;
          if (v !== null && (typeof v === 'object' || typeof v === 'function')) at(v, i + 1);
        },
      });
    } catch (e) { log(e); }
  }
  at(win, 0);
}

function currentScriptText() {
  var s = doc.currentScript;
  if (!s) return '';
  if (s.src) return s.src + '\n' + (s.textContent || '');
  return s.textContent || '';
}

function toArray(l) { return Array.prototype.slice.call(l || []); }

var scriptlets = {};
var aliases = {};
function def(names, f) {
  names = names.split(' ');
  scriptlets[names[0]] = f;
  for (var i = 1; i < names.length; i++) aliases[names[i]] = names[0];
}

// ---------------------------------------------------------------- properties

def('set-constant set', function(chain, value) {
  var c = constant(value);
  if (!chain || !c) return;
  var v = c.v;
  var opts = extra(arguments, 2);
  trapChain(chain, function(owner, prop) {
    var d = safe.getOwnPropertyDescriptor(owner, prop);
    if (d && d.configurable === false) return null;
    // Keep the original's type: a function stays callable when the value is a function.
    return {
      configurable: false,
      enumerable: d ? d.enumerable : true,
      get: function() { return v; },
      set: function(nv) { if (opts.as === 'callback' && typeof nv === 'function') v = nv; },
    };
  });
});

def('abort-on-property-read aopr', function(chain) {
  if (!chain) return;
  trapChain(chain, function() {
    return { configurable: false, get: function() { throw new ReferenceError('blocked'); }, set: function() {} };
  });
});

def('abort-on-property-write aopw', function(chain) {
  if (!chain) return;
  trapChain(chain, function(owner, prop) {
    var v = owner[prop];
    return { configurable: false, get: function() { return v; }, set: function() { throw new ReferenceError('blocked'); } };
  });
});

// Throw when the named property is touched by a script whose text (or address) matches.
def('abort-current-script acs abort-current-inline-script acis', function(chain, needle) {
  if (!chain) return;
  var p = pattern(needle);
  var check = function() {
    var s = doc.currentScript;
    if (!s) return;
    if (p.test(currentScriptText())) throw new ReferenceError('blocked');
  };
  trapChain(chain, function(owner, prop) {
    var d = safe.getOwnPropertyDescriptor(owner, prop);
    if (d && d.configurable === false) return null;
    var v = owner[prop];
    var get = d && d.get, set = d && d.set;
    return {
      configurable: true,
      enumerable: d ? d.enumerable : true,
      get: function() { check(); return get ? get.call(this) : v; },
      set: function(nv) { check(); if (set) set.call(this, nv); else v = nv; },
    };
  });
});

// Throw when the property is read by code whose call stack matches.
def('abort-on-stack-trace aost', function(chain, needle) {
  if (!chain) return;
  var p = pattern(needle);
  var check = function() {
    var st = '';
    try { st = String(new Error().stack || ''); } catch (e) {}
    if (st && p.test(st)) throw new ReferenceError('blocked');
  };
  trapChain(chain, function(owner, prop) {
    var v = owner[prop];
    return { configurable: true, get: function() { check(); return v; }, set: function(nv) { check(); v = nv; } };
  });
});

// ---------------------------------------------------------------- timers

function timerDefuser(name, needle, delay, booster) {
  var p = pattern(needle);
  var d = delay === undefined || delay === '' ? null : String(delay);
  var orig = win[name];
  if (typeof orig !== 'function') return;
  win[name] = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var fn = args[0];
      var text = typeof fn === 'function' ? Function.prototype.toString.call(fn) : String(fn);
      var ms = Number(args[1]) || 0;
      var delayOk = d === null || (d.charAt(0) === '!' ? ms !== Number(d.slice(1)) : ms === Number(d));
      if (booster) {
        if (p.test(text) && delayOk) args[1] = ms * booster;
      } else if (p.test(text) && delayOk) {
        args[0] = function() {};
      }
      return safe.apply(target, self, args);
    },
  });
}

def('no-setTimeout-if nostif prevent-setTimeout setTimeout-defuser std', function(needle, delay) {
  timerDefuser('setTimeout', needle, delay);
});
def('no-setInterval-if nosiif prevent-setInterval setInterval-defuser sid', function(needle, delay) {
  timerDefuser('setInterval', needle, delay);
});
def('adjust-setTimeout nano-setTimeout-booster nano-stb', function(needle, delay, boost) {
  var b = Number(boost);
  if (!(b >= 0.001 && b <= 50)) b = 0.05;
  timerDefuser('setTimeout', needle, delay === undefined ? '1000' : delay, b);
});
def('adjust-setInterval nano-setInterval-booster nano-sib', function(needle, delay, boost) {
  var b = Number(boost);
  if (!(b >= 0.001 && b <= 50)) b = 0.05;
  timerDefuser('setInterval', needle, delay === undefined ? '1000' : delay, b);
});
def('no-requestAnimationFrame-if norafif prevent-requestAnimationFrame', function(needle) {
  var p = pattern(needle);
  var orig = win.requestAnimationFrame;
  if (typeof orig !== 'function') return;
  win.requestAnimationFrame = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      if (typeof args[0] === 'function' && p.test(Function.prototype.toString.call(args[0]))) args[0] = function() {};
      return safe.apply(target, self, args);
    },
  });
});

// ---------------------------------------------------------------- events, eval, windows

def('addEventListener-defuser aeld prevent-addEventListener', function(type, needle) {
  var t = pattern(type), p = pattern(needle);
  var proto = win.EventTarget && win.EventTarget.prototype;
  var owner = proto && proto.addEventListener ? proto : win;
  var orig = owner.addEventListener;
  if (typeof orig !== 'function') return;
  owner.addEventListener = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var fn = args[1];
      var text = '';
      try { text = typeof fn === 'function' ? Function.prototype.toString.call(fn) : (fn && typeof fn.handleEvent === 'function' ? Function.prototype.toString.call(fn.handleEvent) : String(fn)); } catch (e) {}
      if (t.test(args[0]) && p.test(text)) return undefined;
      return safe.apply(target, self, args);
    },
  });
});

def('noeval-if prevent-eval-if', function(needle) {
  var p = pattern(needle);
  var orig = win.eval;
  win.eval = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      if (p.test(args[0])) return undefined;
      return safe.apply(target, self, args);
    },
  });
});
def('noeval silent-noeval', function() {
  win.eval = new safe.Proxy(win.eval, { apply: function() { return undefined; } });
});

def('no-window-open-if nowoif prevent-window-open window.open-defuser', function(needle, delay, decoy) {
  var p = pattern(needle);
  var orig = win.open;
  if (typeof orig !== 'function') return;
  win.open = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      if (!p.test(args[0] === undefined ? '' : args[0])) return safe.apply(target, self, args);
      // A stand-in window, so pages that check the result don't think they were blocked.
      if (decoy !== undefined && decoy !== '') {
        return { closed: false, close: function() { this.closed = true; }, focus: function() {}, blur: function() {}, document: doc, location: { href: '' }, postMessage: function() {} };
      }
      return null;
    },
  });
});

def('window.name-defuser', function() {
  if (win === win.top) win.name = '';
});

def('nowebrtc', function() {
  var names = ['RTCPeerConnection', 'webkitRTCPeerConnection', 'mozRTCPeerConnection'];
  for (var i = 0; i < names.length; i++) {
    if (typeof win[names[i]] !== 'function') continue;
    var Fake = function() {};
    Fake.prototype = { close: function() {}, createDataChannel: function() { return {}; }, createOffer: function() { return Promise.reject(new Error('blocked')); }, setLocalDescription: function() {}, setRemoteDescription: function() {}, addEventListener: function() {}, removeEventListener: function() {} };
    win[names[i]] = Fake;
  }
});

def('alert-buster', function() {
  win.alert = function() {};
});

def('call-nothrow', function(chain) {
  if (!chain) return;
  var parts = chain.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner[parts[i]]; if (!owner) return; }
  var prop = parts[parts.length - 1];
  var f = owner[prop];
  if (typeof f !== 'function') return;
  owner[prop] = new safe.Proxy(f, { apply: function(t, self, args) { try { return safe.apply(t, self, args); } catch (e) { return undefined; } } });
});

def('prevent-refresh refresh-defuser', function(delay) {
  onReady(function() {
    var metas = doc.querySelectorAll('meta[http-equiv="refresh" i]');
    for (var i = 0; i < metas.length; i++) metas[i].remove();
  });
});

def('disable-newtab-links', function() {
  doc.addEventListener('click', function(ev) {
    var a = ev.target && ev.target.closest ? ev.target.closest('a[target="_blank"]') : null;
    if (a) a.removeAttribute('target');
  }, true);
});

def('close-window window-close-if', function(needle) {
  if (pattern(needle).test(location.pathname + location.search)) { try { win.close(); } catch (e) {} }
});

// ---------------------------------------------------------------- network

function matchProps(spec, values) {
  // "url:/ads/ method:POST" or a bare pattern for the URL.
  if (!spec) return true;
  var parts = String(spec).split(/\s+/);
  for (var i = 0; i < parts.length; i++) {
    var kv = parts[i], k = 'url', v = kv;
    var c = kv.indexOf(':');
    if (c > 0 && /^[a-z]+$/i.test(kv.slice(0, c))) { k = kv.slice(0, c); v = kv.slice(c + 1); }
    if (!pattern(v).test(values[k] === undefined ? '' : values[k])) return false;
  }
  return true;
}

def('no-fetch-if prevent-fetch', function(spec, body) {
  var orig = win.fetch;
  if (typeof orig !== 'function') return;
  win.fetch = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var r = args[0], o = args[1] || {};
      var url = typeof r === 'string' ? r : (r && r.url) || String(r);
      var method = o.method || (r && r.method) || 'GET';
      if (!matchProps(spec, { url: url, method: method })) return safe.apply(target, self, args);
      var text = body === 'emptyObj' ? '{}' : body === 'emptyArr' ? '[]' : '';
      return Promise.resolve(new Response(text, { status: 200, statusText: 'OK', headers: { 'Content-Type': body === 'emptyObj' || body === 'emptyArr' ? 'application/json' : 'text/plain' } }));
    },
  });
});

// Answer matching XMLHttpRequests with `text` instead of sending them.
function preventXhr(spec, text) {
  var XHR = win.XMLHttpRequest;
  if (typeof XHR !== 'function') return;
  var open = XHR.prototype.open, send = XHR.prototype.send;
  var hits = new WeakMap();
  XHR.prototype.open = function(method, url) {
    if (matchProps(spec, { url: String(url), method: String(method) })) hits.set(this, true);
    else hits.delete(this);
    return safe.apply(open, this, arguments);
  };
  XHR.prototype.send = function() {
    if (!hits.has(this)) return safe.apply(send, this, arguments);
    var xhr = this;
    var set = function(k, v) { try { safe.defineProperty(xhr, k, { value: v, configurable: true }); } catch (e) {} };
    set('readyState', 4);
    set('status', 200);
    set('statusText', 'OK');
    set('responseText', text);
    set('response', xhr.responseType === 'json' ? safe.JSON_parse(text || 'null') : text);
    safe.setTimeout.call(win, function() {
      ['readystatechange', 'load', 'loadend'].forEach(function(t) {
        try { xhr.dispatchEvent(new Event(t)); } catch (e) {}
        var h = xhr['on' + t];
        if (typeof h === 'function') { try { h.call(xhr, new Event(t)); } catch (e) {} }
      });
    }, 1);
  };
}

def('no-xhr-if prevent-xhr', function(spec, body) {
  preventXhr(spec, body === 'emptyObj' ? '{}' : body === 'emptyArr' ? '[]' : '');
});

// ---------------------------------------------------------------- JSON

// Remove properties (dotted paths, `[]` for every array item, `*` for any key) from objects.
function prune(obj, paths, needs) {
  if (obj === null || typeof obj !== 'object') return false;
  var has = function(o, path) {
    var ps = path.split('.');
    var walk = function(o, i) {
      if (o === null || typeof o !== 'object') return false;
      var k = ps[i];
      if (i === ps.length - 1) return k === '*' ? Object.keys(o).length > 0 : Object.prototype.hasOwnProperty.call(o, k);
      if (k === '[]' || k === '*' || k === '[-]') { for (var key in o) if (walk(o[key], i + 1)) return true; return false; }
      return walk(o[k], i + 1);
    };
    return walk(o, 0);
  };
  for (var i = 0; i < needs.length; i++) if (!has(obj, needs[i])) return false;
  var pruned = false;
  for (var j = 0; j < paths.length; j++) {
    var ps = paths[j].split('.');
    var go = function(o, i) {
      if (o === null || typeof o !== 'object') return;
      var k = ps[i];
      if (i === ps.length - 1) {
        if (k === '*' || k === '[]') { for (var key in o) { delete o[key]; pruned = true; } }
        else if (Object.prototype.hasOwnProperty.call(o, k)) { delete o[k]; pruned = true; }
        return;
      }
      // `[-]`: remove the array items in which the rest of the path is found.
      if (k === '[-]' && Array.isArray(o)) {
        var rest = ps.slice(i + 1).join('.');
        for (var n = o.length - 1; n >= 0; n--) if (has(o[n], rest)) { o.splice(n, 1); pruned = true; }
        return;
      }
      if (k === '[]' || k === '*') { for (var key2 in o) go(o[key2], i + 1); return; }
      go(o[k], i + 1);
    };
    go(obj, 0);
  }
  return pruned;
}

function pruneArgs(paths, needs) {
  return {
    paths: (paths || '').split(/\s+/).filter(Boolean),
    needs: (needs || '').split(/\s+/).filter(Boolean),
  };
}

def('json-prune', function(paths, needs) {
  var a = pruneArgs(paths, needs);
  if (a.paths.length === 0) return;
  JSON.parse = new safe.Proxy(JSON.parse, {
    apply: function(target, self, args) {
      var r = safe.apply(target, self, args);
      prune(r, a.paths, a.needs);
      return r;
    },
  });
});

def('json-prune-fetch-response', function(paths, needs) {
  var a = pruneArgs(paths, needs);
  var opts = extra(arguments, 2);
  var orig = win.fetch;
  if (a.paths.length === 0 || typeof orig !== 'function') return;
  win.fetch = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var r = args[0];
      var url = typeof r === 'string' ? r : (r && r.url) || String(r);
      var p = safe.apply(target, self, args);
      if (!matchProps(opts.propsToMatch, { url: url })) return p;
      return p.then(function(resp) {
        return resp.clone().text().then(function(text) {
          var obj;
          try { obj = safe.JSON_parse(text); } catch (e) { return resp; }
          if (!prune(obj, a.paths, a.needs)) return resp;
          return new Response(safe.JSON_stringify(obj), { status: resp.status, statusText: resp.statusText, headers: resp.headers });
        });
      });
    },
  });
});

def('json-prune-xhr-response', function(paths, needs) {
  var a = pruneArgs(paths, needs);
  var opts = extra(arguments, 2);
  var XHR = win.XMLHttpRequest;
  if (a.paths.length === 0 || typeof XHR !== 'function') return;
  var open = XHR.prototype.open;
  var urls = new WeakMap();
  XHR.prototype.open = function(method, url) { urls.set(this, String(url)); return safe.apply(open, this, arguments); };
  var desc = safe.getOwnPropertyDescriptor(XHR.prototype, 'responseText');
  if (!desc || !desc.get) return;
  var fix = function(xhr, v) {
    if (typeof v !== 'string' || !matchProps(opts.propsToMatch, { url: urls.get(xhr) || '' })) return v;
    var obj;
    try { obj = safe.JSON_parse(v); } catch (e) { return v; }
    return prune(obj, a.paths, a.needs) ? safe.JSON_stringify(obj) : v;
  };
  safe.defineProperty(XHR.prototype, 'responseText', { configurable: true, get: function() { return fix(this, desc.get.call(this)); } });
});

// ---------------------------------------------------------------- the DOM

def('remove-attr ra', function(attrs, selector) {
  var names = String(attrs || '').split('|').filter(Boolean);
  if (names.length === 0) return;
  var sel = selector || names.map(function(n) { return '[' + n + ']'; }).join(',');
  watchDom(function() {
    var els = doc.querySelectorAll(sel);
    for (var i = 0; i < els.length; i++) for (var j = 0; j < names.length; j++) els[i].removeAttribute(names[j]);
  }, names);
});

def('remove-class rc', function(classes, selector) {
  var names = String(classes || '').split('|').filter(Boolean);
  if (names.length === 0) return;
  var sel = selector || names.map(function(n) { return '.' + n; }).join(',');
  watchDom(function() {
    var els = doc.querySelectorAll(sel);
    for (var i = 0; i < els.length; i++) for (var j = 0; j < names.length; j++) els[i].classList.remove(names[j]);
  }, ['class']);
});

def('set-attr', function(selector, attr, value) {
  if (!selector || !attr || /^on/i.test(attr)) return;
  var v = value === undefined ? '' : String(value);
  if (!/^(?:\d+|true|false|''|"")$/.test(v) && v !== '') return;
  v = v.replace(/^['"]|['"]$/g, '');
  watchDom(function() {
    var els = doc.querySelectorAll(selector);
    for (var i = 0; i < els.length; i++) if (els[i].getAttribute(attr) !== v) els[i].setAttribute(attr, v);
  }, [attr]);
});

def('remove-node-text rmnt', function(nodeName, needle) {
  var n = pattern(nodeName), p = pattern(needle);
  watchDom(function() {
    var els = doc.querySelectorAll('*');
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      if (el.childElementCount === 0 && n.test(el.nodeName.toLowerCase()) && p.test(el.textContent)) el.textContent = '';
    }
  });
});

// ---------------------------------------------------------------- storage

function allowedValue(v) {
  var ok = ['true', 'false', 'yes', 'no', 'y', 'n', 'ok', 'on', 'off', 'accept', 'accepted', 'reject', 'rejected', 'allow', 'deny', 'necessary', 'required', 'hide', 'hidden', 'essential', 'nonessential', 'checked', 'unchecked', 'dismiss', 'dismissed', '', '$now$'];
  v = String(v === undefined ? '' : v);
  if (v === 'emptyArr') return '[]';
  if (v === 'emptyObj') return '{}';
  if (v === 'undefined') return undefined;
  if (/^\d+$/.test(v) && Number(v) <= 0x7FFF) return v;
  if (ok.indexOf(v.toLowerCase()) >= 0) return v === '$now$' ? String(Date.now()) : v;
  return null;
}

def('set-cookie', function(name, value, path) {
  var v = allowedValue(value);
  if (!name || v === null || v === undefined) return;
  var p = path === 'none' ? '' : '; path=' + (path || '/');
  try { doc.cookie = encodeURIComponent(name) + '=' + encodeURIComponent(v) + p; } catch (e) {}
});

def('remove-cookie cookie-remover', function(needle) {
  var p = pattern(needle);
  var run = function() {
    var all = (doc.cookie || '').split(';');
    for (var i = 0; i < all.length; i++) {
      var name = all[i].split('=')[0].trim();
      if (!name || !p.test(name)) continue;
      var host = location.hostname;
      var expire = name + '=; expires=Thu, 01 Jan 1970 00:00:00 GMT';
      doc.cookie = expire + '; path=/';
      doc.cookie = expire;
      doc.cookie = expire + '; path=/; domain=' + host;
      doc.cookie = expire + '; path=/; domain=.' + host.split('.').slice(-2).join('.');
    }
  };
  run();
  win.addEventListener('beforeunload', run);
});

function storageItem(which, key, value) {
  var v = allowedValue(value);
  if (!key || v === null) return;
  try {
    var s = win[which];
    if (v === undefined || value === '$remove$') s.removeItem(key);
    else s.setItem(key, v);
  } catch (e) {}
}
def('set-local-storage-item', function(key, value) { storageItem('localStorage', key, value); });
def('set-session-storage-item', function(key, value) { storageItem('sessionStorage', key, value); });

// ---------------------------------------------------------------- response rewriting

// Wrap fetch() so matching responses' text goes through `edit` (text -> text).
function editFetch(spec, edit) {
  var orig = win.fetch;
  if (typeof orig !== 'function') return;
  win.fetch = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var r = args[0];
      var url = typeof r === 'string' ? r : (r && r.url) || String(r);
      var p = safe.apply(target, self, args);
      if (!matchProps(spec, { url: url })) return p;
      return p.then(function(resp) {
        return resp.clone().text().then(function(text) {
          var out = edit(text, url);
          if (out === text) return resp;
          return new Response(out, { status: resp.status, statusText: resp.statusText, headers: resp.headers });
        }, function() { return resp; });
      });
    },
  });
}

// Make XMLHttpRequest's response text go through `edit` for matching requests.
function editXhr(spec, edit) {
  var XHR = win.XMLHttpRequest;
  if (typeof XHR !== 'function') return;
  var open = XHR.prototype.open;
  var urls = new WeakMap();
  XHR.prototype.open = function(method, url) { urls.set(this, String(url)); return safe.apply(open, this, arguments); };
  var desc = safe.getOwnPropertyDescriptor(XHR.prototype, 'responseText');
  var rdesc = safe.getOwnPropertyDescriptor(XHR.prototype, 'response');
  var cache = new WeakMap();
  var fix = function(xhr, v) {
    if (typeof v !== 'string' || !urls.has(xhr) || !matchProps(spec, { url: urls.get(xhr) })) return v;
    var c = cache.get(xhr);
    if (c && c[0] === v) return c[1];
    var out = edit(v, urls.get(xhr));
    cache.set(xhr, [v, out]);
    return out;
  };
  if (desc && desc.get) safe.defineProperty(XHR.prototype, 'responseText', { configurable: true, get: function() { return fix(this, desc.get.call(this)); } });
  if (rdesc && rdesc.get) safe.defineProperty(XHR.prototype, 'response', { configurable: true, get: function() { return fix(this, rdesc.get.call(this)); } });
}

function replacer(find, replacement) {
  var m = /^\/(.+)\/([gimsu]*)$/.exec(find || '');
  var re;
  if (m) { try { re = new safe.RegExp(m[1], m[2]); } catch (e) { return null; } }
  else if (find) re = find;
  else return null;
  var rep = replacement === undefined ? '' : String(replacement);
  return function(text) { return String(text).replace(re, rep); };
}

// HLS playlists: drop the ad segments (lines matching, with the #EXTINF line before each).
def('m3u-prune', function(needle, urlPattern) {
  var p = pattern(needle);
  if (p.empty) return;
  var edit = function(text) {
    if (!/^\s*#EXTM3U/.test(text)) return text;
    var lines = text.split(/\r?\n/);
    var out = [];
    for (var i = 0; i < lines.length; i++) {
      if (p.test(lines[i])) {
        if (out.length && /^#EXTINF/.test(out[out.length - 1])) out.pop();
        continue;
      }
      out.push(lines[i]);
    }
    return out.join('\n');
  };
  editFetch(urlPattern, edit);
  editXhr(urlPattern, edit);
});

// VAST/XML ads: remove the elements a CSS selector (or simple XPath) names from XML answers.
def('xml-prune', function(selector, needle, urlPattern) {
  if (!selector || typeof win.DOMParser !== 'function') return;
  var need = pattern(needle);
  var edit = function(text) {
    if (!/^\s*</.test(text) || !need.test(text)) return text;
    try {
      var d = new win.DOMParser().parseFromString(text, 'text/xml');
      var els = [];
      if (selector.indexOf('xpath(') === 0 && typeof d.evaluate === 'function') {
        var r = d.evaluate(selector.slice(6, -1), d, null, 7, null);
        for (var k = 0; k < r.snapshotLength; k++) els.push(r.snapshotItem(k));
      } else els = toArray(d.querySelectorAll(selector));
      if (els.length === 0) return text;
      els.forEach(function(e) { if (e.parentNode) e.parentNode.removeChild(e); });
      return new win.XMLSerializer().serializeToString(d);
    } catch (e) { return text; }
  };
  editFetch(urlPattern, edit);
  editXhr(urlPattern, edit);
});

// Links that go through a tracker: point them at their real destination.
def('href-sanitizer', function(selector, source) {
  if (!selector) return;
  source = source || 'text';
  var real = function(a) {
    if (source === 'text') return (a.textContent || '').trim();
    if (source.charAt(0) === '?') {
      try { return new URL(a.href).searchParams.get(source.slice(1)) || ''; } catch (e) { return ''; }
    }
    if (source.charAt(0) === '[' && source.charAt(source.length - 1) === ']') return a.getAttribute(source.slice(1, -1)) || '';
    return '';
  };
  watchDom(function() {
    var els = doc.querySelectorAll(selector);
    for (var i = 0; i < els.length; i++) {
      var a = els[i], v = real(a);
      if (/^https?:\/\//.test(v) && a.getAttribute('href') !== v) a.setAttribute('href', v);
    }
  }, ['href']);
});

// Tell scripts that read the computed style of an element what they expect to see (anti-adblock
// checks look at the bait element's display or height).
def('spoof-css', function(selector) {
  if (!selector || typeof win.getComputedStyle !== 'function') return;
  var spoof = {};
  for (var i = 1; i + 1 < arguments.length; i += 2) spoof[String(arguments[i]).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); })] = arguments[i + 1];
  var dash = function(k) { return k.replace(/[A-Z]/g, function(c) { return '-' + c.toLowerCase(); }); };
  win.getComputedStyle = new safe.Proxy(win.getComputedStyle, {
    apply: function(target, self, args) {
      var cs = safe.apply(target, self, args);
      var el = args[0];
      var hit = false;
      try { hit = el && el.matches && el.matches(selector); } catch (e) {}
      if (!hit) return cs;
      return new safe.Proxy(cs, {
        get: function(t, prop) {
          if (prop === 'getPropertyValue') return function(name) { var k = String(name).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); }); return k in spoof ? spoof[k] : t.getPropertyValue(name); };
          if (typeof prop === 'string' && prop in spoof) return spoof[prop];
          var v = t[prop];
          return typeof v === 'function' ? v.bind(t) : v;
        },
      });
    },
  });
  var gbcr = win.Element && win.Element.prototype.getBoundingClientRect;
  if (typeof gbcr === 'function' && ('height' in spoof || 'width' in spoof)) {
    win.Element.prototype.getBoundingClientRect = function() {
      var r = gbcr.call(this);
      var hit = false;
      try { hit = this.matches(selector); } catch (e) {}
      if (!hit) return r;
      var w = 'width' in spoof ? parseFloat(spoof.width) : r.width, h = 'height' in spoof ? parseFloat(spoof.height) : r.height;
      return { x: r.x, y: r.y, top: r.top, left: r.left, width: w, height: h, right: r.left + w, bottom: r.top + h, toJSON: function() { return this; } };
    };
  }
  void dash;
});

def('prevent-innerHTML', function(selector, needle) {
  var p = pattern(needle);
  var proto = win.Element && win.Element.prototype;
  var d = proto && safe.getOwnPropertyDescriptor(proto, 'innerHTML');
  if (!d || !d.set) return;
  safe.defineProperty(proto, 'innerHTML', {
    configurable: true,
    get: d.get,
    set: function(v) {
      var hit = true;
      if (selector) { try { hit = this.matches(selector); } catch (e) { hit = false; } }
      if (hit && p.test(v)) return;
      d.set.call(this, v);
    },
  });
});

// ---------------------------------------------------------------- trusted scriptlets
// Only uBlock Origin's own lists and your own rules may use these (checked before they get here).

// Any JSON value (or the usual constants).
function anyValue(v) {
  var c = constant(v);
  if (c) return c;
  try { return { v: safe.JSON_parse(v) }; } catch (e) {}
  return { v: v };
}

def('trusted-set-constant trusted-set', function(chain, value) {
  if (!chain) return;
  var v = anyValue(value).v;
  trapChain(chain, function(owner, prop) {
    var d = safe.getOwnPropertyDescriptor(owner, prop);
    if (d && d.configurable === false) return null;
    return { configurable: false, enumerable: true, get: function() { return v; }, set: function() {} };
  });
});

def('trusted-replace-fetch-response trusted-rpfr', function(find, replacement, spec) {
  var f = replacer(find, replacement);
  if (f) editFetch(spec, f);
});
def('trusted-replace-xhr-response', function(find, replacement, spec) {
  var f = replacer(find, replacement);
  if (f) editXhr(spec, f);
});

// Text nodes (and scripts added later) whose text matches; inline scripts are edited by the
// browser itself before they run.
def('trusted-replace-node-text trusted-rpnt replace-node-text rpnt', function(nodeName, find, replacement) {
  var n = pattern(nodeName);
  var f = replacer(find, replacement);
  if (!f) return;
  var opts = extra(arguments, 3);
  var cond = pattern(opts.condition);
  watchDom(function() {
    var els = doc.querySelectorAll('*');
    for (var i = 0; i < els.length; i++) {
      var el = els[i];
      var name = el.nodeName.toLowerCase();
      if (name === 'script' || !n.test(name) || el.childElementCount !== 0) continue;
      var t = el.textContent;
      if (!cond.test(t)) continue;
      var nt = f(t);
      if (nt !== t) el.textContent = nt;
    }
  });
});

def('trusted-set-cookie', function(name, value, offset, path) {
  if (!name) return;
  var v = String(value === undefined ? '' : value).replace('$now$', String(Date.now())).replace('$currentDate$', new Date().toUTCString());
  var c = encodeURIComponent(name) + '=' + encodeURIComponent(v);
  var secs = Number(offset);
  if (secs > 0) c += '; expires=' + new Date(Date.now() + secs * 1000).toUTCString();
  c += '; path=' + (path && path !== 'none' ? path : '/');
  try { doc.cookie = c; } catch (e) {}
});

function trustedStorage(which, key, value) {
  if (!key) return;
  try {
    if (value === '$remove$') win[which].removeItem(key);
    else win[which].setItem(key, String(value === undefined ? '' : value).replace('$now$', String(Date.now())));
  } catch (e) {}
}
def('trusted-set-local-storage-item', function(key, value) { trustedStorage('localStorage', key, value); });
def('trusted-set-session-storage-item', function(key, value) { trustedStorage('sessionStorage', key, value); });

// Click elements as they appear, in order (cookie consent "reject" buttons).
def('trusted-click-element', function(selectors, extraMatch, delay) {
  if (!selectors) return;
  var list = String(selectors).split(/\s*,\s*/).filter(Boolean);
  var wait = Math.max(0, Math.min(10000, Number(delay) || 0));
  var start = Date.now();
  var next = 0;
  var tick = function() {
    if (next >= list.length || Date.now() - start > 10000) return true;
    var el = null;
    try { el = doc.querySelector(list[next]); } catch (e) { next = list.length; return true; }
    if (!el) return false;
    try { el.click(); } catch (e) {}
    next++;
    return next >= list.length;
  };
  onReady(function() {
    safe.setTimeout.call(win, function poll() {
      if (!tick()) safe.setTimeout.call(win, poll, 200);
    }, wait);
  });
  void extraMatch;
});

// Change an argument a function is called with.
def('trusted-replace-argument', function(path, argpos, argval) {
  if (!path) return;
  var parts = path.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  if (!owner || typeof owner[prop] !== 'function') return;
  var pos = Number(argpos) || 0;
  var v = anyValue(argval).v;
  var opts = extra(arguments, 3);
  var cond = pattern(opts.condition);
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var k = pos >= 0 ? pos : args.length + pos;
      if (k >= 0 && k < args.length && cond.test(args[k])) args[k] = v;
      return safe.apply(t, self, args);
    },
  });
});

// Strings a function returns, edited.
def('trusted-replace-outbound-text', function(path, find, replacement) {
  if (!path) return;
  var parts = path.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  var f = replacer(find, replacement);
  if (!owner || typeof owner[prop] !== 'function' || !f) return;
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var r = safe.apply(t, self, args);
      return typeof r === 'string' ? f(r) : r;
    },
  });
});

// Prune an object argument before a function gets it.
def('trusted-prune-inbound-object', function(path, argpos, paths, needs) {
  if (!path || !paths) return;
  var parts = path.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  if (!owner || typeof owner[prop] !== 'function') return;
  var a = pruneArgs(paths, needs);
  var pos = Number(argpos) || 0;
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var k = pos >= 0 ? pos : args.length + pos;
      var o = args[k];
      if (o && typeof o === 'object') {
        var copy;
        try { copy = safe.JSON_parse(safe.JSON_stringify(o)); } catch (e) { copy = null; }
        if (copy && prune(copy, a.paths, a.needs)) args[k] = copy;
      }
      return safe.apply(t, self, args);
    },
  });
});

// ---------------------------------------------------------------- JSONPath-style edits

// uBlock Origin's object queries: steps `.key`, `..key` (at any depth), `[*]`, `[n]`,
// `["key"]` and filters `[?.a.b]` (exists), `[?.a.b==x]`, `!=`, `*=` (contains), `^=`
// (starts), `$=` (ends), `=/re/flags`; then an edit: `+={json}` (merge), `=value` (set:
// JSON, else text), `=repl({"regex":"…","replacement":"…"})`, or none (remove). Returns true
// if the object changed.
function jsonEdit(root, query) {
  var q = String(query || '');
  var i = 0;
  var steps = [];
  var readQuoted = function() {
    var quote = q[i], out = '';
    i++;
    while (i < q.length && q[i] !== quote) { if (q[i] === '\\') i++; out += q[i]; i++; }
    i++;
    return out;
  };
  var readName = function() {
    var out = '';
    while (i < q.length && !/[.\[=+!*^$]/.test(q[i])) out += q[i++];
    return out;
  };
  // The bracket's contents, up to its matching `]` (quotes and nested brackets skipped).
  var readBracket = function() {
    var depth = 0, out = '';
    for (; i < q.length; i++) {
      var c = q[i];
      if (c === '"' || c === "'") { var start = i; readQuoted(); out += q.slice(start, i); i--; continue; }
      if (c === '[') { depth++; if (depth === 1) continue; }
      if (c === ']') { depth--; if (depth === 0) { i++; return out; } }
      out += c;
    }
    return null;
  };
  while (i < q.length) {
    var c = q[i];
    if (c === '.' && q[i + 1] === '.') { i += 2; steps.push({ deep: readName() }); }
    else if (c === '.') { i++; if (q[i] === '[') continue; var n = readName(); if (n) steps.push({ key: n }); }
    else if (c === '[') {
      var inner = readBracket();
      if (inner === null) return false;
      if (inner[0] === '?') steps.push({ filter: inner.slice(1) });
      else if (inner === '*') steps.push({ all: true });
      else if (/^-?\d+$/.test(inner)) steps.push({ key: Number(inner) });
      else if (/^["']/.test(inner)) steps.push({ key: inner.slice(1, -1) });
      else steps.push({ key: inner });
    } else break;
  }
  // The edit at the end.
  var rest = q.slice(i), op = null, arg = '';
  if (rest.indexOf('+=') === 0) { op = 'merge'; arg = rest.slice(2); }
  else if (rest[0] === '=') { op = 'set'; arg = rest.slice(1); }
  else if (rest !== '') return false;
  var parseValue = function(t) {
    try { return safe.JSON_parse(t); } catch (e) { return t; }
  };
  // Does `v` pass the filter `.a.b op value`?
  var test = function(v, f) {
    var m = /^((?:\.[^.=!*^$]+)+)\s*(?:(==|!=|\*=|\^=|\$=|=)(.*))?$/.exec(f.trim());
    if (!m) return false;
    var path = m[1].slice(1).split('.');
    var x = v;
    for (var k = 0; k < path.length; k++) {
      if (x === null || typeof x !== 'object' || !(path[k] in x)) return false;
      x = x[path[k]];
    }
    if (!m[2]) return true;
    var want = m[3].trim();
    if (m[2] === '=') { var re = /^\/(.*)\/([a-z]*)$/.exec(want); return !!re && new RegExp(re[1], re[2]).test(String(x)); }
    var w = parseValue(want);
    var sx = typeof x === 'string' ? x : safe.JSON_stringify(x);
    var sw = typeof w === 'string' ? w : safe.JSON_stringify(w);
    switch (m[2]) {
      case '==': return sx === sw;
      case '!=': return sx !== sw;
      case '*=': return sx.indexOf(sw) !== -1;
      case '^=': return sx.indexOf(sw) === 0;
      default: return sx.length >= sw.length && sx.slice(sx.length - sw.length) === sw;
    }
  };
  // Nodes: {o: parent, k: key} (the root has no parent).
  var nodes = [{ o: null, k: null, v: root }];
  var children = function(n) {
    var out = [];
    if (n.v !== null && typeof n.v === 'object') for (var k in n.v) if (Object.prototype.hasOwnProperty.call(n.v, k)) out.push({ o: n.v, k: k, v: n.v[k] });
    return out;
  };
  for (var s = 0; s < steps.length; s++) {
    var st = steps[s], next = [];
    nodes.forEach(function(n) {
      if (st.filter !== undefined) { if (test(n.v, st.filter)) next.push(n); }
      else if (st.all) next.push.apply(next, children(n));
      else if (st.deep !== undefined) {
        var walk = function(m, depth) {
          if (depth > 64) return;
          children(m).forEach(function(c) { if (c.k === st.deep) next.push(c); walk(c, depth + 1); });
        };
        walk(n, 0);
      } else if (n.v !== null && typeof n.v === 'object' && Object.prototype.hasOwnProperty.call(n.v, st.key)) next.push({ o: n.v, k: st.key, v: n.v[st.key] });
    });
    nodes = next;
    if (nodes.length === 0) return false;
  }
  var changed = false;
  nodes.forEach(function(n) {
    if (op === 'merge') {
      var add = parseValue(arg);
      if (n.v && typeof n.v === 'object' && add && typeof add === 'object') { for (var k in add) n.v[k] = add[k]; changed = true; }
    } else if (op === 'set') {
      if (n.o === null) return;
      var r = /^repl\((.*)\)$/.exec(arg);
      if (r) {
        var spec = parseValue(r[1]);
        if (typeof n.v === 'string' && spec && spec.regex !== undefined) { n.o[n.k] = n.v.replace(new RegExp(spec.regex, spec.flags || 'g'), spec.replacement || ''); changed = true; }
      } else { n.o[n.k] = parseValue(arg); changed = true; }
    } else if (n.o !== null) {
      if (Array.isArray(n.o)) n.o.splice(n.k, 1); else delete n.o[n.k];
      changed = true;
    }
  });
  return changed;
}

// Edit an object argument (JSONPath-style query, see jsonEdit) before a function gets it.
def('trusted-edit-inbound-object', function(path, argpos, query) {
  if (!path || !query) return;
  var parts = path.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  if (!owner || typeof owner[prop] !== 'function') return;
  var pos = Number(argpos) || 0;
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var k = pos >= 0 ? pos : args.length + pos;
      var o = args[k];
      if (o && typeof o === 'object') {
        var copy;
        try { copy = safe.JSON_parse(safe.JSON_stringify(o)); } catch (e) { copy = null; }
        if (copy && jsonEdit(copy, query)) args[k] = copy;
      }
      return safe.apply(t, self, args);
    },
  });
});

// Don't let a native method run when its arguments match.
def('trusted-suppress-native-method', function(path, signature, how) {
  if (!path) return;
  var parts = path.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  if (!owner || typeof owner[prop] !== 'function') return;
  var sigs = String(signature || '').split('|').map(function(x) { return x === '' ? null : pattern(x); });
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var hit = sigs.every(function(p, k) {
        if (!p) return true;
        var a = args[k];
        var text = typeof a === 'string' ? a : (a && a.nodeName) ? a.nodeName.toLowerCase() : safe.JSON_stringify(a) || String(a);
        return p.test(text);
      });
      if (!hit) return safe.apply(t, self, args);
      if (how === 'abort') throw new ReferenceError('blocked');
      return undefined;
    },
  });
});

// Pages get an unpatched fetch() (and the like) from a fresh iframe's window: give the iframe
// this window's patched one as soon as it's added.
def('trusted-prevent-dom-bypass', function(methodPath, targetProp) {
  if (!methodPath) return;
  var parts = methodPath.split('.');
  var owner = win;
  for (var i = 0; i < parts.length - 1; i++) { owner = owner && owner[parts[i]]; }
  var prop = parts[parts.length - 1];
  if (!owner || typeof owner[prop] !== 'function') return;
  owner[prop] = new safe.Proxy(owner[prop], {
    apply: function(t, self, args) {
      var r = safe.apply(t, self, args);
      var nodes = [];
      for (var k = 0; k < args.length; k++) {
        var n = args[k];
        if (n && n.nodeType === 1) {
          if (n.nodeName === 'IFRAME') nodes.push(n);
          if (n.querySelectorAll) nodes = nodes.concat(toArray(n.querySelectorAll('iframe')));
        }
      }
      nodes.forEach(function(f) {
        try {
          var w = f.contentWindow;
          if (!w) return;
          if (targetProp) {
            safe.defineProperty(w, targetProp, { value: win[targetProp], configurable: true, writable: true });
          } else {
            w.fetch = win.fetch; w.XMLHttpRequest = win.XMLHttpRequest; w.setTimeout = win.setTimeout; w.setInterval = win.setInterval;
          }
        } catch (e) {}
      });
      return r;
    },
  });
});

// Put markup the filter gives into an element (replacement content), for `duration` ms if set.
def('trusted-create-html', function(selector, html, duration) {
  if (!selector || !html) return;
  onReady(function() {
    var parent = doc.querySelector(selector);
    if (!parent) return;
    var box = doc.createElement('div');
    box.innerHTML = html;
    var added = toArray(box.childNodes);
    added.forEach(function(n) { parent.appendChild(n); });
    var ms = Number(duration);
    if (ms > 0) safe.setTimeout.call(win, function() { added.forEach(function(n) { if (n.parentNode) n.parentNode.removeChild(n); }); }, ms);
  });
});

def('trusted-prevent-fetch', function(spec, body) {
  var orig = win.fetch;
  if (typeof orig !== 'function') return;
  win.fetch = new safe.Proxy(orig, {
    apply: function(target, self, args) {
      var r = args[0], o = args[1] || {};
      var url = typeof r === 'string' ? r : (r && r.url) || String(r);
      if (!matchProps(spec, { url: url, method: o.method || 'GET' })) return safe.apply(target, self, args);
      return Promise.resolve(new Response(body === undefined ? '' : String(body), { status: 200, statusText: 'OK' }));
    },
  });
});
def('trusted-prevent-xhr', function(spec, body) {
  preventXhr(spec, body === undefined ? '' : String(body));
});

// ---------------------------------------------------------------- run the calls

for (var i = 0; i < calls.length; i++) {
  var call = calls[i];
  var name = String(call[0]).replace(/\.js$/, '');
  var f = scriptlets[name] || scriptlets[aliases[name]];
  if (typeof f !== 'function') continue;
  try { f.apply(null, call.slice(1)); } catch (e) { log(e); }
}
