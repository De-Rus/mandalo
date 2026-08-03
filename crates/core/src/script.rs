use crate::error::{CoreError, CoreResult};
use rquickjs::{CatchResultExt, CaughtError, Context, Runtime, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRequest {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScriptResponse {
    pub status: u16,
    pub status_text: String,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub duration_ms: u128,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScriptContext {
    #[serde(default)]
    pub vars: BTreeMap<String, String>,
    #[serde(default)]
    pub request_name: String,
    pub request: ScriptRequest,
    #[serde(default)]
    pub response: Option<ScriptResponse>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScriptTest {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScriptOutcome {
    pub var_sets: BTreeMap<String, String>,
    pub var_unsets: Vec<String>,
    pub tests: Vec<ScriptTest>,
    pub logs: Vec<String>,
    pub request_patch: Option<ScriptRequest>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub memory_bytes: usize,
    pub timeout_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            memory_bytes: 32 * 1024 * 1024,
            timeout_ms: 2000,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptInput<'a> {
    vars: &'a BTreeMap<String, String>,
    request_name: &'a str,
    event_name: &'a str,
    request: &'a ScriptRequest,
    response: &'a Option<ScriptResponse>,
}

const BLOCKED_GLOBALS: &[(&str, &str)] = &[
    (
        "require",
        "there is no module loader; scripts cannot import code",
    ),
    ("module", "there is no module system"),
    ("exports", "there is no module system"),
    ("process", "scripts have no access to the host process"),
    ("fetch", "scripts cannot make network requests"),
    ("XMLHttpRequest", "scripts cannot make network requests"),
    ("WebSocket", "scripts cannot make network requests"),
    (
        "setTimeout",
        "scripts must be synchronous; there are no timers",
    ),
    (
        "setInterval",
        "scripts must be synchronous; there are no timers",
    ),
    (
        "setImmediate",
        "scripts must be synchronous; there are no timers",
    ),
    (
        "queueMicrotask",
        "scripts must be synchronous; there are no timers",
    ),
    ("Deno", "scripts have no host runtime access"),
    ("Bun", "scripts have no host runtime access"),
    ("window", "scripts do not run in a browser"),
    ("document", "scripts do not run in a browser"),
    ("localStorage", "scripts have no persistent storage"),
];

fn explain(message: &str) -> String {
    for (name, why) in BLOCKED_GLOBALS {
        if message.contains(&format!("{name} is not defined")) {
            return format!("{name} is not available in Mándalo scripts: {why}");
        }
    }
    message.to_string()
}

const PRELUDE: &str = r#"
(function () {
  var input = JSON.parse(__ctx_json);
  delete globalThis.__ctx_json;

  var store = {};
  for (var k in input.vars) { store[k] = input.vars[k]; }
  var setOps = {};
  var unsetOps = {};
  var logs = [];
  var tests = [];

  function isFatal(e) {
    var m = e && e.message ? String(e.message) : String(e);
    return /interrupted|out of memory|stack overflow|Maximum call stack/.test(m);
  }

  function display(v) {
    if (typeof v === 'string') return v;
    if (v === undefined) return 'undefined';
    if (v === null) return 'null';
    if (typeof v === 'function') return '[Function]';
    if (typeof v === 'object') {
      if (typeof v.valueOf === 'function' && typeof v.valueOf() === 'string') return v.valueOf();
      try { return JSON.stringify(v); } catch (e) { return String(v); }
    }
    return String(v);
  }

  function repr(v) {
    if (typeof v === 'string') return JSON.stringify(v);
    return display(v);
  }

  function typeName(v) {
    if (v === null) return 'null';
    if (Array.isArray(v)) return 'array';
    return typeof v;
  }

  function deepEqual(a, b) {
    if (a === b) return true;
    if (typeof a !== typeof b) return false;
    if (a === null || b === null) return false;
    if (typeof a !== 'object') return a !== a && b !== b;
    if (Array.isArray(a) !== Array.isArray(b)) return false;
    var ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (var i = 0; i < ka.length; i++) {
      if (!Object.prototype.hasOwnProperty.call(b, ka[i])) return false;
      if (!deepEqual(a[ka[i]], b[ka[i]])) return false;
    }
    return true;
  }

  function toStored(v) {
    if (typeof v === 'string') return v;
    if (v === null || v === undefined) return '';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }

  function requireName(name, where) {
    if (typeof name !== 'string') throw new Error(where + ' requires a string name');
    return name;
  }

  var PASSTHROUGH = {
    toString: 1, valueOf: 1, toJSON: 1, constructor: 1, inspect: 1, then: 1,
    length: 1, name: 1, hasOwnProperty: 1, nodeType: 1, Symbol: 1
  };
  function guard(target, label, hint) {
    return new Proxy(target, {
      get: function (obj, prop) {
        if (typeof prop === 'symbol' || prop in obj || PASSTHROUGH[prop]) return obj[prop];
        throw new Error(label + '.' + String(prop) + ' is not available in Mándalo scripts: ' + hint);
      }
    });
  }

  var B64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = function (text) {
    var s = String(text), out = '', i = 0;
    while (i < s.length) {
      var c1 = s.charCodeAt(i++), c2 = s.charCodeAt(i++), c3 = s.charCodeAt(i++);
      if (c1 > 255 || c2 > 255 || c3 > 255) throw new Error('btoa expects latin1 text');
      out += B64.charAt(c1 >> 2);
      out += B64.charAt(((c1 & 3) << 4) | (isNaN(c2) ? 0 : c2 >> 4));
      out += isNaN(c2) ? '=' : B64.charAt(((c2 & 15) << 2) | (isNaN(c3) ? 0 : c3 >> 6));
      out += isNaN(c3) ? '=' : B64.charAt(c3 & 63);
    }
    return out;
  };
  globalThis.atob = function (text) {
    var s = String(text).replace(/=+$/, ''), out = '', bits = 0, acc = 0;
    for (var i = 0; i < s.length; i++) {
      var v = B64.indexOf(s.charAt(i));
      if (v === -1) throw new Error('atob expects base64 text');
      acc = (acc << 6) | v;
      bits += 6;
      if (bits >= 8) { bits -= 8; out += String.fromCharCode((acc >> bits) & 255); }
    }
    return out;
  };

  function rnd(n) { return Math.floor(Math.random() * n); }
  function pick(list) { return list[rnd(list.length)]; }
  var FIRST = ['Ada', 'Nova', 'Iris', 'Leo', 'Mila', 'Otto', 'Sara', 'Theo'];
  var LAST = ['Vega', 'Rossi', 'Ivanov', 'Kaur', 'Nakamura', 'Silva', 'Dubois', 'Okafor'];
  var WORDS = ['alpha', 'bravo', 'delta', 'ember', 'flint', 'gamma', 'harbor', 'ivory'];
  var CITIES = ['Lisbon', 'Osaka', 'Bogota', 'Tallinn', 'Nairobi', 'Quito'];
  var COUNTRIES = ['Portugal', 'Japan', 'Colombia', 'Estonia', 'Kenya', 'Ecuador'];
  var COLORS = ['red', 'blue', 'green', 'amber', 'violet', 'teal'];
  var JOBS = ['Engineer', 'Analyst', 'Designer', 'Architect', 'Technician'];
  var PRODUCTS = ['Keyboard', 'Lamp', 'Bicycle', 'Notebook', 'Kettle'];
  var COMPANIES = ['Northwind', 'Contoso', 'Umbra', 'Lumen', 'Meridian'];

  function hex(n) {
    var out = '';
    for (var i = 0; i < n; i++) { out += '0123456789abcdef'.charAt(rnd(16)); }
    return out;
  }
  function uuid() {
    return hex(8) + '-' + hex(4) + '-4' + hex(3) + '-' +
      '89ab'.charAt(rnd(4)) + hex(3) + '-' + hex(12);
  }
  var DYNAMIC = {
    $guid: uuid,
    $randomUUID: uuid,
    $timestamp: function () { return String(Math.floor(Date.now() / 1000)); },
    $isoTimestamp: function () { return new Date().toISOString(); },
    $randomInt: function () { return String(rnd(1001)); },
    $randomBoolean: function () { return rnd(2) === 0 ? 'false' : 'true'; },
    $randomAlphaNumeric: function () { return 'abcdefghijklmnopqrstuvwxyz0123456789'.charAt(rnd(36)); },
    $randomWord: function () { return pick(WORDS); },
    $randomWords: function () { return pick(WORDS) + ' ' + pick(WORDS) + ' ' + pick(WORDS); },
    $randomFirstName: function () { return pick(FIRST); },
    $randomLastName: function () { return pick(LAST); },
    $randomFullName: function () { return pick(FIRST) + ' ' + pick(LAST); },
    $randomUserName: function () { return pick(FIRST).toLowerCase() + '.' + pick(LAST).toLowerCase(); },
    $randomEmail: function () { return pick(FIRST).toLowerCase() + '.' + pick(LAST).toLowerCase() + '@example.com'; },
    $randomPassword: function () { return hex(12); },
    $randomPhoneNumber: function () { return '5' + hex(2).replace(/[a-f]/g, '4') + '-' + rnd(900) + '-' + rnd(9000); },
    $randomCity: function () { return pick(CITIES); },
    $randomCountry: function () { return pick(COUNTRIES); },
    $randomStreetAddress: function () { return rnd(400) + ' ' + pick(WORDS) + ' Street'; },
    $randomCompanyName: function () { return pick(COMPANIES); },
    $randomJobTitle: function () { return pick(JOBS); },
    $randomProductName: function () { return pick(PRODUCTS); },
    $randomDomainName: function () { return pick(COMPANIES).toLowerCase() + '.com'; },
    $randomUrl: function () { return 'https://' + pick(COMPANIES).toLowerCase() + '.com/' + pick(WORDS); },
    $randomIP: function () { return rnd(256) + '.' + rnd(256) + '.' + rnd(256) + '.' + rnd(256); },
    $randomColor: function () { return pick(COLORS); },
    $randomHexColor: function () { return '#' + hex(6); },
    $randomDatePast: function () { return new Date(Date.now() - rnd(31536000000)).toISOString(); },
    $randomDateFuture: function () { return new Date(Date.now() + rnd(31536000000)).toISOString(); }
  };
  function dynamicValue(name) {
    if (Object.prototype.hasOwnProperty.call(DYNAMIC, name)) return DYNAMIC[name]();
    throw new Error('{{' + name + '}} is not available in Mándalo scripts: that Postman dynamic variable has no generator here — supported ones are ' + Object.keys(DYNAMIC).join(', '));
  }

  var varsApi = {
    get: function (k) {
      requireName(k, 'get');
      return Object.prototype.hasOwnProperty.call(store, k) ? store[k] : undefined;
    },
    set: function (k, v) {
      requireName(k, 'set');
      var s = toStored(v);
      store[k] = s;
      setOps[k] = s;
      delete unsetOps[k];
    },
    unset: function (k) {
      requireName(k, 'unset');
      delete store[k];
      delete setOps[k];
      unsetOps[k] = true;
    },
    has: function (k) {
      requireName(k, 'has');
      return Object.prototype.hasOwnProperty.call(store, k);
    },
    clear: function () {
      for (var k in store) { varsApi.unset(k); }
    },
    toObject: function () {
      var out = {};
      for (var k in store) { out[k] = store[k]; }
      return out;
    },
    replaceIn: function (template) {
      if (typeof template !== 'string') throw new Error('replaceIn requires a string');
      return template.replace(/\{\{\s*([^}\s]+)\s*\}\}/g, function (whole, name) {
        if (Object.prototype.hasOwnProperty.call(store, name)) return store[name];
        if (name.charAt(0) === '$') return dynamicValue(name);
        return whole;
      });
    }
  };

  function logLine(args) {
    var parts = [];
    for (var i = 0; i < args.length; i++) { parts.push(display(args[i])); }
    logs.push(parts.join(' '));
  }
  globalThis.console = {
    log: function () { logLine(arguments); },
    info: function () { logLine(arguments); },
    debug: function () { logLine(arguments); },
    warn: function () { logLine(arguments); },
    error: function () { logLine(arguments); }
  };

  var reqHeaders = [];
  for (var i = 0; i < input.request.headers.length; i++) {
    reqHeaders.push({ key: input.request.headers[i][0], value: input.request.headers[i][1] });
  }
  function headerIndex(name) {
    var lower = requireName(name, 'headers lookup').toLowerCase();
    for (var i = 0; i < reqHeaders.length; i++) {
      if (String(reqHeaders[i].key).toLowerCase() === lower) return i;
    }
    return -1;
  }
  function headerPair(h) {
    if (!h || typeof h !== 'object' || typeof h.key !== 'string') {
      throw new Error('pm.request.headers expects a { key, value } object');
    }
    return { key: h.key, value: h.value === undefined || h.value === null ? '' : String(h.value) };
  }
  var requestHeadersApi = {
    add: function (h) { reqHeaders.push(headerPair(h)); },
    upsert: function (h) {
      var pair = headerPair(h);
      var at = headerIndex(pair.key);
      if (at === -1) reqHeaders.push(pair); else reqHeaders[at] = pair;
    },
    remove: function (name) {
      var at = headerIndex(name);
      if (at !== -1) reqHeaders.splice(at, 1);
    },
    has: function (name) { return headerIndex(name) !== -1; },
    get: function (name) {
      var at = headerIndex(name);
      return at === -1 ? undefined : reqHeaders[at].value;
    },
    all: function () { return reqHeaders.slice(); },
    each: function (fn) {
      if (typeof fn !== 'function') throw new Error('pm.request.headers.each requires a function');
      for (var i = 0; i < reqHeaders.length; i++) { fn(reqHeaders[i]); }
    },
    count: function () { return reqHeaders.length; }
  };

  function splitUrl(text) {
    var raw = String(text);
    var hash = '';
    var at = raw.indexOf('#');
    if (at !== -1) { hash = raw.slice(at); raw = raw.slice(0, at); }
    var q = raw.indexOf('?');
    var base = q === -1 ? raw : raw.slice(0, q);
    var params = [];
    if (q !== -1) {
      var pieces = raw.slice(q + 1).split('&');
      for (var i = 0; i < pieces.length; i++) {
        if (pieces[i] === '') continue;
        var eq = pieces[i].indexOf('=');
        params.push(eq === -1
          ? { key: pieces[i], value: null }
          : { key: pieces[i].slice(0, eq), value: pieces[i].slice(eq + 1) });
      }
    }
    return { base: base, params: params, hash: hash };
  }

  function makeUrl(text) {
    var parts = splitUrl(text);
    function render() {
      var out = parts.base;
      var pairs = [];
      for (var i = 0; i < parts.params.length; i++) {
        var p = parts.params[i];
        pairs.push(p.value === null || p.value === undefined ? String(p.key) : String(p.key) + '=' + String(p.value));
      }
      if (pairs.length) out += '?' + pairs.join('&');
      return out + parts.hash;
    }
    function indexOfKey(name) {
      for (var i = 0; i < parts.params.length; i++) {
        if (String(parts.params[i].key) === String(name)) return i;
      }
      return -1;
    }
    var query = guard({
      add: function (pair) {
        if (typeof pair === 'string') { parts.params.push({ key: pair, value: null }); return; }
        if (!pair || typeof pair !== 'object' || typeof pair.key !== 'string') {
          throw new Error('pm.request.url.query.add expects a { key, value } object');
        }
        parts.params.push({ key: pair.key, value: pair.value === undefined ? null : String(pair.value) });
      },
      remove: function (name) {
        var at = indexOfKey(typeof name === 'object' && name ? name.key : name);
        if (at !== -1) parts.params.splice(at, 1);
      },
      upsert: function (pair) {
        var at = indexOfKey(pair && pair.key);
        if (at === -1) query.add(pair); else parts.params[at] = { key: pair.key, value: String(pair.value) };
      },
      has: function (name) { return indexOfKey(name) !== -1; },
      get: function (name) {
        var at = indexOfKey(name);
        return at === -1 ? undefined : parts.params[at].value;
      },
      all: function () { return parts.params.slice(); },
      count: function () { return parts.params.length; },
      clear: function () { parts.params = []; },
      each: function (fn) {
        if (typeof fn !== 'function') throw new Error('pm.request.url.query.each requires a function');
        for (var i = 0; i < parts.params.length; i++) { fn(parts.params[i]); }
      },
      toObject: function () {
        var out = {};
        for (var i = 0; i < parts.params.length; i++) { out[parts.params[i].key] = parts.params[i].value; }
        return out;
      }
    }, 'pm.request.url.query', 'the query list supports add, upsert, remove, get, has, all, count, clear, each and toObject');

    var scheme = /^([a-z][a-z0-9+.-]*):\/\//i.exec(parts.base);
    var afterScheme = scheme ? parts.base.slice(scheme[0].length) : parts.base;
    var slash = afterScheme.indexOf('/');
    var hostText = slash === -1 ? afterScheme : afterScheme.slice(0, slash);
    var pathText = slash === -1 ? '' : afterScheme.slice(slash + 1);

    return guard({
      query: query,
      protocol: scheme ? scheme[1] : undefined,
      host: hostText ? hostText.split('.') : [],
      path: pathText ? pathText.split('/') : [],
      getHost: function () { return hostText; },
      getPath: function () { return slash === -1 ? '/' : '/' + pathText; },
      addQueryParams: function (list) {
        var items = Array.isArray(list) ? list : [list];
        for (var i = 0; i < items.length; i++) { query.add(items[i]); }
      },
      removeQueryParams: function (list) {
        var items = Array.isArray(list) ? list : [list];
        for (var i = 0; i < items.length; i++) { query.remove(items[i]); }
      },
      toString: render,
      valueOf: render,
      toJSON: render
    }, 'pm.request.url', 'pm.request.url is a Postman URL object, not a string — call pm.request.url.toString() before using string methods');
  }

  function makeBody(text) {
    if (text === null || text === undefined) return undefined;
    var value = String(text);
    return guard({
      mode: 'raw',
      raw: value,
      toString: function () { return value; },
      valueOf: function () { return value; },
      toJSON: function () { return value; }
    }, 'pm.request.body', 'the imported body is raw text — read pm.request.body.raw, or assign a new string to pm.request.body');
  }

  var urlValue = makeUrl(input.request.url);
  var bodyValue = makeBody(input.request.body);
  var request = {
    method: input.request.method,
    headers: requestHeadersApi
  };
  Object.defineProperty(request, 'url', {
    get: function () { return urlValue; },
    set: function (v) { urlValue = makeUrl(v); },
    configurable: true,
    enumerable: true
  });
  Object.defineProperty(request, 'body', {
    get: function () { return bodyValue; },
    set: function (v) { bodyValue = makeBody(v); },
    configurable: true,
    enumerable: true
  });

  function snapshot() {
    var headers = [];
    for (var i = 0; i < reqHeaders.length; i++) {
      headers.push([String(reqHeaders[i].key), String(reqHeaders[i].value)]);
    }
    return {
      method: String(request.method),
      url: String(request.url),
      headers: headers,
      body: request.body === undefined || request.body === null ? null : String(request.body)
    };
  }
  var initial = JSON.stringify(snapshot());

  function assertFail(message) { throw new Error(message); }

  function chain(value, flags) {
    flags = flags || {};
    var neg = !!flags.neg, deepMode = !!flags.deep, anyMode = !!flags.any;
    var self = {};
    var proxied;
    function check(ok, what) {
      if (neg ? ok : !ok) {
        assertFail('expected ' + repr(value) + (neg ? ' not ' : ' ') + what);
      }
    }
    var words = ['to', 'be', 'been', 'is', 'that', 'which', 'and', 'has', 'have', 'with', 'at', 'of', 'same', 'does', 'but', 'still', 'also', 'all', 'own', 'ordered', 'itself'];
    for (var i = 0; i < words.length; i++) {
      Object.defineProperty(self, words[i], { get: function () { return proxied; }, configurable: true });
    }
    Object.defineProperty(self, 'not', { get: function () { return chain(value, { neg: !neg, deep: deepMode, any: anyMode }); }, configurable: true });
    Object.defineProperty(self, 'deep', { get: function () { return chain(value, { neg: neg, deep: true, any: anyMode }); }, configurable: true });
    Object.defineProperty(self, 'any', { get: function () { return chain(value, { neg: neg, deep: deepMode, any: true }); }, configurable: true });
    Object.defineProperty(self, 'exist', { get: function () { check(value !== null && value !== undefined, 'to exist'); }, configurable: true });
    Object.defineProperty(self, 'finite', { get: function () { check(typeof value === 'number' && isFinite(value), 'to be a finite number'); }, configurable: true });
    Object.defineProperty(self, 'NaN', { get: function () { check(typeof value === 'number' && value !== value, 'to be NaN'); }, configurable: true });
    Object.defineProperty(self, 'true', { get: function () { check(value === true, 'to be true'); }, configurable: true });
    Object.defineProperty(self, 'false', { get: function () { check(value === false, 'to be false'); }, configurable: true });
    Object.defineProperty(self, 'null', { get: function () { check(value === null, 'to be null'); }, configurable: true });
    Object.defineProperty(self, 'undefined', { get: function () { check(value === undefined, 'to be undefined'); }, configurable: true });
    Object.defineProperty(self, 'ok', { get: function () { check(!!value, 'to be truthy'); }, configurable: true });
    Object.defineProperty(self, 'empty', {
      get: function () {
        var len = value === null || value === undefined ? -1
          : (typeof value === 'string' || Array.isArray(value)) ? value.length : Object.keys(value).length;
        check(len === 0, 'to be empty');
      },
      configurable: true
    });

    self.equal = function (other) {
      if (deepMode) { check(deepEqual(value, other), 'to deeply equal ' + repr(other)); return proxied; }
      check(value === other, 'to equal ' + repr(other));
      return proxied;
    };
    self.equals = self.equal;
    self.eq = self.equal;
    self.eql = function (other) { check(deepEqual(value, other), 'to deeply equal ' + repr(other)); return proxied; };
    self.eqls = self.eql;
    self.a = function (type) {
      requireName(type, 'expect().to.be.a');
      check(typeName(value) === type.toLowerCase(), 'to be a ' + type + ' but got ' + typeName(value));
      return proxied;
    };
    self.an = self.a;
    self.above = function (n) {
      if (typeof n !== 'number') throw new Error('expect().to.be.above requires a number');
      check(value > n, 'to be above ' + n);
      return proxied;
    };
    self.greaterThan = self.above;
    self.below = function (n) {
      if (typeof n !== 'number') throw new Error('expect().to.be.below requires a number');
      check(value < n, 'to be below ' + n);
      return proxied;
    };
    self.lessThan = self.below;
    self.least = function (n) {
      if (typeof n !== 'number') throw new Error('expect().to.be.at.least requires a number');
      check(value >= n, 'to be at least ' + n);
      return proxied;
    };
    self.most = function (n) {
      if (typeof n !== 'number') throw new Error('expect().to.be.at.most requires a number');
      check(value <= n, 'to be at most ' + n);
      return proxied;
    };
    self.include = function (needle) {
      var ok;
      if (typeof value === 'string') {
        ok = value.indexOf(String(needle)) !== -1;
      } else if (Array.isArray(value)) {
        ok = false;
        for (var j = 0; j < value.length; j++) { if (deepEqual(value[j], needle)) { ok = true; break; } }
      } else if (value !== null && typeof value === 'object' && needle !== null && typeof needle === 'object') {
        ok = true;
        var keys = Object.keys(needle);
        for (var m = 0; m < keys.length; m++) {
          if (!deepEqual(value[keys[m]], needle[keys[m]])) { ok = false; break; }
        }
      } else {
        throw new Error('expect().to.include needs a string, array or object target');
      }
      check(ok, 'to include ' + repr(needle));
      return proxied;
    };
    self.contain = self.include;
    self.contains = self.include;
    self.includes = self.include;
    self.property = function (name, expected) {
      requireName(name, 'expect().to.have.property');
      var present = value !== null && value !== undefined && Object.prototype.hasOwnProperty.call(Object(value), name);
      if (arguments.length < 2) {
        check(present, 'to have property ' + repr(name));
        return chain(present ? value[name] : undefined, { neg: false });
      }
      check(present && deepEqual(value[name], expected), 'to have property ' + repr(name) + ' equal to ' + repr(expected));
      return chain(present ? value[name] : undefined, { neg: false });
    };
    self.lengthOf = function (n) {
      if (typeof n !== 'number') throw new Error('expect().to.have.lengthOf requires a number');
      if (value === null || value === undefined || typeof value.length !== 'number') {
        throw new Error('expect().to.have.lengthOf needs a target with a length');
      }
      check(value.length === n, 'to have length ' + n + ' but got ' + value.length);
      return proxied;
    };
    self.length = self.lengthOf;
    self.match = function (re) {
      if (!(re instanceof RegExp)) throw new Error('expect().to.match requires a regular expression');
      check(re.test(String(value)), 'to match ' + String(re));
      return proxied;
    };
    self.matches = self.match;
    self.oneOf = function (list) {
      if (!Array.isArray(list)) throw new Error('expect().to.be.oneOf requires an array');
      var found = false;
      for (var q = 0; q < list.length; q++) { if (deepEqual(list[q], value)) { found = true; break; } }
      check(found, 'to be one of ' + repr(list));
      return proxied;
    };
    self.within = function (low, high) {
      if (typeof low !== 'number' || typeof high !== 'number') {
        throw new Error('expect().to.be.within requires two numbers');
      }
      check(value >= low && value <= high, 'to be within ' + low + ' and ' + high);
      return proxied;
    };
    self.closeTo = function (n, delta) {
      if (typeof n !== 'number' || typeof delta !== 'number') {
        throw new Error('expect().to.be.closeTo requires a number and a delta');
      }
      check(typeof value === 'number' && Math.abs(value - n) <= delta, 'to be close to ' + n + ' ± ' + delta);
      return proxied;
    };
    self.string = function (needle) {
      if (typeof value !== 'string') throw new Error('expect().to.have.string needs a string target');
      check(value.indexOf(String(needle)) !== -1, 'to contain the string ' + repr(needle));
      return proxied;
    };
    self.keys = function () {
      var wanted = Array.isArray(arguments[0]) ? arguments[0] : Array.prototype.slice.call(arguments);
      if (value === null || value === undefined || typeof value !== 'object') {
        throw new Error('expect().to.have.keys needs an object or array target');
      }
      var actual = Object.keys(value);
      var hits = 0;
      for (var k = 0; k < wanted.length; k++) {
        if (actual.indexOf(String(wanted[k])) !== -1) hits++;
      }
      var ok = anyMode ? hits > 0 : hits === wanted.length && actual.length === wanted.length;
      check(ok, (anyMode ? 'to have any of the keys ' : 'to have exactly the keys ') + repr(wanted) + ' but had ' + repr(actual));
      return proxied;
    };
    self.key = self.keys;
    self.members = function (list) {
      if (!Array.isArray(list)) throw new Error('expect().to.have.members requires an array');
      if (!Array.isArray(value)) throw new Error('expect().to.have.members needs an array target');
      function covers(a, b) {
        for (var x = 0; x < b.length; x++) {
          var found = false;
          for (var y = 0; y < a.length; y++) { if (deepEqual(a[y], b[x])) { found = true; break; } }
          if (!found) return false;
        }
        return true;
      }
      check(covers(value, list) && covers(list, value), 'to have members ' + repr(list));
      return proxied;
    };
    self.satisfy = function (fn) {
      if (typeof fn !== 'function') throw new Error('expect().to.satisfy requires a function');
      check(!!fn(value), 'to satisfy the given function');
      return proxied;
    };
    self.satisfies = self.satisfy;
    self.throw = function () { throw new Error('expect().to.throw is not available in Mándalo scripts: assert on values instead'); };
    proxied = guard(self, 'pm.expect(...)', 'that chai assertion is not implemented — supported ones are equal, eql, deep.equal, a/an, above, below, least, most, within, closeTo, include, string, property, keys, members, lengthOf, match, oneOf, satisfy, exist, empty, ok, true, false, null, undefined, finite, NaN and their .not forms');
    return proxied;
  }

  function expect(value) { return chain(value, { neg: false }); }
  expect.fail = function (message) {
    assertFail(message === undefined ? 'expect.fail() was called' : String(message));
  };

  var response = null;
  if (input.response) {
    var raw = input.response;
    function responseHeaderIndex(name) {
      var lower = requireName(name, 'pm.response.headers.get').toLowerCase();
      for (var i = 0; i < raw.headers.length; i++) {
        if (String(raw.headers[i][0]).toLowerCase() === lower) return i;
      }
      return -1;
    }
    function contentType() {
      var at = responseHeaderIndex('content-type');
      return at === -1 ? '' : String(raw.headers[at][1]).toLowerCase();
    }
    function parsedBody() {
      try { return { ok: true, value: JSON.parse(raw.body) }; }
      catch (e) { return { ok: false, error: e && e.message ? String(e.message) : String(e) }; }
    }
    function pathLookup(root, path) {
      var parts = String(path).replace(/\[(\d+)\]/g, '.$1').split('.');
      var cursor = root;
      for (var i = 0; i < parts.length; i++) {
        if (cursor === null || cursor === undefined) return { found: false };
        if (!Object.prototype.hasOwnProperty.call(Object(cursor), parts[i])) return { found: false };
        cursor = cursor[parts[i]];
      }
      return { found: true, value: cursor };
    }
    var STATUS_RANGES = {
      ok: [200, 300, 'to be ok'],
      success: [200, 300, 'to be a success'],
      info: [100, 200, 'to be informational'],
      redirection: [300, 400, 'to be a redirection'],
      clientError: [400, 500, 'to be a client error'],
      serverError: [500, 600, 'to be a server error'],
      error: [400, 600, 'to be an error']
    };
    var STATUS_CODES = {
      accepted: [202, 'to be accepted'],
      badRequest: [400, 'to be a bad request'],
      unauthorized: [401, 'to be unauthorized'],
      forbidden: [403, 'to be forbidden'],
      notFound: [404, 'to be not found'],
      rateLimited: [429, 'to be rate limited']
    };
    function responseAssertions(neg) {
      function check(ok, what) {
        if (neg ? ok : !ok) {
          assertFail('expected response ' + (neg ? 'not ' : '') + what + ' but got ' + raw.status + ' ' + raw.statusText);
        }
      }
      var node;
      var have = {
        status: function (wanted) {
          if (typeof wanted === 'number') { check(raw.status === wanted, 'to have status ' + wanted); }
          else if (typeof wanted === 'string') { check(raw.statusText === wanted, 'to have status ' + JSON.stringify(wanted)); }
          else { throw new Error('pm.response.to.have.status requires a number or status text'); }
          return node;
        },
        header: function (name, value) {
          var at = responseHeaderIndex(name);
          if (arguments.length < 2) { check(at !== -1, 'to have header ' + repr(name)); return node; }
          check(at !== -1 && String(raw.headers[at][1]) === String(value), 'to have header ' + repr(name) + ' equal to ' + repr(value));
          return node;
        },
        body: function (expected) {
          if (arguments.length === 0) { check(raw.body.length > 0, 'to have a body'); }
          else if (expected instanceof RegExp) { check(expected.test(raw.body), 'to have a body matching ' + String(expected)); }
          else { check(raw.body === String(expected), 'to have body ' + repr(expected)); }
          return node;
        },
        jsonBody: function (first, second) {
          var parsed = parsedBody();
          if (!parsed.ok) { check(false, 'to have a JSON body (it did not parse: ' + parsed.error + ')'); return node; }
          if (arguments.length === 0) { check(true, 'to have a JSON body'); return node; }
          if (arguments.length === 1) {
            if (typeof first === 'string') {
              check(pathLookup(parsed.value, first).found, 'to have the JSON body path ' + repr(first));
            } else {
              check(deepEqual(parsed.value, first), 'to have the JSON body ' + repr(first));
            }
            return node;
          }
          var hit = pathLookup(parsed.value, first);
          check(hit.found && deepEqual(hit.value, second), 'to have ' + repr(first) + ' equal to ' + repr(second) + ' in its JSON body');
          return node;
        },
        jsonSchema: function () {
          throw new Error('pm.response.to.have.jsonSchema is not available in Mándalo scripts: no JSON-schema validator is bundled — assert on the fields you care about with pm.expect');
        }
      };
      var be = {};
      function defineStatus(name, test, what) {
        Object.defineProperty(be, name, {
          get: function () { check(test(), what); },
          configurable: true
        });
      }
      Object.keys(STATUS_RANGES).forEach(function (name) {
        var spec = STATUS_RANGES[name];
        defineStatus(name, function () { return raw.status >= spec[0] && raw.status < spec[1]; }, spec[2]);
      });
      Object.keys(STATUS_CODES).forEach(function (name) {
        var spec = STATUS_CODES[name];
        defineStatus(name, function () { return raw.status === spec[0]; }, spec[1]);
      });
      defineStatus('withBody', function () { return raw.body.length > 0; }, 'to have a body');
      defineStatus('json', function () { return parsedBody().ok; }, 'to be JSON');
      defineStatus('html', function () { return contentType().indexOf('html') !== -1; }, 'to be HTML');
      defineStatus('xml', function () { return contentType().indexOf('xml') !== -1; }, 'to be XML');

      node = {
        have: guard(have, 'pm.response.to.have', 'supported response assertions are status, header, body, jsonBody and jsonSchema'),
        be: guard(be, 'pm.response.to.be', 'supported response states are ' + Object.keys(STATUS_RANGES).concat(Object.keys(STATUS_CODES)).concat(['withBody', 'json', 'html', 'xml']).join(', '))
      };
      Object.defineProperty(node, 'not', { get: function () { return responseAssertions(!neg); }, configurable: true });
      node = guard(node, 'pm.response.to', 'a response assertion starts with .have, .be or .not');
      return node;
    }
    response = {
      code: raw.status,
      status: raw.statusText,
      responseTime: raw.durationMs,
      responseSize: raw.body.length,
      headers: {
        get: function (name) {
          var at = responseHeaderIndex(name);
          return at === -1 ? undefined : raw.headers[at][1];
        },
        has: function (name) { return responseHeaderIndex(name) !== -1; },
        all: function () { return raw.headers.map(function (h) { return { key: h[0], value: h[1] }; }); }
      },
      text: function () { return raw.body; },
      reason: function () { return raw.statusText; },
      size: function () {
        var header = 0;
        for (var i = 0; i < raw.headers.length; i++) {
          header += String(raw.headers[i][0]).length + String(raw.headers[i][1]).length + 4;
        }
        return { body: raw.body.length, header: header, total: raw.body.length + header };
      },
      json: function () {
        try { return JSON.parse(raw.body); }
        catch (e) { throw new Error('response body is not valid JSON: ' + (e && e.message ? e.message : String(e))); }
      },
      to: responseAssertions(false)
    };
  }

  function test(name, fn) {
    requireName(name, 'pm.test');
    if (typeof fn !== 'function') throw new Error('pm.test requires a callback function');
    if (fn.length > 0) {
      throw new Error("pm.test's done() callback is not available in Mándalo scripts: tests run synchronously — drop the done parameter and assert inline");
    }
    try {
      fn();
      tests.push({ name: name, passed: true, error: null });
    } catch (e) {
      if (isFatal(e)) throw e;
      tests.push({ name: name, passed: false, error: e && e.message ? String(e.message) : String(e) });
    }
  }

  function unavailable(what, why) {
    return function () { throw new Error(what + ' is not available in Mándalo scripts: ' + why); };
  }

  var pm = {
    info: {
      requestName: input.requestName,
      requestId: uuid(),
      eventName: input.eventName,
      iteration: 0,
      iterationCount: 1
    },
    environment: varsApi,
    globals: varsApi,
    variables: varsApi,
    collectionVariables: varsApi,
    request: request,
    response: response,
    test: test,
    expect: expect,
    sendRequest: unavailable('pm.sendRequest', 'scripts cannot make network requests — chain a real request in the collection and capture what you need from its response'),
    setNextRequest: unavailable('pm.setNextRequest', 'run order is the collection order; branching is not implemented')
  };
  var blockedMembers = [
    ['execution', 'pm.execution', 'run-flow control (setNextRequest, skipRequest, location) is not implemented — requests run in collection order'],
    ['cookies', 'pm.cookies', 'there is no cookie jar; send cookies as an explicit Cookie header'],
    ['visualizer', 'pm.visualizer', 'response visualizations are not implemented'],
    ['iterationData', 'pm.iterationData', 'there is no data-file runner (newman -d); pass the value as an environment variable instead'],
    ['vault', 'pm.vault', 'the Postman vault is not implemented; use a Mándalo secret'],
    ['require', 'pm.require', 'there is no module loader; scripts cannot import packages']
  ];
  for (var b = 0; b < blockedMembers.length; b++) {
    (function (entry) {
      Object.defineProperty(pm, entry[0], {
        get: unavailable(entry[1], entry[2]),
        configurable: true
      });
    })(blockedMembers[b]);
  }

  var legacyGlobals = [
    ['CryptoJS', 'crypto-js is not bundled; compute the signature outside the script and pass it in as a variable'],
    ['_', 'lodash is not bundled; plain JavaScript array and object methods are available'],
    ['moment', 'moment is not bundled; use the built-in Date object'],
    ['xml2Json', 'the XML helper is not bundled; assert on the raw text with pm.response.text()'],
    ['tv4', 'no JSON-schema validator is bundled; assert on the fields you care about with pm.expect'],
    ['Ajv', 'no JSON-schema validator is bundled; assert on the fields you care about with pm.expect'],
    ['Buffer', 'the node Buffer is not available; use btoa/atob for base64']
  ];
  for (var g = 0; g < legacyGlobals.length; g++) {
    (function (entry) {
      Object.defineProperty(globalThis, entry[0], {
        get: unavailable(entry[0], entry[1]),
        set: function () {},
        configurable: true
      });
    })(legacyGlobals[g]);
  }

  Object.defineProperty(String.prototype, 'has', {
    value: function (needle) { return String(this).indexOf(String(needle)) !== -1; },
    configurable: true,
    writable: true
  });

  globalThis.tests = {};
  globalThis.postman = {
    setEnvironmentVariable: function (k, v) { varsApi.set(k, v); },
    getEnvironmentVariable: function (k) { return varsApi.get(k); },
    clearEnvironmentVariable: function (k) { varsApi.unset(k); },
    setGlobalVariable: function (k, v) { varsApi.set(k, v); },
    getGlobalVariable: function (k) { return varsApi.get(k); },
    clearGlobalVariable: function (k) { varsApi.unset(k); },
    getResponseHeader: function (name) { return response ? response.headers.get(name) : undefined; },
    setNextRequest: unavailable('postman.setNextRequest', 'run order is the collection order; branching is not implemented'),
    getResponseCookie: unavailable('postman.getResponseCookie', 'there is no cookie jar; read the Set-Cookie header instead')
  };
  if (input.response) {
    globalThis.responseBody = input.response.body;
    globalThis.responseTime = input.response.durationMs;
    globalThis.responseCode = {
      code: input.response.status,
      name: input.response.statusText,
      detail: input.response.statusText
    };
    globalThis.responseHeaders = {};
    for (var h = 0; h < input.response.headers.length; h++) {
      globalThis.responseHeaders[input.response.headers[h][0]] = input.response.headers[h][1];
    }
  }

  globalThis.pm = pm;
  globalThis.__mandalo = {
    outcome: function () {
      var patch = null;
      if (!input.response) {
        var now = snapshot();
        if (JSON.stringify(now) !== initial) patch = now;
      }
      var legacy = globalThis.tests;
      if (legacy && typeof legacy === 'object') {
        var names = Object.keys(legacy);
        for (var i = 0; i < names.length; i++) {
          tests.push({
            name: names[i],
            passed: !!legacy[names[i]],
            error: legacy[names[i]] ? null : 'the legacy tests["' + names[i] + '"] assertion was falsy'
          });
        }
      }
      return JSON.stringify({
        varSets: setOps,
        varUnsets: Object.keys(unsetOps),
        tests: tests,
        logs: logs,
        requestPatch: patch
      });
    }
  };
})();
"#;

pub fn run_script(source: &str, ctx: ScriptContext, limits: Limits) -> CoreResult<ScriptOutcome> {
    let event_name = if ctx.response.is_some() {
        "test"
    } else {
        "prerequest"
    };
    let input = serde_json::to_string(&ScriptInput {
        vars: &ctx.vars,
        request_name: &ctx.request_name,
        event_name,
        request: &ctx.request,
        response: &ctx.response,
    })
    .map_err(|e| CoreError::Script(format!("script context is not serializable: {e}")))?;

    let runtime = Runtime::new()
        .map_err(|e| CoreError::Script(format!("script runtime failed to start: {e}")))?;
    runtime.set_memory_limit(limits.memory_bytes);
    runtime.set_max_stack_size(256 * 1024);
    let deadline = Instant::now() + Duration::from_millis(limits.timeout_ms);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));

    let context = Context::full(&runtime)
        .map_err(|e| CoreError::Script(format!("script context failed to start: {e}")))?;

    let describe = |err: CaughtError| -> CoreError {
        if Instant::now() >= deadline {
            return CoreError::ScriptTimeout(format!("script exceeded {}ms", limits.timeout_ms));
        }
        match err {
            CaughtError::Value(v) if v.is_null() || v.is_undefined() => CoreError::Script(format!(
                "script exceeded the memory limit of {} bytes",
                limits.memory_bytes
            )),
            other => CoreError::Script(explain(other.to_string().trim())),
        }
    };

    let prefixed = |prefix: &str, err: CoreError| -> CoreError {
        let message = format!("{prefix}: {}", err.message());
        match err {
            CoreError::ScriptTimeout(_) => CoreError::ScriptTimeout(message),
            _ => CoreError::Script(message),
        }
    };

    context.with(|ctx_js| {
        ctx_js
            .globals()
            .set("__ctx_json", input)
            .map_err(|e| CoreError::Script(format!("script context injection failed: {e}")))?;

        ctx_js
            .eval::<Value, _>(PRELUDE)
            .catch(&ctx_js)
            .map_err(|e| prefixed("script engine failed to initialise", describe(e)))?;

        let mut options = rquickjs::context::EvalOptions::default();
        options.global = true;
        options.strict = false;
        options.promise = false;
        options.filename = Some("script.js".to_string());
        ctx_js
            .eval_with_options::<Value, _>(source.to_string(), options)
            .catch(&ctx_js)
            .map_err(describe)?;

        let json = ctx_js
            .eval::<String, _>("__mandalo.outcome()")
            .catch(&ctx_js)
            .map_err(|e| prefixed("script results could not be read", describe(e)))?;

        serde_json::from_str::<ScriptOutcome>(&json)
            .map_err(|e| CoreError::Script(format!("script results could not be parsed: {e}")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ScriptContext {
        ScriptContext {
            vars: BTreeMap::new(),
            request_name: "Get user".to_string(),
            request: ScriptRequest {
                method: "GET".to_string(),
                url: "https://x.dev/users/1".to_string(),
                headers: vec![("Accept".to_string(), "application/json".to_string())],
                body: None,
            },
            response: None,
        }
    }

    fn with_response() -> ScriptContext {
        let mut c = ctx();
        c.response = Some(ScriptResponse {
            status: 200,
            status_text: "OK".to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("X-Trace".to_string(), "abc".to_string()),
            ],
            body: r#"{"token":"t0k","count":3,"nested":{"a":1},"list":[1,2,3]}"#.to_string(),
            duration_ms: 42,
        });
        c
    }

    fn run(source: &str, c: ScriptContext) -> ScriptOutcome {
        run_script(source, c, Limits::default()).unwrap()
    }

    fn fail(source: &str, c: ScriptContext) -> String {
        run_script(source, c, Limits::default())
            .unwrap_err()
            .to_string()
    }

    fn tested(source: &str) -> ScriptTest {
        let out = run(source, with_response());
        assert_eq!(out.tests.len(), 1, "expected exactly one test");
        out.tests[0].clone()
    }

    fn passes(source: &str) -> bool {
        tested(source).passed
    }

    #[test]
    fn environment_set_surfaces_in_var_sets() {
        let out = run(r#"pm.environment.set("token", "abc");"#, ctx());
        assert_eq!(out.var_sets.get("token").unwrap(), "abc");
        assert!(out.var_unsets.is_empty());
    }

    #[test]
    fn environment_get_reads_incoming_vars() {
        let mut c = ctx();
        c.vars
            .insert("base".to_string(), "https://x.dev".to_string());
        let out = run(r#"console.log(pm.environment.get("base"));"#, c);
        assert_eq!(out.logs, vec!["https://x.dev"]);
    }

    #[test]
    fn environment_get_missing_is_undefined() {
        let out = run(r#"console.log(pm.environment.get("nope"));"#, ctx());
        assert_eq!(out.logs, vec!["undefined"]);
    }

    #[test]
    fn environment_unset_surfaces_in_var_unsets() {
        let mut c = ctx();
        c.vars.insert("token".to_string(), "old".to_string());
        let out = run(r#"pm.environment.unset("token");"#, c);
        assert_eq!(out.var_unsets, vec!["token".to_string()]);
        assert!(out.var_sets.is_empty());
    }

    #[test]
    fn set_then_unset_reports_only_the_unset() {
        let out = run(
            r#"pm.environment.set("a","1"); pm.environment.unset("a");"#,
            ctx(),
        );
        assert!(out.var_sets.is_empty());
        assert_eq!(out.var_unsets, vec!["a".to_string()]);
    }

    #[test]
    fn unset_then_set_reports_only_the_set() {
        let mut c = ctx();
        c.vars.insert("a".to_string(), "0".to_string());
        let out = run(
            r#"pm.environment.unset("a"); pm.environment.set("a","2");"#,
            c,
        );
        assert_eq!(out.var_sets.get("a").unwrap(), "2");
        assert!(out.var_unsets.is_empty());
    }

    #[test]
    fn environment_has_and_to_object() {
        let mut c = ctx();
        c.vars.insert("a".to_string(), "1".to_string());
        let out = run(
            r#"console.log(pm.environment.has("a"), pm.environment.has("b"), JSON.stringify(pm.environment.toObject()));"#,
            c,
        );
        assert_eq!(out.logs, vec![r#"true false {"a":"1"}"#]);
    }

    #[test]
    fn variables_and_collection_variables_share_the_store() {
        let out = run(
            r#"pm.collectionVariables.set("k","v"); console.log(pm.variables.get("k"));"#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["v"]);
        assert_eq!(out.var_sets.get("k").unwrap(), "v");
    }

    #[test]
    fn non_string_values_are_stored_as_text() {
        let out = run(
            r#"pm.environment.set("n", 5); pm.environment.set("o", {a:1});"#,
            ctx(),
        );
        assert_eq!(out.var_sets.get("n").unwrap(), "5");
        assert_eq!(out.var_sets.get("o").unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn request_url_mutation_surfaces_in_patch() {
        let out = run(r#"pm.request.url = "https://y.dev/other";"#, ctx());
        let patch = out.request_patch.unwrap();
        assert_eq!(patch.url, "https://y.dev/other");
        assert_eq!(patch.method, "GET");
    }

    #[test]
    fn request_method_and_body_mutation_surface() {
        let out = run(
            r#"pm.request.method = "POST"; pm.request.body = "{\"a\":1}";"#,
            ctx(),
        );
        let patch = out.request_patch.unwrap();
        assert_eq!(patch.method, "POST");
        assert_eq!(patch.body.unwrap(), r#"{"a":1}"#);
    }

    #[test]
    fn untouched_request_produces_no_patch() {
        let out = run(r#"console.log(pm.request.url);"#, ctx());
        assert!(out.request_patch.is_none());
        assert_eq!(out.logs, vec!["https://x.dev/users/1"]);
    }

    #[test]
    fn header_add_upsert_remove_get() {
        let out = run(
            r#"
            pm.request.headers.add({ key: "X-A", value: "1" });
            pm.request.headers.upsert({ key: "accept", value: "text/plain" });
            pm.request.headers.remove("X-A");
            pm.request.headers.add({ key: "X-B", value: 2 });
            console.log(pm.request.headers.get("ACCEPT"));
            console.log(pm.request.headers.get("missing"));
            "#,
            ctx(),
        );
        let patch = out.request_patch.unwrap();
        assert_eq!(
            patch.headers,
            vec![
                ("accept".to_string(), "text/plain".to_string()),
                ("X-B".to_string(), "2".to_string()),
            ]
        );
        assert_eq!(out.logs, vec!["text/plain", "undefined"]);
    }

    #[test]
    fn header_add_without_key_fails_loud() {
        let err = fail(r#"pm.request.headers.add("X-A: 1");"#, ctx());
        assert!(err.contains("{ key, value }"), "{err}");
    }

    #[test]
    fn request_mutations_in_post_script_are_not_patched() {
        let out = run(
            r#"pm.request.url = "https://y.dev"; pm.request.headers.add({key:"X",value:"1"});"#,
            with_response(),
        );
        assert!(out.request_patch.is_none());
    }

    #[test]
    fn response_fields_are_exposed() {
        let out = run(
            r#"console.log(pm.response.code, pm.response.status, pm.response.responseTime);"#,
            with_response(),
        );
        assert_eq!(out.logs, vec!["200 OK 42"]);
    }

    #[test]
    fn response_text_and_json() {
        let out = run(
            r#"console.log(pm.response.json().token, pm.response.text().length);"#,
            with_response(),
        );
        assert_eq!(out.logs, vec!["t0k 57"]);
    }

    #[test]
    fn response_json_on_invalid_body_throws() {
        let mut c = with_response();
        c.response.as_mut().unwrap().body = "not json".to_string();
        let err = fail(r#"pm.response.json();"#, c);
        assert!(err.contains("not valid JSON"), "{err}");
    }

    #[test]
    fn response_json_error_is_catchable() {
        let mut c = with_response();
        c.response.as_mut().unwrap().body = "<html>".to_string();
        let out = run(
            r#"try { pm.response.json(); } catch (e) { console.log("caught"); }"#,
            c,
        );
        assert_eq!(out.logs, vec!["caught"]);
    }

    #[test]
    fn response_headers_get_is_case_insensitive() {
        let out = run(
            r#"console.log(pm.response.headers.get("content-TYPE"), pm.response.headers.get("nope"));"#,
            with_response(),
        );
        assert_eq!(out.logs, vec!["application/json undefined"]);
    }

    #[test]
    fn response_to_have_status_and_be_ok() {
        assert!(passes(
            r#"pm.test("t", function(){ pm.response.to.have.status(200); pm.response.to.be.ok; });"#
        ));
        let failed = tested(r#"pm.test("t", function(){ pm.response.to.have.status(404); });"#);
        assert!(!failed.passed);
        assert!(failed.error.unwrap().contains("to have status 404"));
    }

    #[test]
    fn response_to_have_status_accepts_status_text() {
        assert!(passes(
            r#"pm.test("t", function(){ pm.response.to.have.status("OK"); });"#
        ));
    }

    #[test]
    fn response_to_not_have_status() {
        assert!(passes(
            r#"pm.test("t", function(){ pm.response.to.not.have.status(500); });"#
        ));
    }

    #[test]
    fn pm_test_records_pass() {
        let t = tested(r#"pm.test("all good", function(){ pm.expect(1).to.equal(1); });"#);
        assert_eq!(t.name, "all good");
        assert!(t.passed);
        assert!(t.error.is_none());
    }

    #[test]
    fn pm_test_records_failure_without_aborting() {
        let out = run(
            r#"
            pm.test("bad", function(){ pm.expect(1).to.equal(2); });
            pm.test("good", function(){ pm.expect(1).to.equal(1); });
            console.log("after");
            "#,
            with_response(),
        );
        assert_eq!(out.tests.len(), 2);
        assert!(!out.tests[0].passed);
        assert!(out.tests[0].error.as_ref().unwrap().contains("to equal 2"));
        assert!(out.tests[1].passed);
        assert_eq!(out.logs, vec!["after"]);
    }

    #[test]
    fn pm_test_captures_a_raw_throw() {
        let t = tested(r#"pm.test("boom", function(){ throw new Error("kaboom"); });"#);
        assert!(!t.passed);
        assert_eq!(t.error.unwrap(), "kaboom");
    }

    #[test]
    fn pm_test_requires_a_function() {
        let err = fail(r#"pm.test("x", "not a function");"#, ctx());
        assert!(err.contains("callback function"), "{err}");
    }

    #[test]
    fn expect_equal_eql_and_types() {
        assert!(passes(
            r#"pm.test("t", function(){
            pm.expect("a").to.equal("a");
            pm.expect({x:[1,2]}).to.eql({x:[1,2]});
            pm.expect("s").to.be.a("string");
            pm.expect([1]).to.be.an("array");
            pm.expect(null).to.be.a("null");
        });"#
        ));
    }

    #[test]
    fn expect_eql_is_deep_not_strict() {
        let t = tested(r#"pm.test("t", function(){ pm.expect({a:1}).to.equal({a:1}); });"#);
        assert!(!t.passed);
    }

    #[test]
    fn expect_boolean_null_undefined_getters() {
        assert!(passes(
            r#"pm.test("t", function(){
            pm.expect(true).to.be.true;
            pm.expect(false).to.be.false;
            pm.expect(null).to.be.null;
            pm.expect(undefined).to.be.undefined;
        });"#
        ));
        let t = tested(r#"pm.test("t", function(){ pm.expect(1).to.be.true; });"#);
        assert!(!t.passed);
        assert!(t.error.unwrap().contains("to be true"));
    }

    #[test]
    fn expect_above_below_include_property_length_match() {
        assert!(passes(
            r#"pm.test("t", function(){
            pm.expect(5).to.be.above(4);
            pm.expect(5).to.be.below(6);
            pm.expect("hello").to.include("ell");
            pm.expect([1,2,3]).to.include(2);
            pm.expect({a:1}).to.have.property("a");
            pm.expect({a:1}).to.have.property("a", 1);
            pm.expect([1,2,3]).to.have.lengthOf(3);
            pm.expect("abc123").to.match(/[0-9]+/);
        });"#
        ));
    }

    #[test]
    fn expect_negation_flips_every_assertion() {
        assert!(passes(
            r#"pm.test("t", function(){
            pm.expect(1).to.not.equal(2);
            pm.expect("x").to.not.be.a("number");
            pm.expect([1]).to.not.include(9);
            pm.expect({a:1}).to.not.have.property("b");
            pm.expect(true).to.not.be.false;
        });"#
        ));
        let t = tested(r#"pm.test("t", function(){ pm.expect(1).to.not.equal(1); });"#);
        assert!(!t.passed);
        assert!(t.error.unwrap().contains("not to equal 1"));
    }

    #[test]
    fn expect_match_requires_a_regexp() {
        let t = tested(r#"pm.test("t", function(){ pm.expect("a").to.match("a"); });"#);
        assert!(!t.passed);
        assert!(t.error.unwrap().contains("regular expression"));
    }

    #[test]
    fn console_captures_all_levels_and_stringifies() {
        let out = run(
            r#"console.log("a", 1, true); console.warn({b:2}); console.error(null, undefined, [1,2]);"#,
            ctx(),
        );
        assert_eq!(
            out.logs,
            vec!["a 1 true", r#"{"b":2}"#, "null undefined [1,2]"]
        );
    }

    #[test]
    fn pm_info_exposes_request_and_event_name() {
        let out = run(
            r#"console.log(pm.info.requestName, pm.info.eventName);"#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["Get user prerequest"]);
        let out = run(
            r#"console.log(pm.info.requestName, pm.info.eventName);"#,
            with_response(),
        );
        assert_eq!(out.logs, vec!["Get user test"]);
    }

    #[test]
    fn pm_response_is_null_in_a_pre_script() {
        let out = run(r#"console.log(pm.response === null);"#, ctx());
        assert_eq!(out.logs, vec!["true"]);
    }

    #[test]
    fn send_request_throws_a_named_error() {
        let err = fail(r#"pm.sendRequest("https://x.dev", function(){});"#, ctx());
        assert!(err.contains("pm.sendRequest is not available"), "{err}");
        assert!(err.contains("network requests"), "{err}");
    }

    #[test]
    fn pm_execution_throws_a_named_error() {
        let err = fail(r#"pm.execution.skipRequest();"#, ctx());
        assert!(err.contains("pm.execution is not available"), "{err}");
    }

    #[test]
    fn blocked_api_errors_are_catchable() {
        let out = run(
            r#"try { pm.sendRequest("x"); } catch (e) { console.log(e.message); }"#,
            ctx(),
        );
        assert!(out.logs[0].contains("pm.sendRequest is not available"));
    }

    #[test]
    fn host_globals_are_absent() {
        let out = run(
            r#"
            var names = ["require","process","fetch","XMLHttpRequest","setTimeout","setInterval","Deno","Bun","window","document","localStorage","module","exports","__ctx_json"];
            for (var i = 0; i < names.length; i++) {
              if (typeof globalThis[names[i]] !== "undefined") console.log("LEAK:" + names[i]);
            }
            console.log("done");
            "#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["done"]);
    }

    #[test]
    fn only_pm_and_console_are_added_to_globals() {
        let out = run(
            r#"console.log(typeof pm, typeof console, typeof JSON, typeof Math, typeof Date);"#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["object object object object function"]);
    }

    #[test]
    fn calling_require_names_the_limitation() {
        let err = fail(r#"var x = require("fs");"#, ctx());
        assert!(err.contains("require is not available"), "{err}");
        assert!(err.contains("module loader"), "{err}");
    }

    #[test]
    fn calling_set_timeout_names_the_limitation() {
        let err = fail(r#"setTimeout(function(){}, 1);"#, ctx());
        assert!(err.contains("setTimeout is not available"), "{err}");
        assert!(err.contains("synchronous"), "{err}");
    }

    #[test]
    fn calling_fetch_names_the_limitation() {
        let err = fail(r#"fetch("https://x.dev");"#, ctx());
        assert!(err.contains("fetch is not available"), "{err}");
    }

    #[test]
    fn top_level_throw_is_an_error() {
        let err = fail(r#"throw new Error("nope");"#, ctx());
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn syntax_error_reports_the_line() {
        let err = fail(r#"function ("#, ctx());
        assert!(err.contains("function name expected"), "{err}");
        assert!(err.contains("script.js:1"), "{err}");
    }

    #[test]
    fn runtime_error_reports_the_line() {
        let err = fail("var a = 1;\nnope.boom();\n", ctx());
        assert!(err.contains("script.js:2"), "{err}");
    }

    #[test]
    fn a_failed_run_does_not_poison_the_next_one() {
        let err = fail(
            r#"pm.environment.set("a","1"); throw new Error("boom");"#,
            ctx(),
        );
        assert!(err.contains("boom"));
        let out = run(r#"pm.environment.set("b","2");"#, ctx());
        assert_eq!(out.var_sets.len(), 1);
        assert_eq!(out.var_sets.get("b").unwrap(), "2");
    }

    #[test]
    fn infinite_loop_is_killed_by_the_timeout() {
        let limits = Limits {
            memory_bytes: 32 * 1024 * 1024,
            timeout_ms: 200,
        };
        let started = Instant::now();
        let err = run_script("while (true) {}", ctx(), limits)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "script exceeded 200ms");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn infinite_loop_inside_a_test_is_killed_by_the_timeout() {
        let limits = Limits {
            memory_bytes: 32 * 1024 * 1024,
            timeout_ms: 200,
        };
        let started = Instant::now();
        let err = run_script(
            r#"pm.test("hang", function(){ while (true) {} });"#,
            ctx(),
            limits,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "script exceeded 200ms");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_loop_that_swallows_errors_is_still_killed() {
        let limits = Limits {
            memory_bytes: 32 * 1024 * 1024,
            timeout_ms: 200,
        };
        let started = Instant::now();
        let err = run_script(
            r#"while (true) { try { var x = 1 + 1; } catch (e) {} }"#,
            ctx(),
            limits,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, "script exceeded 200ms");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn memory_limit_is_a_clean_error() {
        let limits = Limits {
            memory_bytes: 2 * 1024 * 1024,
            timeout_ms: 5000,
        };
        let err = run_script(
            r#"var a = []; for (var i = 0; i < 5000000; i++) { a.push({ i: i, s: "padding padding padding" }); }"#,
            ctx(),
            limits,
        )
        .unwrap_err().to_string();
        assert_eq!(err, "script exceeded the memory limit of 2097152 bytes");
    }

    #[test]
    fn memory_limit_does_not_poison_the_next_run() {
        let limits = Limits {
            memory_bytes: 2 * 1024 * 1024,
            timeout_ms: 5000,
        };
        run_script(
            r#"var a = []; for (var i = 0; i < 5000000; i++) { a.push({ i: i, s: "padding padding padding" }); }"#,
            ctx(),
            limits,
        )
        .unwrap_err();
        let out = run(r#"pm.environment.set("a","1");"#, ctx());
        assert_eq!(out.var_sets.get("a").unwrap(), "1");
    }

    #[test]
    fn deep_recursion_is_a_caught_error() {
        let err = fail(r#"function f(n){ return f(n + 1); } f(0);"#, ctx());
        assert!(err.to_lowercase().contains("stack"), "{err}");
    }

    #[test]
    fn deep_recursion_inside_a_test_is_not_swallowed() {
        let err = fail(
            r#"pm.test("deep", function(){ function f(n){ return f(n+1); } f(0); });"#,
            ctx(),
        );
        assert!(err.to_lowercase().contains("stack"), "{err}");
    }

    #[test]
    fn realistic_postman_script_end_to_end() {
        let out = run(
            r#"
            pm.test("status is 200", function () { pm.response.to.have.status(200); });
            var json = pm.response.json();
            pm.environment.set("token", json.token);
            pm.test("has token", function () { pm.expect(json.token).to.be.a("string"); });
            pm.test("count above zero", function () { pm.expect(json.count).to.be.above(0); });
            pm.test("list length", function () { pm.expect(json.list).to.have.lengthOf(3); });
            console.log("token", json.token);
            "#,
            with_response(),
        );
        assert_eq!(out.tests.len(), 4);
        assert!(out.tests.iter().all(|t| t.passed));
        assert_eq!(out.var_sets.get("token").unwrap(), "t0k");
        assert_eq!(out.logs, vec!["token t0k"]);
        assert!(out.request_patch.is_none());
    }

    #[test]
    fn realistic_pre_script_end_to_end() {
        let mut c = ctx();
        c.vars.insert("token".to_string(), "abc".to_string());
        let out = run(
            r#"
            pm.request.headers.upsert({ key: "Authorization", value: "Bearer " + pm.environment.get("token") });
            pm.request.url = pm.request.url + "?trace=1";
            pm.environment.set("lastRequest", pm.info.requestName);
            "#,
            c,
        );
        let patch = out.request_patch.unwrap();
        assert_eq!(patch.url, "https://x.dev/users/1?trace=1");
        assert_eq!(
            patch.headers,
            vec![
                ("Accept".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer abc".to_string()),
            ]
        );
        assert_eq!(out.var_sets.get("lastRequest").unwrap(), "Get user");
    }

    #[test]
    fn sloppy_mode_globals_are_allowed() {
        let out = run(r#"token = "implicit"; console.log(token);"#, ctx());
        assert_eq!(out.logs, vec!["implicit"]);
    }

    #[test]
    fn empty_script_is_a_clean_empty_outcome() {
        let out = run("", ctx());
        assert_eq!(out, ScriptOutcome::default());
    }

    #[test]
    fn an_unknown_chai_assertion_names_itself_and_lists_the_real_ones() {
        let err = fail(r#"pm.expect(1).to.be.bananas;"#, with_response());
        assert!(
            err.contains("pm.expect(...).bananas is not available"),
            "{err}"
        );
        assert!(err.contains("oneOf"), "{err}");
    }

    #[test]
    fn an_unknown_response_state_names_the_supported_ones() {
        let err = fail(r#"pm.response.to.be.teapot;"#, with_response());
        assert!(
            err.contains("pm.response.to.be.teapot is not available"),
            "{err}"
        );
        assert!(err.contains("rateLimited"), "{err}");
    }

    #[test]
    fn json_schema_validation_says_why_it_is_missing() {
        let err = fail(r#"pm.response.to.have.jsonSchema({});"#, with_response());
        assert!(err.contains("no JSON-schema validator is bundled"), "{err}");
    }

    #[test]
    fn a_done_callback_test_is_rejected_by_name() {
        let err = fail(
            r#"pm.test("t", function (done) { done(); });"#,
            with_response(),
        );
        assert!(err.contains("done() callback is not available"), "{err}");
        assert!(err.contains("synchronously"), "{err}");
    }

    #[test]
    fn npm_sandbox_globals_name_themselves_instead_of_being_undefined() {
        for (source, needle) in [
            (r#"CryptoJS.MD5("a");"#, "crypto-js is not bundled"),
            (r#"_.map([], function(){});"#, "lodash is not bundled"),
            (r#"moment().format();"#, "moment is not bundled"),
            (r#"xml2Json("<a/>");"#, "the XML helper is not bundled"),
        ] {
            let err = fail(source, ctx());
            assert!(err.contains("is not available in Mándalo scripts"), "{err}");
            assert!(err.contains(needle), "{err}");
        }
    }

    #[test]
    fn the_legacy_postman_object_writes_variables() {
        let out = run(
            r#"postman.setEnvironmentVariable("a", "1"); postman.setGlobalVariable("b", 2); postman.clearEnvironmentVariable("c");"#,
            ctx(),
        );
        assert_eq!(out.var_sets.get("a").unwrap(), "1");
        assert_eq!(out.var_sets.get("b").unwrap(), "2");
        assert_eq!(out.var_unsets, vec!["c".to_string()]);
    }

    #[test]
    fn legacy_sandbox_globals_mirror_the_response() {
        let out = run(
            r#"
            tests["code"] = responseCode.code === 200;
            tests["body"] = responseBody.has("t0k");
            tests["fast"] = responseTime < 500;
            tests["header"] = responseHeaders["Content-Type"] === "application/json";
            "#,
            with_response(),
        );
        assert_eq!(out.tests.len(), 4);
        assert!(out.tests.iter().all(|t| t.passed), "{:?}", out.tests);
    }

    #[test]
    fn the_url_object_reads_and_rewrites_query_parameters() {
        let mut c = ctx();
        c.request.url = "https://x.dev/users?page=1&sort=asc".to_string();
        let out = run(
            r#"
            pm.request.url.query.remove("sort");
            pm.request.url.query.upsert({ key: "page", value: "2" });
            pm.request.url.query.add({ key: "trace", value: "on" });
            console.log(pm.request.url.getPath(), pm.request.url.query.get("page"));
            "#,
            c,
        );
        assert_eq!(
            out.request_patch.unwrap().url,
            "https://x.dev/users?page=2&trace=on"
        );
        assert_eq!(out.logs, vec!["/users 2"]);
    }

    #[test]
    fn replace_in_generates_dynamic_variables_and_keeps_unknown_names() {
        let out = run(
            r#"
            console.log(pm.variables.replaceIn("{{$guid}}").length);
            console.log(pm.variables.replaceIn("{{nope}}"));
            "#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["36", "{{nope}}"]);
    }

    #[test]
    fn base64_helpers_round_trip() {
        let out = run(
            r#"console.log(btoa("user:pass"), atob("dXNlcjpwYXNz"));"#,
            ctx(),
        );
        assert_eq!(out.logs, vec!["dXNlcjpwYXNz user:pass"]);
    }

    #[test]
    fn default_limits_are_32mb_and_2s() {
        let l = Limits::default();
        assert_eq!(l.memory_bytes, 32 * 1024 * 1024);
        assert_eq!(l.timeout_ms, 2000);
    }
}
