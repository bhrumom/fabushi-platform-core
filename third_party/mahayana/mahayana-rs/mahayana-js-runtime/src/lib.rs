//! Portable JavaScript compatibility runtime for Mahayana plugins.
//!
//! This runtime embeds QuickJS-NG through rquickjs. Rust remains the host and
//! source of truth for lifecycle, service dependency state, permissions,
//! storage and marketplace installation. The JavaScript layer implements the
//! public Cordis surface expected by DeepSeek Harness plugins.

use mahayana_plugin_runtime::{PluginState, ServiceRegistry};
use reqwest::blocking::Client;
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Context, Ctx, Error as JsError, Function, Module, Promise, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use url::Url;

const CORDIS_MODULE: &str = r#"
const Context = globalThis.__MahayanaContext;
const Service = globalThis.__MahayanaService;
const Schema = globalThis.__MahayanaSchema;
const Logger = globalThis.__MahayanaLogger;

export const symbols = {
  shadow: Symbol.for('cordis.shadow'),
  receiver: Symbol.for('cordis.receiver'),
  original: Symbol.for('cordis.original'),
  metadata: Symbol.for('cordis.metadata'),
  initHooks: Symbol.for('cordis.initHooks'),
  checkProto: Symbol.for('cordis.checkProto'),
  effect: Symbol.for('cordis.effect'),
  filter: Symbol.for('cordis.filter'),
  isolate: Symbol.for('cordis.isolate'),
  intercept: Symbol.for('cordis.intercept'),
  init: Symbol.for('cordis.init'),
  check: Symbol.for('cordis.check'),
  config: Symbol.for('cordis.config'),
  invoke: Symbol.for('cordis.invoke'),
  extend: Symbol.for('cordis.extend'),
  tracker: Symbol.for('cordis.tracker'),
  resolveConfig: Symbol.for('cordis.resolveConfig'),
};

export class CordisError extends Error {
  constructor(code, message) { super(message || code); this.name = 'CordisError'; this.code = code; }
}
CordisError.Code = { INACTIVE_EFFECT: 'cannot create effect on inactive context' };

export class ValidationError extends TypeError {
  constructor(issues = []) {
    super(Array.from(issues, issue => issue?.message || String(issue)).join('\n'));
    this.name = 'ValidationError';
  }
}

export class DisposableList {
  constructor() { this.values = []; }
  get length() { return this.values.length; }
  push(value) { this.values.push(value); return () => this.delete(value); }
  delete(value) { const i = this.values.indexOf(value); if (i < 0) return false; this.values.splice(i, 1); return true; }
  clear() { const values = [...this.values].reverse(); this.values.length = 0; return values; }
  [Symbol.iterator]() { return this.values[Symbol.iterator](); }
}

export function isObject(value) { return !!value && (typeof value === 'object' || typeof value === 'function'); }
export function isConstructor(func) {
  if (typeof func !== 'function' || !func.prototype) return false;
  const source = Function.prototype.toString.call(func);
  return /^class\s/.test(source) || Object.getOwnPropertyNames(func.prototype).length > 1;
}
export function withProps(target, props = {}) {
  return new Proxy(target, {
    get(object, key, receiver) { return key in props && key !== 'constructor' ? Reflect.get(props, key, receiver) : Reflect.get(object, key, receiver); },
    set(object, key, value, receiver) { return key in props && key !== 'constructor' ? Reflect.set(props, key, value, receiver) : Reflect.set(object, key, value, receiver); },
  });
}
export function buildOuterStack() { const outer = new Error(); return () => String(outer.stack || '').split('\n').slice(3); }
export function composeError(callback) { return callback({ offset: 1, error: new Error() }); }

export { Context, Service, Schema, Logger };
export default { Context, Service, Schema, Logger, symbols, CordisError, ValidationError, DisposableList };
"#;

const DSH_TOOLS_MODULE: &str = r#"
export function defineTool(definition) { return definition; }
export function ToolName(value) { return String(value); }
export default { defineTool, ToolName };
"#;

const DSH_LLM_MODULE: &str = r#"
export function CallId(value) { return String(value); }
export function MessageId(value) { return String(value); }
export default { CallId, MessageId };
"#;

const NODE_PATH_MODULE: &str = r#"
function parts(input) { return String(input).replaceAll('\\', '/').split('/').filter(Boolean); }
export function normalize(input) {
  const out = [];
  for (const part of parts(input)) {
    if (part === '.') continue;
    if (part === '..') { out.pop(); continue; }
    out.push(part);
  }
  return (String(input).startsWith('/') ? '/' : '') + out.join('/');
}
export function join(...items) { return normalize(items.filter(Boolean).join('/')); }
export function resolve(...items) { return join(...items); }
export function basename(input, suffix = '') {
  const value = parts(input).at(-1) || '';
  return suffix && value.endsWith(suffix) ? value.slice(0, -suffix.length) : value;
}
export function dirname(input) {
  const p = parts(input); p.pop(); return (String(input).startsWith('/') ? '/' : '') + p.join('/');
}
export function extname(input) {
  const base = basename(input); const i = base.lastIndexOf('.'); return i > 0 ? base.slice(i) : '';
}
export function isAbsolute(input) { return String(input).startsWith('/') || /^[A-Za-z]:[\\/]/.test(String(input)); }
export const sep = '/';
const path = { normalize, join, resolve, basename, dirname, extname, isAbsolute, sep };
export default path;
"#;

const NODE_EVENTS_MODULE: &str = r#"
export class EventEmitter {
  constructor() { this.listeners = new Map(); }
  on(name, fn) { const list = this.listeners.get(name) || []; list.push(fn); this.listeners.set(name, list); return this; }
  addListener(name, fn) { return this.on(name, fn); }
  once(name, fn) { const wrapped = (...args) => { this.off(name, wrapped); return fn(...args); }; return this.on(name, wrapped); }
  off(name, fn) { const list = this.listeners.get(name) || []; this.listeners.set(name, list.filter(v => v !== fn)); return this; }
  removeListener(name, fn) { return this.off(name, fn); }
  removeAllListeners(name) { if (name === undefined) this.listeners.clear(); else this.listeners.delete(name); return this; }
  emit(name, ...args) { const list = [...(this.listeners.get(name) || [])]; for (const fn of list) fn(...args); return list.length > 0; }
  listenerCount(name) { return (this.listeners.get(name) || []).length; }
}
export default EventEmitter;
"#;

const NODE_BUFFER_MODULE: &str = r#"
function utf8Encode(value) {
  const text = unescape(encodeURIComponent(String(value)));
  const out = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) out[i] = text.charCodeAt(i);
  return out;
}
function utf8Decode(bytes) {
  let s = ''; for (const byte of bytes) s += String.fromCharCode(byte);
  return decodeURIComponent(escape(s));
}
export class Buffer extends Uint8Array {
  static from(value) {
    if (typeof value === 'string') return new Buffer(utf8Encode(value));
    if (value instanceof Uint8Array || Array.isArray(value)) return new Buffer(value);
    throw new TypeError('unsupported Buffer.from input');
  }
  static alloc(size, fill = 0) { const out = new Buffer(size); out.fill(fill); return out; }
  toString(encoding = 'utf8') { if (encoding !== 'utf8' && encoding !== 'utf-8') throw new Error('only utf8 is supported'); return utf8Decode(this); }
}
export default Buffer;
"#;

const NODE_PROCESS_MODULE: &str = r#"
const process = globalThis.process;
export const env = process.env;
export const platform = process.platform;
export const arch = process.arch;
export const versions = process.versions;
export default process;
"#;

const NODE_OS_MODULE: &str = r#"
export const platform = () => globalThis.process.platform;
export const arch = () => globalThis.process.arch;
export const homedir = () => globalThis.__host_home_dir();
export const tmpdir = () => globalThis.__host_temp_dir();
export const EOL = '\n';
export default { platform, arch, homedir, tmpdir, EOL };
"#;

const NODE_FS_MODULE: &str = r#"
function unsupported() {
  throw new Error('node:fs direct access is blocked by Mahayana; use a granted file capability');
}
export const readFileSync = unsupported;
export const writeFileSync = unsupported;
export const existsSync = unsupported;
export const promises = new Proxy({}, { get() { return async () => unsupported(); } });
export default { readFileSync, writeFileSync, existsSync, promises };
"#;

const NODE_URL_MODULE: &str = r#"
export function fileURLToPath(value) {
  const source = String(value);
  if (!source.startsWith('file://')) throw new TypeError('fileURLToPath expects a file: URL');
  let path = decodeURIComponent(source.replace(/^file:\/\/(localhost)?/, ''));
  if (/^\/[A-Za-z]:\//.test(path)) path = path.slice(1);
  return path;
}
export function pathToFileURL(value) {
  let path = String(value).replaceAll('\\', '/');
  if (!path.startsWith('/')) path = '/' + path;
  return { href: 'file://' + encodeURI(path), toString() { return this.href; } };
}
export function domainToASCII(value) { return String(value).toLowerCase(); }
export function domainToUnicode(value) { return String(value); }
export const URL = globalThis.URL;
export const URLSearchParams = globalThis.URLSearchParams;
export default { fileURLToPath, pathToFileURL, domainToASCII, domainToUnicode, URL, URLSearchParams };
"#;

const NODE_UTIL_MODULE: &str = r#"
export function promisify(fn) {
  return (...args) => new Promise((resolve, reject) => fn(...args, (error, value) => error ? reject(error) : resolve(value)));
}
export function callbackify(fn) {
  return (...args) => { const callback = args.pop(); Promise.resolve(fn(...args)).then(value => callback(null, value), callback); };
}
export function inspect(value) { try { return typeof value === 'string' ? value : JSON.stringify(value); } catch { return String(value); } }
export function format(...args) { return args.map(value => typeof value === 'string' ? value : inspect(value)).join(' '); }
export const types = {
  isPromise: value => !!value && typeof value.then === 'function',
  isDate: value => value instanceof Date,
  isRegExp: value => value instanceof RegExp,
  isArrayBuffer: value => value instanceof ArrayBuffer,
  isTypedArray: value => ArrayBuffer.isView(value),
};
export default { promisify, callbackify, inspect, format, types };
"#;

const NODE_ASSERT_MODULE: &str = r#"
function fail(message = 'Assertion failed') { throw new Error(message); }
export function ok(value, message) { if (!value) fail(message); }
export function strictEqual(actual, expected, message) { if (actual !== expected) fail(message || `Expected ${actual} === ${expected}`); }
export function notStrictEqual(actual, expected, message) { if (actual === expected) fail(message || `Expected ${actual} !== ${expected}`); }
export function deepStrictEqual(actual, expected, message) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) fail(message || 'Expected values to be deeply equal');
}
export { fail };
export const strict = { ok, strictEqual, notStrictEqual, deepStrictEqual, fail };
export default Object.assign(ok, strict);
"#;

const NODE_QUERYSTRING_MODULE: &str = r#"
export function stringify(value = {}) {
  return Object.entries(value).flatMap(([key, raw]) => (Array.isArray(raw) ? raw : [raw]).map(item => `${encodeURIComponent(key)}=${encodeURIComponent(item ?? '')}`)).join('&');
}
export function parse(value = '') {
  const out = {};
  for (const part of String(value).split('&')) {
    if (!part) continue;
    const [rawKey, rawValue = ''] = part.split('=', 2);
    const key = decodeURIComponent(rawKey.replaceAll('+', ' '));
    const item = decodeURIComponent(rawValue.replaceAll('+', ' '));
    if (Object.prototype.hasOwnProperty.call(out, key)) out[key] = Array.isArray(out[key]) ? [...out[key], item] : [out[key], item];
    else out[key] = item;
  }
  return out;
}
export const encode = stringify;
export const decode = parse;
export default { stringify, parse, encode, decode };
"#;

const NODE_CRYPTO_MODULE: &str = r#"
function asString(value) {
  if (typeof value === 'string') return value;
  if (value instanceof Uint8Array || Array.isArray(value)) return Array.from(value).map(byte => String.fromCharCode(byte)).join('');
  return String(value);
}
export function randomUUID() { return __host_random_uuid(); }
export function createHash(algorithm) {
  if (String(algorithm).toLowerCase().replace('-', '') !== 'sha256') throw new Error('Mahayana crypto shim currently supports sha256');
  let chunks = '';
  return {
    update(value) { chunks += asString(value); return this; },
    digest(encoding = 'hex') {
      const hex = __host_sha256(chunks);
      if (encoding === 'hex') return hex;
      if (encoding === 'buffer') return hex;
      throw new Error(`unsupported digest encoding: ${encoding}`);
    }
  };
}
export function timingSafeEqual(left, right) {
  const a = asString(left), b = asString(right);
  if (a.length !== b.length) throw new Error('Input buffers must have the same byte length');
  let diff = 0; for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i); return diff === 0;
}
export default { randomUUID, createHash, timingSafeEqual };
"#;

const NODE_CHILD_PROCESS_MODULE: &str = r#"
function denied() { throw new Error('node:child_process is unavailable in the portable Mahayana runtime; request a desktop process capability or use MCP'); }
export const spawn = denied;
export const exec = denied;
export const execFile = denied;
export const fork = denied;
export const spawnSync = denied;
export const execSync = denied;
export const execFileSync = denied;
export default { spawn, exec, execFile, fork, spawnSync, execSync, execFileSync };
"#;

const RUNTIME_PRELUDE: &str = r#"
(() => {
  const modules = new Map();
  const plugins = new Map();
  const services = new Map();
  const serviceOwners = new Map();
  const isolatedServices = new Map();
  const isolatedOwners = new Map();
  const listeners = new Map();
  const tools = new Map();
  const timerCallbacks = new Map();
  let nextTimerId = 1;
  let childFiberUid = 1;

  function scheduleTimer(pluginId, callback, delay, repeat, args = []) {
    if (typeof callback !== 'function') throw new TypeError('timer callback must be a function');
    const ms = Math.max(0, Math.min(Number(delay) || 0, 24 * 60 * 60 * 1000));
    const id = nextTimerId++;
    timerCallbacks.set(id, { pluginId, callback, args, repeat: !!repeat });
    __host_timer_schedule(String(pluginId || 'global'), id, ms, !!repeat);
    return id;
  }

  function cancelTimer(id) {
    const numeric = Number(id);
    const existed = timerCallbacks.delete(numeric);
    __host_timer_cancel(numeric);
    return existed;
  }

  globalThis.setTimeout = (callback, delay = 0, ...args) => scheduleTimer('global', callback, delay, false, args);
  globalThis.clearTimeout = cancelTimer;
  globalThis.setInterval = (callback, delay = 0, ...args) => scheduleTimer('global', callback, delay, true, args);
  globalThis.clearInterval = cancelTimer;
  globalThis.__mahayanaFireTimer = id => {
    const timer = timerCallbacks.get(Number(id));
    if (!timer) return false;
    if (!timer.repeat) timerCallbacks.delete(Number(id));
    timer.callback(...timer.args);
    return true;
  };

  function pluginRecord(pluginId) {
    const record = plugins.get(pluginId);
    if (!record) throw new Error(`plugin ${pluginId} is not active`);
    return record;
  }

  function disposeOne(record, disposer) {
    try { return disposer(); } catch (error) { __host_log(record.id, 'error', String(error?.stack || error)); }
  }

  function trackDisposer(record, disposer, label = 'effect') {
    if (typeof disposer !== 'function') return disposer;
    let active = true;
    const tracked = () => {
      if (!active) return false;
      active = false;
      const index = record.disposers.indexOf(tracked);
      if (index >= 0) record.disposers.splice(index, 1);
      disposeOne(record, disposer);
      return true;
    };
    tracked.__effectLabel = String(label || 'effect');
    record.disposers.push(tracked);
    return tracked;
  }

  function normalizeInject(inject) {
    if (!inject) return [];
    if (Array.isArray(inject)) return inject.map(String);
    return Object.keys(inject).filter(key => inject[key] !== false);
  }

  function dependenciesAvailable(deps) {
    return normalizeInject(deps).every(name =>
      ['tools', 'storage', 'network', 'llm'].includes(name) || services.has(name)
    );
  }

  class SchemaNode {
    constructor(kind, value) { this.kind = kind; this.value = value; }
    default() { return this; } description() { return this; } required() { return this; }
    role() { return this; } hidden() { return this; } experimental() { return this; }
    min() { return this; } max() { return this; } pattern() { return this; }
  }
  const Schema = new Proxy({}, {
    get(_target, key) {
      if (key === 'is') return () => true;
      return (...args) => new SchemaNode(String(key), args);
    }
  });

  class Logger {
    constructor(name = 'plugin') { this.name = String(name); }
    debug(...args) { __host_log(this.name, 'debug', args.map(String).join(' ')); }
    info(...args) { __host_log(this.name, 'info', args.map(String).join(' ')); }
    warn(...args) { __host_log(this.name, 'warn', args.map(String).join(' ')); }
    error(...args) { __host_log(this.name, 'error', args.map(String).join(' ')); }
    success(...args) { __host_log(this.name, 'info', args.map(String).join(' ')); }
    extend(name) { return new Logger(`${this.name}:${name}`); }
  }

  class Service {
    constructor(ctx, name) {
      if (!ctx || typeof ctx.__registerService !== 'function') throw new Error('Service requires a Cordis Context');
      this.ctx = ctx;
      this.name = String(name);
      ctx.__registerService(this.name, this);
    }
  }
  Service.init = Symbol.for('cordis.init');
  Service.check = Symbol.for('cordis.check');
  Service.config = Symbol.for('cordis.config');
  Service.invoke = Symbol.for('cordis.invoke');
  Service.extend = Symbol.for('cordis.extend');
  Service.tracker = Symbol.for('cordis.tracker');
  Service.resolveConfig = Symbol.for('cordis.resolveConfig');

  function listenerList(name) {
    let list = listeners.get(name);
    if (!list) { list = []; listeners.set(name, list); }
    return list;
  }

  function removeListener(entry) {
    const list = listeners.get(entry.name);
    if (!list || !list.includes(entry)) return false;
    const next = list.filter(item => item !== entry);
    if (next.length) listeners.set(entry.name, next); else listeners.delete(entry.name);
    return true;
  }

  function createTools(ctx) {
    const pluginId = ctx.pluginId;
    return {
      register(definition) {
        if (!definition || !definition.name || typeof definition.execute !== 'function') {
          throw new TypeError('tools.register expects a tool definition with name and execute');
        }
        const name = String(definition.name);
        tools.set(name, { owner: pluginId, definition });
        const metadata = { ...definition };
        delete metadata.execute;
        if (metadata.output && typeof metadata.output === 'object') {
          metadata.output = { ...metadata.output };
          delete metadata.output.render;
        }
        __host_register_tool(pluginId, name, JSON.stringify(metadata));
        const disposer = () => {
          const current = tools.get(name);
          if (current && current.owner === pluginId) {
            tools.delete(name);
            __host_unregister_tool(pluginId, name);
          }
        };
        return trackDisposer(ctx.__record, disposer, `ctx.tools.register(${name})`);
      },
      async execute(exec) {
        const entry = tools.get(String(exec?.name));
        if (!entry) throw new Error(`tool not found: ${exec?.name}`);
        const value = await entry.definition.execute(exec?.arguments || {});
        let content;
        if (entry.definition.output && typeof entry.definition.output.render === 'function') {
          content = entry.definition.output.render(exec?.arguments || {}, value);
        } else if (Array.isArray(value)) {
          content = value;
        } else {
          content = [{ type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value) }];
        }
        const result = { content, value };
        globalThis.__mahayanaEmit('tools/result', exec, result);
        return result;
      },
      list() { return [...tools.keys()]; }
    };
  }

  function requirePermission(pluginId, permission) {
    if (!__host_permission_check(String(pluginId), String(permission))) {
      throw new Error(`plugin ${pluginId} has not been granted ${permission}`);
    }
  }

  function permissionedService(pluginId, permission, service) {
    return new Proxy(service, {
      get(target, property, receiver) {
        const value = Reflect.get(target, property, receiver);
        if (typeof value !== 'function') {
          requirePermission(pluginId, permission);
          return value;
        }
        return (...args) => {
          requirePermission(pluginId, permission);
          return value.apply(target, args);
        };
      }
    });
  }

  function createStorage(pluginId) {
    return {
      get(key, fallback = undefined) {
        const raw = __host_storage_get(pluginId, String(key));
        return raw == null ? fallback : JSON.parse(raw);
      },
      set(key, value) { __host_storage_set(pluginId, String(key), JSON.stringify(value)); return value; },
      remove(key) { __host_storage_remove(pluginId, String(key)); },
    };
  }

  function createNetwork(pluginId) {
    return {
      request(options) {
        const request = typeof options === 'string' ? { url: options } : { ...(options || {}) };
        const raw = __host_http_request(
          pluginId,
          String(request.method || 'GET'),
          String(request.url || ''),
          JSON.stringify(request.headers || {}),
          request.body == null ? '' : String(request.body)
        );
        return JSON.parse(raw);
      }
    };
  }

  function createTimerService(ctx) {
    const schedule = (callback, delay, repeat, args = []) =>
      scheduleTimer(ctx.pluginId, callback, delay, repeat, args);
    const service = {
      setTimeout(callback, delay) { return service.timeout(callback, delay); },
      setInterval(callback, delay) { return service.interval(callback, delay); },
      timeout(...args) {
        const callback = typeof args[0] === 'function' ? args.shift() : undefined;
        const delay = Number(args[0]) || 0;
        if (callback) {
          let dispose;
          dispose = ctx.effect(() => {
            const timer = schedule(() => {
              dispose();
              callback();
            }, delay, false);
            return () => cancelTimer(timer);
          }, 'ctx.timeout()');
          return dispose;
        }
        let dispose;
        return new Promise((resolve, reject) => {
          dispose = ctx.effect(() => {
            const timer = schedule(resolve, delay, false);
            return () => {
              cancelTimer(timer);
              reject(new Error('Context has been disposed'));
            };
          }, 'ctx.timeout()');
        }).finally(() => dispose?.());
      },
      interval(...args) {
        const callback = typeof args[0] === 'function' ? args.shift() : undefined;
        const delay = Number(args[0]) || 0;
        if (callback) {
          return ctx.effect(() => {
            const timer = schedule(callback, delay, true);
            return () => cancelTimer(timer);
          }, 'ctx.interval()');
        }
        let done;
        let waiting;
        const dispose = ctx.effect(() => {
          const timer = schedule(() => {
            if (waiting) {
              const current = waiting;
              waiting = undefined;
              current.resolve({ done: false, value: undefined });
            }
          }, delay, true);
          return () => {
            cancelTimer(timer);
            if (!done) done = { kind: 'throw', reason: new Error('Context has been disposed') };
            if (waiting && done.kind === 'throw') waiting.reject(done.reason);
          };
        }, 'ctx.interval()');
        return {
          next() {
            if (done?.kind === 'return') return Promise.resolve({ done: true, value: done.value });
            if (done?.kind === 'throw') return Promise.reject(done.reason);
            return new Promise((resolve, reject) => { waiting = { resolve, reject }; });
          },
          return(value) {
            if (!done) done = { kind: 'return', value };
            waiting?.resolve({ done: true, value });
            waiting = undefined;
            dispose();
            return Promise.resolve({ done: true, value });
          },
          throw(reason) {
            if (!done) done = { kind: 'throw', reason };
            waiting?.reject(reason);
            waiting = undefined;
            dispose();
            return Promise.resolve({ done: true, value: undefined });
          },
          [Symbol.asyncIterator]() { return this; },
        };
      },
      throttle(callback, delay, noTrailing = false) {
        let lastCall = -Infinity;
        let pending;
        let disposed = !!noTrailing;
        const dispose = ctx.effect(() => () => {
          disposed = true;
          if (pending) cancelTimer(pending);
        }, 'ctx.throttle()');
        const wrapper = (...args) => {
          if (disposed && noTrailing) return;
          if (pending) cancelTimer(pending);
          const now = Date.now();
          const remaining = Number(delay) - now + lastCall;
          if (remaining <= 0) {
            lastCall = now;
            callback(...args);
            pending = undefined;
          } else if (!disposed) {
            pending = schedule(() => {
              lastCall = Date.now();
              pending = undefined;
              callback(...args);
            }, remaining, false);
          }
        };
        wrapper.dispose = dispose;
        return wrapper;
      },
      debounce(callback, delay) {
        let pending;
        let disposed = false;
        const dispose = ctx.effect(() => () => {
          disposed = true;
          if (pending) cancelTimer(pending);
        }, 'ctx.debounce()');
        const wrapper = (...args) => {
          if (disposed) return;
          if (pending) cancelTimer(pending);
          pending = schedule(() => {
            pending = undefined;
            if (!disposed) callback(...args);
          }, delay, false);
        };
        wrapper.dispose = dispose;
        return wrapper;
      },
    };
    return service;
  }

  function serviceFromContext(ctx, name) {
    const key = String(name);
    const isolation = ctx.__isolations.get(key);
    if (isolation) return isolatedServices.get(isolation)?.get(key);
    if (key === 'tools') return createTools(ctx);
    if (key === 'storage') return permissionedService(ctx.pluginId, 'storage.local', createStorage(ctx.pluginId));
    if (key === 'network') return permissionedService(ctx.pluginId, 'network.request', createNetwork(ctx.pluginId));
    if (key === 'llm') return globalThis.__MahayanaLlmService;
    if (key === 'timer') return createTimerService(ctx);
    return services.get(key);
  }

  function dependenciesAvailableForContext(ctx, deps) {
    return normalizeInject(deps).every(name => serviceFromContext(ctx, name) !== undefined);
  }

  function unloadRecord(record) {
    for (const child of [...(record.children || [])].reverse()) {
      child.fiber?.dispose();
    }
    while (record.disposers.length) disposeOne(record, record.disposers.pop());
  }

  function createFiberFacade(record, ctx, plugin, config, deps) {
    const fiber = {
      uid: childFiberUid++,
      ctx,
      config,
      state: 'pending',
      store: undefined,
      inertia: undefined,
      name: plugin?.name || plugin?.constructor?.name || 'anonymous',
      assertActive() {
        if (fiber.uid == null) throw new Error('INACTIVE_EFFECT');
      },
      effect(execute, label) { return ctx.effect(execute, label); },
      getEffects() {
        return record.disposers
          .filter(disposer => disposer?.__effectLabel)
          .map(disposer => ({ label: disposer.__effectLabel, children: [] }));
      },
      async dispose() {
        if (fiber.uid == null) return;
        fiber.state = 'unloading';
        unloadRecord(record);
        fiber.state = 'disposed';
        fiber.uid = null;
        const parent = record.parent;
        if (parent?.children) parent.children = parent.children.filter(child => child !== record);
      },
      async restart() {
        fiber.assertActive();
        unloadRecord(record);
        fiber.state = 'pending';
        tryMountChild(record);
        return fiber;
      },
      async update(nextConfig) {
        fiber.assertActive();
        fiber.config = nextConfig;
        record.config = nextConfig;
        return fiber.restart();
      },
      async await() {
        if (record.error) throw record.error;
        return fiber;
      },
    };
    fiber.then = (resolve, reject) => {
      if (record.error) { reject?.(record.error); return; }
      Object.defineProperty(fiber, 'then', { value: undefined, configurable: true });
      try { resolve?.(fiber); } catch (error) { reject?.(error); }
    };
    return fiber;
  }

  function tryMountChild(record) {
    if (record.fiber?.uid == null) return;
    const ready = dependenciesAvailableForContext(record.ctx, record.deps);
    if (!ready) {
      if (record.fiber.state === 'active') unloadRecord(record);
      record.fiber.state = 'pending';
      record.fiber.store = undefined;
      return;
    }
    if (record.fiber.state === 'active') return;
    record.fiber.state = 'loading';
    record.error = undefined;
    try {
      const result = mountPluginValue(record.plugin, record.ctx, record.config);
      if (typeof result === 'function') trackDisposer(record, result, `plugin:${record.fiber.name}`);
      record.fiber.store = Object.fromEntries(normalizeInject(record.deps).map(name => [name, record.ctx.get(name)]));
      record.fiber.state = 'active';
    } catch (error) {
      unloadRecord(record);
      record.error = error;
      record.fiber.state = 'failed';
      throw error;
    }
  }

  function notifyInjectedChildren() {
    const visit = record => {
      for (const child of record.children || []) {
        try { tryMountChild(child); } catch (error) { __host_log(record.id, 'error', String(error?.stack || error)); }
        visit(child);
      }
    };
    for (const record of plugins.values()) visit(record);
  }

  function createChildFiber(parentCtx, plugin, config = {}) {
    const deps = normalizeInject(plugin?.inject ?? plugin?.constructor?.inject);
    const record = {
      id: parentCtx.pluginId,
      parent: parentCtx.__record,
      disposers: [],
      children: [],
      plugin,
      config,
      deps,
      ctx: undefined,
      fiber: undefined,
      error: undefined,
    };
    const ctx = new Context(parentCtx.pluginId, record, parentCtx);
    record.ctx = ctx;
    const fiber = createFiberFacade(record, ctx, plugin, config, deps);
    record.fiber = fiber;
    ctx.fiber = fiber;
    ctx.scope = fiber;
    parentCtx.__record.children.push(record);
    tryMountChild(record);
    return fiber;
  }

  class Context {
    constructor(pluginId, record = undefined, parent = undefined) {
      this.pluginId = pluginId;
      this.__record = record || pluginRecord(pluginId);
      this.__parent = parent;
      this.__isolations = new Map(parent?.__isolations || []);
      this.__intercepts = new Map(parent?.__intercepts || []);
      this.root = parent?.root || this;
      this.baseUrl = parent?.baseUrl;
      this.events = this;
      this.reflect = this;
      this.registry = this;
      this.logger = name => new Logger(name || pluginId);
      this.fiber = this.__record.fiber || null;
      this.scope = this.fiber;
      return new Proxy(this, {
        get(target, property, receiver) {
          if (Reflect.has(target, property)) return Reflect.get(target, property, receiver);
          if (typeof property === 'string') return target.get(property);
          return undefined;
        }
      });
    }
    static is(value) { return !!value && typeof value === 'object' && value.events === value && value.registry === value; }
    get(name, _strict = true) { return serviceFromContext(this, name); }
    set(name, value) {
      const key = String(name);
      const isolation = this.__isolations.get(key);
      if (isolation) {
        const owner = isolatedOwners.get(isolation)?.get(key);
        if (owner !== this.pluginId) throw new Error(`service ${key} is not owned by plugin ${this.pluginId}`);
        isolatedServices.get(isolation).set(key, value);
        return;
      }
      if (serviceOwners.get(key) !== this.pluginId) throw new Error(`service ${key} is not owned by plugin ${this.pluginId}`);
      services.set(key, value);
      notifyInjectedChildren();
    }
    provide(name, value = undefined) {
      const key = String(name);
      const isolation = this.__isolations.get(key);
      if (isolation) {
        let values = isolatedServices.get(isolation);
        let owners = isolatedOwners.get(isolation);
        if (!values) { values = new Map(); isolatedServices.set(isolation, values); }
        if (!owners) { owners = new Map(); isolatedOwners.set(isolation, owners); }
        if (values.has(key)) throw new Error(`service already provided in isolated scope: ${key}`);
        values.set(key, value);
        owners.set(key, this.pluginId);
        const disposer = () => {
          if (owners.get(key) !== this.pluginId) return false;
          values.delete(key); owners.delete(key); notifyInjectedChildren(); return true;
        };
        notifyInjectedChildren();
        return trackDisposer(this.__record, disposer, `ctx.provide(${key})`);
      }
      if (services.has(key)) throw new Error(`service already provided: ${key}`);
      services.set(key, value);
      serviceOwners.set(key, this.pluginId);
      __host_register_service(this.pluginId, key);
      const disposer = () => {
        if (serviceOwners.get(key) !== this.pluginId) return false;
        services.delete(key);
        serviceOwners.delete(key);
        __host_unregister_service(this.pluginId, key);
        notifyInjectedChildren();
        return true;
      };
      notifyInjectedChildren();
      return trackDisposer(this.__record, disposer, `ctx.provide(${key})`);
    }
    __registerService(name, value) { return this.provide(name, value); }
    accessor(name, options) {
      const key = String(name);
      if (!options || typeof options.get !== 'function') throw new TypeError('ctx.accessor requires a get hook');
      if (Object.prototype.hasOwnProperty.call(this, key)) throw new Error(`context property already declared: ${key}`);
      Object.defineProperty(this, key, {
        configurable: true,
        enumerable: true,
        get: () => options.get.call(this),
        set: options.set ? value => options.set.call(this, value) : undefined,
      });
      return trackDisposer(this.__record, () => delete this[key], `ctx.accessor(${key})`);
    }
    mixin(source, mixins) {
      const mappings = Array.isArray(mixins)
        ? Object.fromEntries(mixins.map(key => [key, key]))
        : { ...(mixins || {}) };
      const disposers = [];
      for (const [sourceKey, contextKey] of Object.entries(mappings)) {
        disposers.push(this.accessor(String(contextKey), {
          get: () => {
            const target = typeof source === 'string' ? this.get(source) : source;
            const value = target?.[sourceKey];
            return typeof value === 'function' ? value.bind(target) : value;
          }
        }));
      }
      return () => disposers.reverse().forEach(dispose => dispose());
    }
    extend(meta = {}) {
      const child = new Context(this.pluginId, this.__record, this);
      for (const key of Reflect.ownKeys(meta || {})) child[key] = meta[key];
      return child;
    }
    isolate(name, label = Symbol(String(name))) {
      const child = this.extend();
      child.__isolations = new Map(this.__isolations);
      child.__isolations.set(String(name), label);
      if (!isolatedServices.has(label)) isolatedServices.set(label, new Map());
      if (!isolatedOwners.has(label)) isolatedOwners.set(label, new Map());
      return child;
    }
    intercept(name, config) {
      const child = this.extend();
      child.__intercepts = new Map(this.__intercepts);
      const list = [...(child.__intercepts.get(String(name)) || []), config];
      child.__intercepts.set(String(name), list);
      return child;
    }
    effect(setup, label = 'ctx.effect()') {
      if (this.__record.fiber?.uid === null) throw new Error('INACTIVE_EFFECT');
      if (typeof setup !== 'function') throw new TypeError('ctx.effect requires a function');
      const result = setup();
      if (result == null) return () => false;
      if (typeof result !== 'function') throw new TypeError('ctx.effect currently requires a synchronous disposer');
      return trackDisposer(this.__record, result, label);
    }
    on(name, listener, options = {}) {
      const normalized = typeof options === 'boolean' ? { prepend: options } : (options || {});
      const entry = { name: String(name), owner: this.pluginId, listener, prepend: !!normalized.prepend, global: !!normalized.global };
      const list = listenerList(entry.name);
      if (entry.prepend) list.unshift(entry); else list.push(entry);
      return trackDisposer(this.__record, () => removeListener(entry), `ctx.on(${entry.name})`);
    }
    once(name, listener, options = {}) {
      let dispose;
      const wrapped = (...args) => { dispose?.(); return listener(...args); };
      dispose = this.on(name, wrapped, options);
      return dispose;
    }
    emit(name, ...args) { return globalThis.__mahayanaEmit(String(name), ...args); }
    waterfall(name, ...args) { return globalThis.__mahayanaWaterfall(String(name), ...args); }
    parallel(name, ...args) { return globalThis.__mahayanaParallel(String(name), ...args); }
    serial(name, ...args) { return globalThis.__mahayanaSerial(String(name), ...args); }
    bail(name, ...args) { return globalThis.__mahayanaBail(String(name), ...args); }
    timeout(...args) { return createTimerService(this).timeout(...args); }
    interval(...args) { return createTimerService(this).interval(...args); }
    setTimeout(...args) { return createTimerService(this).setTimeout(...args); }
    setInterval(...args) { return createTimerService(this).setInterval(...args); }
    throttle(...args) { return createTimerService(this).throttle(...args); }
    debounce(...args) { return createTimerService(this).debounce(...args); }
    plugin(plugin, config = {}) { return createChildFiber(this, plugin, config); }
    inject(deps, callback) {
      if (typeof callback !== 'function') throw new TypeError('ctx.inject requires a callback');
      return createChildFiber(this, { name: callback.name || 'inject', inject: deps, apply: callback }, {});
    }
  }
  Context.effect = Symbol.for('cordis.effect');
  Context.filter = Symbol.for('cordis.filter');
  Context.isolate = Symbol.for('cordis.isolate');
  Context.intercept = Symbol.for('cordis.intercept');

  function mountPluginValue(plugin, ctx, config) {
    if (typeof plugin === 'function') {
      const source = Function.prototype.toString.call(plugin);
      if (plugin.prototype instanceof Service || /^class\s/.test(source)) return new plugin(ctx, config);
      return plugin(ctx, config);
    }
    if (plugin && typeof plugin.apply === 'function') return plugin.apply(ctx, config);
    throw new TypeError('unsupported child plugin shape');
  }

  globalThis.__MahayanaContext = Context;
  globalThis.__MahayanaService = Service;
  globalThis.__MahayanaSchema = Schema;
  globalThis.__MahayanaLogger = Logger;
  globalThis.__MahayanaLlmService = {
    async chat() { throw new Error('LLM bridge is not configured for this host'); },
    async generate() { throw new Error('LLM bridge is not configured for this host'); }
  };

  globalThis.console = {
    debug: (...args) => __host_log('console', 'debug', args.map(String).join(' ')),
    log: (...args) => __host_log('console', 'info', args.map(String).join(' ')),
    info: (...args) => __host_log('console', 'info', args.map(String).join(' ')),
    warn: (...args) => __host_log('console', 'warn', args.map(String).join(' ')),
    error: (...args) => __host_log('console', 'error', args.map(String).join(' ')),
  };

  globalThis.process = {
    env: {},
    platform: __host_platform(),
    arch: __host_arch(),
    versions: { node: 'mahayana-compat', mahayana: '1' },
    cwd: () => __host_home_dir(),
    nextTick: (fn, ...args) => Promise.resolve().then(() => fn(...args)),
  };

  globalThis.__mahayanaRegisterModule = (pluginId, module) => {
    modules.set(pluginId, module);
    const rawInject = module.inject ?? module.default?.inject ?? module.default?.constructor?.inject ?? [];
    const inject = Array.isArray(rawInject)
      ? rawInject.map(String)
      : Object.keys(rawInject || {}).filter(key => rawInject[key] !== false);
    return JSON.stringify({ inject });
  };

  globalThis.__mahayanaDescribe = pluginId => {
    const module = modules.get(pluginId);
    if (!module) throw new Error(`plugin module not registered: ${pluginId}`);
    const rawInject = module.inject ?? module.default?.inject ?? module.default?.constructor?.inject ?? [];
    const inject = Array.isArray(rawInject)
      ? rawInject.map(String)
      : Object.keys(rawInject || {}).filter(key => rawInject[key] !== false);
    return JSON.stringify({ inject });
  };

  globalThis.__mahayanaLoad = (pluginId, configJson) => {
    if (plugins.has(pluginId)) return plugins.get(pluginId).fiber;
    const module = modules.get(pluginId);
    if (!module) throw new Error(`plugin module not registered: ${pluginId}`);
    const config = configJson ? JSON.parse(configJson) : {};
    const record = {
      id: pluginId,
      disposers: [],
      children: [],
      instance: undefined,
      ctx: undefined,
      config,
      error: undefined,
      fiber: undefined,
    };
    plugins.set(pluginId, record);
    const ctx = new Context(pluginId, record);
    record.ctx = ctx;
    const fiber = {
      uid: childFiberUid++,
      ctx,
      config,
      state: 'loading',
      store: undefined,
      inertia: undefined,
      name: module.name || module.default?.name || pluginId,
      assertActive() { if (fiber.uid == null) throw new Error('INACTIVE_EFFECT'); },
      effect(execute, label) { return ctx.effect(execute, label); },
      getEffects() {
        return record.disposers
          .filter(disposer => disposer?.__effectLabel)
          .map(disposer => ({ label: disposer.__effectLabel, children: [] }));
      },
      async dispose() { globalThis.__mahayanaUnload(pluginId); },
      async restart() {
        fiber.assertActive();
        const nextConfig = record.config;
        globalThis.__mahayanaUnload(pluginId);
        return globalThis.__mahayanaLoad(pluginId, JSON.stringify(nextConfig));
      },
      async update(nextConfig) {
        fiber.assertActive();
        record.config = nextConfig;
        fiber.config = nextConfig;
        return fiber.restart();
      },
      async await() { if (record.error) throw record.error; return fiber; },
    };
    fiber.then = (resolve, reject) => {
      if (record.error) { reject?.(record.error); return; }
      Object.defineProperty(fiber, 'then', { value: undefined, configurable: true });
      try { resolve?.(fiber); } catch (error) { reject?.(error); }
    };
    record.fiber = fiber;
    ctx.fiber = fiber;
    ctx.scope = fiber;
    try {
      const pluginShape = typeof module.apply === 'function' ? { apply: module.apply } : module.default;
      if (!pluginShape) throw new Error('plugin exports neither apply() nor a supported default plugin');
      record.instance = mountPluginValue(pluginShape, ctx, config);
      if (typeof record.instance === 'function') {
        trackDisposer(record, record.instance, `plugin:${fiber.name}`);
      }
      fiber.state = 'active';
      return fiber;
    } catch (error) {
      record.error = error;
      fiber.state = 'failed';
      unloadRecord(record);
      plugins.delete(pluginId);
      throw error;
    }
  };

  globalThis.__mahayanaUnload = pluginId => {
    const record = plugins.get(pluginId);
    if (!record) return;
    if (record.fiber) record.fiber.state = 'unloading';
    unloadRecord(record);
    for (const [name, owner] of [...serviceOwners.entries()]) {
      if (owner === pluginId) {
        services.delete(name); serviceOwners.delete(name); __host_unregister_service(pluginId, name);
      }
    }
    for (const [name, entry] of [...tools.entries()]) {
      if (entry.owner === pluginId) { tools.delete(name); __host_unregister_tool(pluginId, name); }
    }
    for (const [name, list] of [...listeners.entries()]) {
      const next = list.filter(entry => entry.owner !== pluginId);
      if (next.length) listeners.set(name, next); else listeners.delete(name);
    }
    for (const [id, timer] of [...timerCallbacks.entries()]) {
      if (timer.pluginId === pluginId) {
        timerCallbacks.delete(id);
        __host_timer_cancel(id);
      }
    }
    __host_timer_cancel_plugin(pluginId);
    if (record.fiber) {
      record.fiber.state = 'disposed';
      record.fiber.uid = null;
    }
    plugins.delete(pluginId);
    notifyInjectedChildren();
  };

  globalThis.__mahayanaEmit = (name, ...args) => {
    for (const entry of [...(listeners.get(name) || [])]) entry.listener(...args);
  };

  globalThis.__mahayanaWaterfall = (name, ...args) => {
    const list = [...(listeners.get(name) || [])];
    const finalNext = typeof args.at(-1) === 'function' ? args.pop() : () => args[0];
    const dispatch = index => {
      if (index >= list.length) return finalNext();
      const entry = list[index];
      return entry.listener(...args, () => dispatch(index + 1));
    };
    return dispatch(0);
  };

  globalThis.__mahayanaParallel = async (name, ...args) => {
    await Promise.all([...(listeners.get(name) || [])].map(entry => entry.listener(...args)));
  };

  const isBailValue = value => value !== undefined && value !== null && value !== false;

  globalThis.__mahayanaSerial = async (name, ...args) => {
    for (const entry of [...(listeners.get(name) || [])]) {
      const result = await entry.listener(...args);
      if (isBailValue(result)) return result;
    }
    return undefined;
  };

  globalThis.__mahayanaBail = (name, ...args) => {
    for (const entry of [...(listeners.get(name) || [])]) {
      const result = entry.listener(...args);
      if (isBailValue(result)) return result;
    }
    return undefined;
  };

  globalThis.__mahayanaInvokeTool = async (name, argumentsJson) => {
    const entry = tools.get(String(name));
    if (!entry) throw new Error(`tool not found: ${name}`);
    const args = argumentsJson ? JSON.parse(argumentsJson) : {};
    const value = await entry.definition.execute(args);
    return JSON.stringify(value);
  };

  globalThis.__mahayanaForgetModule = pluginId => { modules.delete(pluginId); };
  globalThis.__mahayanaActivePlugins = () => JSON.stringify([...plugins.keys()]);
  globalThis.__mahayanaTools = () => JSON.stringify([...tools.keys()]);
})();
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekBundleEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub config: Value,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub inject: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeepSeekBundlePluginState {
    pub entry_id: String,
    pub runtime_id: String,
    pub state: PluginState,
}

pub fn discover_deepseek_bundle(root: &Path) -> Result<Vec<DeepSeekBundleEntry>, JsRuntimeError> {
    let root = root.canonicalize().map_err(JsRuntimeError::Io)?;
    let package_json = root.join("package.json");
    let patch = if package_json.is_file() {
        let package: Value = serde_json::from_str(&fs::read_to_string(&package_json)?)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
        package
            .get("dsh")
            .and_then(|dsh| dsh.get("bundle"))
            .and_then(|bundle| bundle.get("patch"))
            .and_then(Value::as_str)
            .map(|value| root.join(value.trim_start_matches("./")))
            .filter(|path| path.is_file())
            .or_else(|| {
                root.join("cordis.patch.yml")
                    .is_file()
                    .then(|| root.join("cordis.patch.yml"))
            })
    } else {
        root.join("cordis.patch.yml")
            .is_file()
            .then(|| root.join("cordis.patch.yml"))
    };
    let Some(patch) = patch else {
        return Ok(Vec::new());
    };
    if !patch.starts_with(&root) {
        return Err(JsRuntimeError::InvalidPlugin(
            "DeepSeek bundle patch escapes plugin root".into(),
        ));
    }
    let yaml: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(&patch)?)
        .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
    let value = serde_json::to_value(yaml)
        .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
    let mut entries = Vec::new();
    collect_bundle_entries(&value, None, &mut entries);
    let mut seen = BTreeSet::new();
    entries.retain(|entry| seen.insert(entry.id.clone()));
    Ok(entries)
}

fn collect_bundle_entries(
    value: &Value,
    hint: Option<&str>,
    output: &mut Vec<DeepSeekBundleEntry>,
) {
    match value {
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let hint = format!("entry-{index}");
                collect_bundle_entries(item, Some(&hint), output);
            }
        }
        Value::Object(map) => {
            if let Some(name) = map.get("name").and_then(Value::as_str) {
                let id = map
                    .get("id")
                    .and_then(Value::as_str)
                    .or(hint)
                    .unwrap_or(name)
                    .to_string();
                let inject = match map.get("inject") {
                    Some(Value::Array(items)) => items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect(),
                    Some(Value::Object(items)) => items
                        .iter()
                        .filter(|(_, value)| value.as_bool() != Some(false))
                        .map(|(name, _)| name.clone())
                        .collect(),
                    _ => Vec::new(),
                };
                output.push(DeepSeekBundleEntry {
                    id,
                    name: name.to_string(),
                    config: map
                        .get("config")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!({})),
                    disabled: map
                        .get("disabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    inject,
                });
                return;
            }
            for (key, child) in map {
                collect_bundle_entries(child, Some(key), output);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub portable_compatible: bool,
    pub mobile_compatible: bool,
    #[serde(default)]
    pub supported_modules: Vec<String>,
    #[serde(default)]
    pub capability_required_modules: Vec<String>,
    #[serde(default)]
    pub desktop_only_modules: Vec<String>,
    #[serde(default)]
    pub unsupported_modules: Vec<String>,
    #[serde(default)]
    pub native_addons: Vec<String>,
    #[serde(default)]
    pub commonjs_require_files: Vec<String>,
}

impl CompatibilityReport {
    pub fn blockers(&self) -> Vec<String> {
        let mut blockers = Vec::new();
        blockers.extend(
            self.capability_required_modules
                .iter()
                .map(|module| format!("capability required: {module}")),
        );
        blockers.extend(
            self.desktop_only_modules
                .iter()
                .map(|module| format!("desktop only: {module}")),
        );
        blockers.extend(
            self.unsupported_modules
                .iter()
                .map(|module| format!("unsupported: {module}")),
        );
        blockers.extend(
            self.native_addons
                .iter()
                .map(|path| format!("native Node addon: {path}")),
        );
        blockers.extend(
            self.commonjs_require_files
                .iter()
                .map(|path| format!("CommonJS require() needs compatibility transform: {path}")),
        );
        blockers
    }
}

pub fn scan_package_compatibility(root: &Path) -> Result<CompatibilityReport, JsRuntimeError> {
    let root = root.canonicalize().map_err(JsRuntimeError::Io)?;
    if !root.is_dir() {
        return Err(JsRuntimeError::InvalidPlugin(
            "compatibility scan root must be a directory".into(),
        ));
    }
    let mut modules = BTreeSet::new();
    let mut native_addons = BTreeSet::new();
    let mut commonjs_require_files = BTreeSet::new();
    let mut stack = vec![root.clone()];
    let mut visited_files = 0usize;
    while let Some(directory) = stack.pop() {
        for entry in fs::read_dir(&directory).map_err(JsRuntimeError::Io)? {
            let entry = entry.map_err(JsRuntimeError::Io)?;
            let file_type = entry.file_type().map_err(JsRuntimeError::Io)?;
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) != Some(".git") {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            visited_files = visited_files.saturating_add(1);
            if visited_files > 20_000 {
                return Err(JsRuntimeError::InvalidPlugin(
                    "plugin compatibility scan exceeded 20,000 files".into(),
                ));
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.extension().and_then(|value| value.to_str()) == Some("node") {
                native_addons.insert(relative);
                continue;
            }
            if !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx")
            ) {
                continue;
            }
            let metadata = fs::metadata(&path).map_err(JsRuntimeError::Io)?;
            if metadata.len() > 4 * 1024 * 1024 {
                continue;
            }
            let source = fs::read_to_string(&path).map_err(JsRuntimeError::Io)?;
            for module in discover_module_specifiers(&source) {
                if module.starts_with("node:") || known_node_bare_module(&module) {
                    modules.insert(normalize_node_module(&module));
                }
            }
            if has_dynamic_require(&source) {
                commonjs_require_files.insert(relative);
            }
        }
    }

    let mut supported_modules = Vec::new();
    let mut capability_required_modules = Vec::new();
    let mut desktop_only_modules = Vec::new();
    let mut unsupported_modules = Vec::new();
    for module in modules {
        match module.as_str() {
            "node:path" | "node:events" | "node:buffer" | "node:process" | "node:os"
            | "node:url" | "node:util" | "node:assert" | "node:querystring" | "node:crypto" => {
                supported_modules.push(module)
            }
            "node:fs" | "node:fs/promises" | "node:http" | "node:https" => {
                capability_required_modules.push(module)
            }
            "node:child_process"
            | "node:worker_threads"
            | "node:net"
            | "node:tls"
            | "node:dgram"
            | "node:cluster" => desktop_only_modules.push(module),
            _ => unsupported_modules.push(module),
        }
    }
    let native_addons = native_addons.into_iter().collect::<Vec<_>>();
    let commonjs_require_files = commonjs_require_files.into_iter().collect::<Vec<_>>();
    let portable_compatible = capability_required_modules.is_empty()
        && desktop_only_modules.is_empty()
        && unsupported_modules.is_empty()
        && native_addons.is_empty()
        && commonjs_require_files.is_empty();
    let mobile_compatible = portable_compatible;
    Ok(CompatibilityReport {
        portable_compatible,
        mobile_compatible,
        supported_modules,
        capability_required_modules,
        desktop_only_modules,
        unsupported_modules,
        native_addons,
        commonjs_require_files,
    })
}

fn normalize_node_module(module: &str) -> String {
    if module.starts_with("node:") {
        module.to_string()
    } else {
        format!("node:{module}")
    }
}

fn known_node_bare_module(module: &str) -> bool {
    matches!(
        module,
        "path"
            | "events"
            | "buffer"
            | "process"
            | "os"
            | "url"
            | "util"
            | "assert"
            | "querystring"
            | "crypto"
            | "fs"
            | "fs/promises"
            | "http"
            | "https"
            | "child_process"
            | "worker_threads"
            | "net"
            | "tls"
            | "dgram"
            | "cluster"
            | "stream"
            | "zlib"
            | "module"
            | "vm"
    )
}

fn discover_module_specifiers(source: &str) -> BTreeSet<String> {
    let mut modules = BTreeSet::new();
    for marker in [" from ", "import(", "import (", "require(", "require ("] {
        let mut offset = 0usize;
        while let Some(found) = source[offset..].find(marker) {
            let start = offset + found + marker.len();
            if let Some(value) = quoted_string_at(&source[start..]) {
                modules.insert(value);
            }
            offset = start;
            if offset >= source.len() {
                break;
            }
        }
    }
    modules
}

fn quoted_string_at(source: &str) -> Option<String> {
    let source = source.trim_start();
    let quote = source.as_bytes().first().copied()?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let mut escaped = false;
    for (index, byte) in source.as_bytes().iter().copied().enumerate().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == quote {
            return Some(source[1..index].to_string());
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDescription {
    #[serde(default)]
    pub inject: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    ServiceRegistered {
        plugin_id: String,
        service: String,
    },
    ServiceUnregistered {
        plugin_id: String,
        service: String,
    },
    ToolRegistered {
        plugin_id: String,
        tool: String,
        metadata_json: String,
    },
    ToolUnregistered {
        plugin_id: String,
        tool: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    pub plugin_id: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct HostTimer {
    plugin_id: String,
    due: Instant,
    delay: Duration,
    repeat: bool,
}

#[derive(Default)]
struct BridgeState {
    events: Vec<HostEvent>,
    logs: Vec<LogRecord>,
    storage: HashMap<(String, String), String>,
    tools: BTreeSet<String>,
    timers: HashMap<u64, HostTimer>,
    grants: HashMap<String, BTreeSet<String>>,
}

#[derive(Clone)]
struct SharedModules {
    roots: Arc<RwLock<Vec<PathBuf>>>,
    revision: Arc<AtomicU64>,
}

impl SharedModules {
    fn new() -> Self {
        Self {
            roots: Arc::new(RwLock::new(Vec::new())),
            revision: Arc::new(AtomicU64::new(1)),
        }
    }

    fn add_root(&self, root: &Path) -> Result<PathBuf, JsRuntimeError> {
        let canonical = root.canonicalize().map_err(JsRuntimeError::Io)?;
        let mut roots = self.roots.write().map_err(|_| JsRuntimeError::Poisoned)?;
        if !roots.iter().any(|existing| existing == &canonical) {
            roots.push(canonical.clone());
        }
        Ok(canonical)
    }

    fn allowed(&self, path: &Path) -> bool {
        self.roots
            .read()
            .map(|roots| roots.iter().any(|root| path.starts_with(root)))
            .unwrap_or(false)
    }

    fn bump_revision(&self) -> u64 {
        self.revision
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }

    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }
}

struct CompatResolver {
    modules: SharedModules,
}

impl Resolver for CompatResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        if builtin_module(name).is_some() {
            return Ok(name.to_string());
        }
        let resolved = resolve_module_path(&self.modules, base, name)
            .map_err(|error| JsError::new_resolving_message(base, name, error.to_string()))?;
        Ok(format!(
            "{}?mh_rev={}",
            resolved.to_string_lossy(),
            self.modules.revision()
        ))
    }
}

struct CompatLoader {
    modules: SharedModules,
}

impl Loader for CompatLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, Declared>> {
        if let Some(source) = builtin_module(name) {
            return Module::declare(ctx.clone(), name, source);
        }
        let clean_name = name.split('?').next().unwrap_or(name);
        let path = Path::new(clean_name);
        let canonical = path
            .canonicalize()
            .map_err(|error| JsError::new_loading_message(name, error.to_string()))?;
        if !self.modules.allowed(&canonical) {
            return Err(JsError::new_loading_message(
                name,
                "module path is outside plugin roots",
            ));
        }
        let mut source = fs::read_to_string(&canonical)
            .map_err(|error| JsError::new_loading_message(name, error.to_string()))?;
        if is_commonjs(&canonical, &source) {
            source = wrap_commonjs(&source);
        }
        Module::declare(ctx.clone(), name, source)
    }
}

#[derive(Debug, Clone)]
struct PluginRecord {
    entry: PathBuf,
    inject: Vec<String>,
    config_json: String,
    enabled: bool,
    state: PluginState,
}

pub struct DeepSeekJsHost {
    runtime: Runtime,
    context: Context,
    modules: SharedModules,
    bridge: Arc<Mutex<BridgeState>>,
    services: ServiceRegistry,
    plugins: HashMap<String, PluginRecord>,
}

impl DeepSeekJsHost {
    pub fn new() -> Result<Self, JsRuntimeError> {
        let runtime = Runtime::new().map_err(JsRuntimeError::Js)?;
        let modules = SharedModules::new();
        runtime.set_loader(
            CompatResolver {
                modules: modules.clone(),
            },
            CompatLoader {
                modules: modules.clone(),
            },
        );
        let context = Context::full(&runtime).map_err(JsRuntimeError::Js)?;
        let bridge = Arc::new(Mutex::new(BridgeState::default()));
        install_host_functions(&context, bridge.clone())?;
        context
            .with(|ctx| ctx.eval::<(), _>(RUNTIME_PRELUDE))
            .map_err(JsRuntimeError::Js)?;
        let mut services = ServiceRegistry::default();
        for service in ["tools", "storage", "network", "llm", "timer"] {
            services.provide(service, "mahayana:host");
        }
        Ok(Self {
            runtime,
            context,
            modules,
            bridge,
            services,
            plugins: HashMap::new(),
        })
    }

    pub fn register_deepseek_bundle_with_grants(
        &mut self,
        root: &Path,
        grants: &[String],
    ) -> Result<Vec<DeepSeekBundlePluginState>, JsRuntimeError> {
        let canonical_root = self.modules.add_root(root)?;
        let entries = discover_deepseek_bundle(&canonical_root)?;
        let mut states = Vec::new();
        for entry in entries {
            let runtime_id = bundle_runtime_id(&canonical_root, &entry.id, &entry.name);
            if entry.disabled {
                states.push(DeepSeekBundlePluginState {
                    entry_id: entry.id,
                    runtime_id,
                    state: PluginState::Disposed,
                });
                continue;
            }
            let resolved = self.resolve_bundle_entry(&canonical_root, &entry.name)?;
            let relative = resolved.strip_prefix(&canonical_root).map_err(|_| {
                JsRuntimeError::InvalidPlugin(format!(
                    "bundle entry {} resolves outside installed package",
                    entry.name
                ))
            })?;
            let state = self.register_plugin_with_grants(
                &runtime_id,
                &canonical_root,
                relative,
                &entry.config,
                grants,
            )?;
            if !entry.inject.is_empty() {
                if let Some(record) = self.plugins.get_mut(&runtime_id) {
                    record.inject = merge_injects(&record.inject, &entry.inject);
                }
                self.reconcile()?;
            }
            states.push(DeepSeekBundlePluginState {
                entry_id: entry.id,
                runtime_id: runtime_id.clone(),
                state: self.plugin_state(&runtime_id).unwrap_or(state),
            });
        }
        Ok(states)
    }

    fn resolve_bundle_entry(&self, root: &Path, name: &str) -> Result<PathBuf, JsRuntimeError> {
        if name.starts_with("./") || name.starts_with("../") {
            return canonical_entry(root, Path::new(name));
        }
        if let Some(entry) = resolve_self_package_entry(root, name)? {
            return Ok(entry);
        }
        let base = root.join("__mahayana_bundle_entry__.mjs");
        resolve_module_path(&self.modules, &base.to_string_lossy(), name)
    }

    pub fn set_plugin_grants(
        &self,
        plugin_id: &str,
        grants: impl IntoIterator<Item = String>,
    ) -> Result<(), JsRuntimeError> {
        let mut bridge = self.bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
        bridge
            .grants
            .insert(plugin_id.to_string(), grants.into_iter().collect());
        Ok(())
    }

    pub fn register_plugin_with_grants(
        &mut self,
        plugin_id: &str,
        root: &Path,
        entry: &Path,
        config: &Value,
        grants: &[String],
    ) -> Result<PluginState, JsRuntimeError> {
        self.set_plugin_grants(plugin_id, grants.iter().cloned())?;
        self.register_plugin_inner(plugin_id, root, entry, config)
    }

    pub fn register_plugin(
        &mut self,
        plugin_id: &str,
        root: &Path,
        entry: &Path,
        config: &Value,
    ) -> Result<PluginState, JsRuntimeError> {
        self.set_plugin_grants(
            plugin_id,
            ["storage.local".to_string(), "network.request".to_string()],
        )?;
        self.register_plugin_inner(plugin_id, root, entry, config)
    }

    fn register_plugin_inner(
        &mut self,
        plugin_id: &str,
        root: &Path,
        entry: &Path,
        config: &Value,
    ) -> Result<PluginState, JsRuntimeError> {
        validate_plugin_id(plugin_id)?;
        let root = self.modules.add_root(root)?;
        let entry = canonical_entry(&root, entry)?;
        let description = self.describe_plugin_module(plugin_id, &entry)?;
        let config_json = serde_json::to_string(config)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
        self.plugins.insert(
            plugin_id.to_string(),
            PluginRecord {
                entry,
                inject: description.inject,
                config_json,
                enabled: true,
                state: PluginState::Pending,
            },
        );
        self.reconcile()?;
        Ok(self.plugins[plugin_id].state)
    }

    pub fn enable_plugin(&mut self, plugin_id: &str) -> Result<PluginState, JsRuntimeError> {
        let record = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| JsRuntimeError::PluginNotFound(plugin_id.to_string()))?;
        record.enabled = true;
        if record.state == PluginState::Disposed {
            record.state = PluginState::Pending;
        }
        self.reconcile()?;
        Ok(self.plugins[plugin_id].state)
    }

    pub fn disable_plugin(&mut self, plugin_id: &str) -> Result<(), JsRuntimeError> {
        if let Some(record) = self.plugins.get_mut(plugin_id) {
            record.enabled = false;
        } else {
            return Err(JsRuntimeError::PluginNotFound(plugin_id.to_string()));
        }
        self.unload_with_dependents(plugin_id, false)?;
        if let Some(record) = self.plugins.get_mut(plugin_id) {
            record.state = PluginState::Disposed;
        }
        Ok(())
    }

    pub fn reload_plugin(&mut self, plugin_id: &str) -> Result<PluginState, JsRuntimeError> {
        let record = self
            .plugins
            .get(plugin_id)
            .cloned()
            .ok_or_else(|| JsRuntimeError::PluginNotFound(plugin_id.to_string()))?;
        self.describe_plugin_module(plugin_id, &record.entry)?;
        self.unload_with_dependents(plugin_id, true)?;
        self.reconcile()?;
        Ok(self.plugins[plugin_id].state)
    }

    pub fn update_plugin(
        &mut self,
        plugin_id: &str,
        root: &Path,
        entry: &Path,
        config: &Value,
        enabled: bool,
        inject_override: Option<&[String]>,
    ) -> Result<PluginState, JsRuntimeError> {
        validate_plugin_id(plugin_id)?;
        if !self.plugins.contains_key(plugin_id) {
            self.register_plugin(plugin_id, root, entry, config)?;
            if !enabled {
                self.disable_plugin(plugin_id)?;
                return Ok(PluginState::Disposed);
            }
            if let Some(inject) = inject_override {
                if let Some(record) = self.plugins.get_mut(plugin_id) {
                    record.inject = merge_injects(&record.inject, inject);
                }
                self.reconcile()?;
            }
            return Ok(self.plugins[plugin_id].state);
        }

        let previous = self.plugins.get(plugin_id).cloned().unwrap();
        let candidate_root = self.modules.add_root(root)?;
        let candidate_entry = canonical_entry(&candidate_root, entry)?;
        let mut description = self.describe_plugin_module(plugin_id, &candidate_entry)?;
        if let Some(inject) = inject_override {
            description.inject = merge_injects(&description.inject, inject);
        }
        let config_json = serde_json::to_string(config)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;

        self.unload_with_dependents(plugin_id, true)?;
        self.plugins.insert(
            plugin_id.to_string(),
            PluginRecord {
                entry: candidate_entry,
                inject: description.inject,
                config_json,
                enabled,
                state: if enabled {
                    PluginState::Pending
                } else {
                    PluginState::Disposed
                },
            },
        );
        if enabled {
            self.reconcile()?;
        }
        let candidate_state = self.plugins[plugin_id].state;
        if candidate_state != PluginState::Failed {
            return Ok(candidate_state);
        }

        let candidate_error = format!("candidate plugin {plugin_id} failed during apply");
        self.services.remove_provider(plugin_id);
        self.describe_plugin_module(plugin_id, &previous.entry)
            .map_err(|rollback| JsRuntimeError::UpdateRollback {
                plugin_id: plugin_id.to_string(),
                candidate: candidate_error.clone(),
                rollback: rollback.to_string(),
            })?;
        let mut restored = previous;
        restored.state = if restored.enabled {
            PluginState::Pending
        } else {
            PluginState::Disposed
        };
        self.plugins.insert(plugin_id.to_string(), restored);
        if self.plugins[plugin_id].enabled {
            self.reconcile()
                .map_err(|rollback| JsRuntimeError::UpdateRollback {
                    plugin_id: plugin_id.to_string(),
                    candidate: candidate_error.clone(),
                    rollback: rollback.to_string(),
                })?;
        }
        Err(JsRuntimeError::UpdateFailed {
            plugin_id: plugin_id.to_string(),
            message: candidate_error,
        })
    }

    pub fn remove_plugin(&mut self, plugin_id: &str) -> Result<(), JsRuntimeError> {
        if !self.plugins.contains_key(plugin_id) {
            return Err(JsRuntimeError::PluginNotFound(plugin_id.to_string()));
        }
        self.disable_plugin(plugin_id)?;
        self.plugins.remove(plugin_id);
        self.context
            .with(|ctx| {
                let function: Function = ctx.globals().get("__mahayanaForgetModule")?;
                function.call::<_, ()>((plugin_id.to_string(),))
            })
            .map_err(JsRuntimeError::Js)?;
        Ok(())
    }

    pub fn plugin_state(&self, plugin_id: &str) -> Option<PluginState> {
        self.plugins.get(plugin_id).map(|plugin| plugin.state)
    }

    pub fn plugin_description(&self, plugin_id: &str) -> Option<PluginDescription> {
        self.plugins.get(plugin_id).map(|plugin| PluginDescription {
            inject: plugin.inject.clone(),
        })
    }

    pub fn active_plugins(&self) -> Result<Vec<String>, JsRuntimeError> {
        let json = self.call_string_function("__mahayanaActivePlugins", ())?;
        serde_json::from_str(&json)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))
    }

    pub fn registered_tools(&self) -> Result<Vec<String>, JsRuntimeError> {
        let json = self.call_string_function("__mahayanaTools", ())?;
        serde_json::from_str(&json)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))
    }

    pub fn call_tool_json(&self, name: &str, arguments: &Value) -> Result<Value, JsRuntimeError> {
        let arguments_json = serde_json::to_string(arguments)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
        let bridge = self.bridge.clone();
        let result_json = self
            .context
            .with(|ctx| {
                let function: Function = ctx.globals().get("__mahayanaInvokeTool")?;
                let promise: Promise = function.call((name.to_string(), arguments_json))?;
                loop {
                    if let Some(result) = promise.result::<String>() {
                        return result;
                    }
                    let mut progressed = false;
                    while ctx.execute_pending_job() {
                        progressed = true;
                    }
                    if let Some(result) = promise.result::<String>() {
                        return result;
                    }
                    let due = take_due_timers(&bridge).map_err(js_bridge_error)?;
                    if !due.is_empty() {
                        fire_timer_ids(&ctx, &due)?;
                        continue;
                    }
                    if progressed {
                        continue;
                    }
                    match next_timer_delay(&bridge).map_err(js_bridge_error)? {
                        Some(delay) => {
                            std::thread::sleep(delay.min(Duration::from_millis(50)));
                        }
                        None => return Err(JsError::WouldBlock),
                    }
                }
            })
            .map_err(JsRuntimeError::Js)?;
        serde_json::from_str(&result_json)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))
    }

    pub fn pump_timers(&self) -> Result<usize, JsRuntimeError> {
        let due = take_due_timers(&self.bridge)?;
        let fired = self
            .context
            .with(|ctx| {
                fire_timer_ids(&ctx, &due)?;
                while ctx.execute_pending_job() {}
                Ok::<usize, JsError>(due.len())
            })
            .map_err(JsRuntimeError::Js)?;
        Ok(fired)
    }

    pub fn next_timer_delay(&self) -> Result<Option<Duration>, JsRuntimeError> {
        next_timer_delay(&self.bridge)
    }

    pub fn drain_events(&self) -> Result<Vec<HostEvent>, JsRuntimeError> {
        let mut bridge = self.bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
        Ok(std::mem::take(&mut bridge.events))
    }

    pub fn logs(&self) -> Result<Vec<LogRecord>, JsRuntimeError> {
        let bridge = self.bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
        Ok(bridge.logs.clone())
    }

    pub fn storage_snapshot(&self) -> Result<HashMap<(String, String), String>, JsRuntimeError> {
        let bridge = self.bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
        Ok(bridge.storage.clone())
    }

    /// Keeps the embedded QuickJS runtime owned by this host for the full
    /// lifetime of all plugin contexts. The explicit accessor is intentionally
    /// omitted so rquickjs diagnostics do not leak into Mahayana's public ABI.
    pub fn runtime_is_alive(&self) -> bool {
        let _ = &self.runtime;
        true
    }

    fn describe_plugin_module(
        &self,
        plugin_id: &str,
        entry: &Path,
    ) -> Result<PluginDescription, JsRuntimeError> {
        self.modules.bump_revision();
        let module_specifier = serde_json::to_string(&entry.to_string_lossy().into_owned())
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
        let plugin_json = serde_json::to_string(plugin_id)
            .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
        let wrapper = format!(
            "import * as plugin from {module_specifier}; globalThis.__mahayanaRegisterModule({plugin_json}, plugin);"
        );
        self.context
            .with(|ctx| {
                Module::evaluate(
                    ctx.clone(),
                    format!("mahayana:describe:{plugin_id}"),
                    wrapper,
                )?
                .finish::<()>()?;
                let function: Function = ctx.globals().get("__mahayanaDescribe")?;
                let json: String = function.call((plugin_id.to_string(),))?;
                serde_json::from_str(&json).map_err(|error| {
                    JsError::new_from_js_message("string", "PluginDescription", error.to_string())
                })
            })
            .map_err(JsRuntimeError::Js)
    }

    fn dependencies_ready(&self, plugin_id: &str) -> bool {
        self.plugins.get(plugin_id).is_some_and(|plugin| {
            plugin
                .inject
                .iter()
                .all(|service| self.services.get(service).is_some())
        })
    }

    fn reconcile(&mut self) -> Result<(), JsRuntimeError> {
        loop {
            let ready = self
                .plugins
                .iter()
                .filter(|(id, plugin)| {
                    plugin.enabled
                        && matches!(plugin.state, PluginState::Installed | PluginState::Pending)
                        && self.dependencies_ready(id)
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            if ready.is_empty() {
                break;
            }
            let mut progressed = false;
            for plugin_id in ready {
                if self.activate_plugin(&plugin_id).is_ok() {
                    progressed = true;
                } else if let Some(plugin) = self.plugins.get_mut(&plugin_id) {
                    plugin.state = PluginState::Failed;
                }
            }
            if !progressed {
                break;
            }
        }
        for (id, plugin) in &mut self.plugins {
            if plugin.enabled
                && plugin.state != PluginState::Active
                && plugin.state != PluginState::Failed
                && !plugin
                    .inject
                    .iter()
                    .all(|service| self.services.get(service).is_some())
            {
                let _ = id;
                plugin.state = PluginState::Pending;
            }
        }
        Ok(())
    }

    fn activate_plugin(&mut self, plugin_id: &str) -> Result<(), JsRuntimeError> {
        let config_json = {
            let plugin = self
                .plugins
                .get_mut(plugin_id)
                .ok_or_else(|| JsRuntimeError::PluginNotFound(plugin_id.to_string()))?;
            plugin.state = PluginState::Loading;
            plugin.config_json.clone()
        };
        self.context
            .with(|ctx| {
                let function: Function = ctx.globals().get("__mahayanaLoad")?;
                function.call::<_, ()>((plugin_id.to_string(), config_json))
            })
            .map_err(JsRuntimeError::Js)?;
        self.apply_bridge_events()?;
        if let Some(plugin) = self.plugins.get_mut(plugin_id) {
            plugin.state = PluginState::Active;
        }
        Ok(())
    }

    fn apply_bridge_events(&mut self) -> Result<(), JsRuntimeError> {
        let events = self.drain_events()?;
        for event in events {
            match event {
                HostEvent::ServiceRegistered { plugin_id, service } => {
                    self.services.provide(service, plugin_id);
                }
                HostEvent::ServiceUnregistered { plugin_id, service } => {
                    self.services.remove_service(&service, &plugin_id);
                }
                HostEvent::ToolRegistered { .. } | HostEvent::ToolUnregistered { .. } => {}
            }
        }
        Ok(())
    }

    fn unload_with_dependents(
        &mut self,
        plugin_id: &str,
        keep_enabled: bool,
    ) -> Result<(), JsRuntimeError> {
        let mut order = Vec::new();
        self.collect_dependents(plugin_id, &mut BTreeSet::new(), &mut order);
        order.push(plugin_id.to_string());
        for id in order {
            if self
                .plugins
                .get(&id)
                .is_some_and(|plugin| plugin.state == PluginState::Active)
            {
                if let Some(plugin) = self.plugins.get_mut(&id) {
                    plugin.state = PluginState::Unloading;
                }
                self.context
                    .with(|ctx| {
                        let function: Function = ctx.globals().get("__mahayanaUnload")?;
                        function.call::<_, ()>((id.clone(),))
                    })
                    .map_err(JsRuntimeError::Js)?;
                self.services.remove_provider(&id);
                self.apply_bridge_events()?;
            }
            if let Some(plugin) = self.plugins.get_mut(&id) {
                plugin.state = if plugin.enabled || (id == plugin_id && keep_enabled) {
                    PluginState::Pending
                } else {
                    PluginState::Disposed
                };
            }
        }
        Ok(())
    }

    fn collect_dependents(
        &self,
        provider_plugin_id: &str,
        visited: &mut BTreeSet<String>,
        out: &mut Vec<String>,
    ) {
        for (candidate_id, candidate) in &self.plugins {
            if candidate.state != PluginState::Active || visited.contains(candidate_id) {
                continue;
            }
            let depends = candidate.inject.iter().any(|service| {
                self.services
                    .get(service)
                    .is_some_and(|provider| provider.plugin_id == provider_plugin_id)
            });
            if depends {
                visited.insert(candidate_id.clone());
                self.collect_dependents(candidate_id, visited, out);
                out.push(candidate_id.clone());
            }
        }
    }

    fn call_string_function<A>(&self, name: &str, args: A) -> Result<String, JsRuntimeError>
    where
        for<'js> A: rquickjs::function::IntoArgs<'js>,
    {
        self.context
            .with(|ctx| {
                let function: Function = ctx.globals().get(name)?;
                function.call(args)
            })
            .map_err(JsRuntimeError::Js)
    }
}

fn js_bridge_error(error: JsRuntimeError) -> JsError {
    JsError::new_from_js_message("Rust bridge", "JavaScript", error.to_string())
}

fn take_due_timers(bridge: &Arc<Mutex<BridgeState>>) -> Result<Vec<u64>, JsRuntimeError> {
    let mut state = bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
    let now = Instant::now();
    let due = state
        .timers
        .iter()
        .filter(|(_, timer)| timer.due <= now)
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for id in &due {
        if let Some(timer) = state.timers.get_mut(id) {
            if timer.repeat {
                timer.due = now + timer.delay;
            } else {
                state.timers.remove(id);
            }
        }
    }
    Ok(due)
}

fn next_timer_delay(bridge: &Arc<Mutex<BridgeState>>) -> Result<Option<Duration>, JsRuntimeError> {
    let state = bridge.lock().map_err(|_| JsRuntimeError::Poisoned)?;
    let now = Instant::now();
    Ok(state
        .timers
        .values()
        .map(|timer| timer.due.saturating_duration_since(now))
        .min())
}

fn fire_timer_ids(ctx: &Ctx<'_>, ids: &[u64]) -> rquickjs::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let function: Function = ctx.globals().get("__mahayanaFireTimer")?;
    for id in ids {
        function.call::<_, ()>((*id as i32,))?;
    }
    Ok(())
}

fn install_host_functions(
    context: &Context,
    bridge: Arc<Mutex<BridgeState>>,
) -> Result<(), JsRuntimeError> {
    let http_client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(8))
        .user_agent("Mahayana-JS-Runtime/1")
        .build()
        .map_err(|error| JsRuntimeError::Transport(error.to_string()))?;

    context.with(|ctx| {
        let globals = ctx.globals();

        let log_state = bridge.clone();
        globals.set(
            "__host_log",
            Function::new(ctx.clone(), move |plugin_id: String, level: String, message: String| {
                if let Ok(mut state) = log_state.lock() {
                    state.logs.push(LogRecord { plugin_id, level, message });
                }
            })?,
        )?;

        let service_state = bridge.clone();
        globals.set(
            "__host_register_service",
            Function::new(ctx.clone(), move |plugin_id: String, service: String| {
                if let Ok(mut state) = service_state.lock() {
                    state.events.push(HostEvent::ServiceRegistered { plugin_id, service });
                }
            })?,
        )?;

        let service_state = bridge.clone();
        globals.set(
            "__host_unregister_service",
            Function::new(ctx.clone(), move |plugin_id: String, service: String| {
                if let Ok(mut state) = service_state.lock() {
                    state.events.push(HostEvent::ServiceUnregistered { plugin_id, service });
                }
            })?,
        )?;

        let tool_state = bridge.clone();
        globals.set(
            "__host_register_tool",
            Function::new(
                ctx.clone(),
                move |plugin_id: String, tool: String, metadata_json: String| {
                    if let Ok(mut state) = tool_state.lock() {
                        state.tools.insert(tool.clone());
                        state.events.push(HostEvent::ToolRegistered {
                            plugin_id,
                            tool,
                            metadata_json,
                        });
                    }
                },
            )?,
        )?;

        let tool_state = bridge.clone();
        globals.set(
            "__host_unregister_tool",
            Function::new(ctx.clone(), move |plugin_id: String, tool: String| {
                if let Ok(mut state) = tool_state.lock() {
                    state.tools.remove(&tool);
                    state.events.push(HostEvent::ToolUnregistered { plugin_id, tool });
                }
            })?,
        )?;

        let storage_state = bridge.clone();
        globals.set(
            "__host_storage_get",
            Function::new(ctx.clone(), move |plugin_id: String, key: String| -> Option<String> {
                storage_state
                    .lock()
                    .ok()
                    .and_then(|state| state.storage.get(&(plugin_id, key)).cloned())
            })?,
        )?;

        let storage_state = bridge.clone();
        globals.set(
            "__host_storage_set",
            Function::new(ctx.clone(), move |plugin_id: String, key: String, value: String| {
                if let Ok(mut state) = storage_state.lock() {
                    state.storage.insert((plugin_id, key), value);
                }
            })?,
        )?;

        let storage_state = bridge.clone();
        globals.set(
            "__host_storage_remove",
            Function::new(ctx.clone(), move |plugin_id: String, key: String| {
                if let Ok(mut state) = storage_state.lock() {
                    state.storage.remove(&(plugin_id, key));
                }
            })?,
        )?;

        globals.set(
            "__host_http_request",
            Function::new(
                ctx.clone(),
                move |_plugin_id: String, method: String, url: String, headers_json: String, body: String| -> rquickjs::Result<String> {
                    let url = Url::parse(&url).map_err(|error| JsError::new_from_js_message("string", "URL", error.to_string()))?;
                    if url.scheme() != "https" {
                        return Err(JsError::new_from_js_message("string", "HTTPS URL", "only HTTPS is allowed"));
                    }
                    let method = reqwest::Method::from_bytes(method.as_bytes())
                        .map_err(|error| JsError::new_from_js_message("string", "HTTP method", error.to_string()))?;
                    let mut request = http_client.request(method, url);
                    let headers: HashMap<String, String> = serde_json::from_str(&headers_json)
                        .map_err(|error| JsError::new_from_js_message("string", "headers", error.to_string()))?;
                    for (name, value) in headers {
                        request = request.header(name, value);
                    }
                    if !body.is_empty() {
                        request = request.body(body);
                    }
                    let response = request.send()
                        .map_err(|error| JsError::new_from_js_message("request", "response", error.to_string()))?;
                    let status = response.status().as_u16();
                    let headers = response
                        .headers()
                        .iter()
                        .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.to_string(), value.to_string())))
                        .collect::<HashMap<_, _>>();
                    let body = response.text()
                        .map_err(|error| JsError::new_from_js_message("response", "text", error.to_string()))?;
                    serde_json::to_string(&serde_json::json!({ "status": status, "headers": headers, "body": body }))
                        .map_err(|error| JsError::new_from_js_message("response", "JSON", error.to_string()))
                },
            )?,
        )?;

        let timer_state = bridge.clone();
        globals.set(
            "__host_timer_schedule",
            Function::new(
                ctx.clone(),
                move |plugin_id: String, id: i32, delay_ms: f64, repeat: bool| {
                    let delay_ms = if delay_ms.is_finite() {
                        delay_ms.clamp(0.0, 24.0 * 60.0 * 60.0 * 1000.0)
                    } else {
                        0.0
                    };
                    let millis = delay_ms.round() as u64;
                    let delay = Duration::from_millis(if repeat { millis.max(1) } else { millis });
                    if let Ok(mut state) = timer_state.lock() {
                        state.timers.insert(
                            id.max(0) as u64,
                            HostTimer {
                                plugin_id,
                                due: Instant::now() + delay,
                                delay,
                                repeat,
                            },
                        );
                    }
                },
            )?,
        )?;

        let timer_state = bridge.clone();
        globals.set(
            "__host_timer_cancel",
            Function::new(ctx.clone(), move |id: i32| {
                timer_state
                    .lock()
                    .map(|mut state| state.timers.remove(&(id.max(0) as u64)).is_some())
                    .unwrap_or(false)
            })?,
        )?;

        let timer_state = bridge.clone();
        globals.set(
            "__host_timer_cancel_plugin",
            Function::new(ctx.clone(), move |plugin_id: String| {
                if let Ok(mut state) = timer_state.lock() {
                    state
                        .timers
                        .retain(|_, timer| timer.plugin_id != plugin_id);
                }
            })?,
        )?;

        let permission_state = bridge.clone();
        globals.set(
            "__host_permission_check",
            Function::new(
                ctx.clone(),
                move |plugin_id: String, permission: String| {
                    permission_state
                        .lock()
                        .map(|state| {
                            state
                                .grants
                                .get(&plugin_id)
                                .is_some_and(|grants| grants.contains(&permission))
                        })
                        .unwrap_or(false)
                },
            )?,
        )?;

        globals.set(
            "__host_random_uuid",
            Function::new(ctx.clone(), || uuid::Uuid::new_v4().to_string()),
        )?;
        globals.set(
            "__host_sha256",
            Function::new(ctx.clone(), |value: String| format!("{:x}", Sha256::digest(value.as_bytes()))),
        )?;
        globals.set("__host_platform", Function::new(ctx.clone(), host_platform))?;
        globals.set("__host_arch", Function::new(ctx.clone(), host_arch))?;
        globals.set("__host_home_dir", Function::new(ctx.clone(), host_home_dir))?;
        globals.set("__host_temp_dir", Function::new(ctx.clone(), host_temp_dir))?;
        Ok(())
    }).map_err(JsRuntimeError::Js)
}

fn host_platform() -> String {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
    .to_string()
}

fn host_arch() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
    .to_string()
}

fn host_home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string())
}

fn host_temp_dir() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

fn builtin_module(name: &str) -> Option<&'static str> {
    match name {
        "@deepseek-ai/cordis" => Some(CORDIS_MODULE),
        "@deepseek-ai/dsh-tools" => Some(DSH_TOOLS_MODULE),
        "@deepseek-ai/dsh-llm" => Some(DSH_LLM_MODULE),
        "node:path" | "path" => Some(NODE_PATH_MODULE),
        "node:events" | "events" => Some(NODE_EVENTS_MODULE),
        "node:buffer" | "buffer" => Some(NODE_BUFFER_MODULE),
        "node:process" | "process" => Some(NODE_PROCESS_MODULE),
        "node:os" | "os" => Some(NODE_OS_MODULE),
        "node:fs" | "fs" => Some(NODE_FS_MODULE),
        "node:url" | "url" => Some(NODE_URL_MODULE),
        "node:util" | "util" => Some(NODE_UTIL_MODULE),
        "node:assert" | "assert" => Some(NODE_ASSERT_MODULE),
        "node:querystring" | "querystring" => Some(NODE_QUERYSTRING_MODULE),
        "node:crypto" | "crypto" => Some(NODE_CRYPTO_MODULE),
        "node:child_process" | "child_process" => Some(NODE_CHILD_PROCESS_MODULE),
        _ => None,
    }
}

fn resolve_module_path(
    modules: &SharedModules,
    base: &str,
    name: &str,
) -> Result<PathBuf, JsRuntimeError> {
    if name.contains('\0') {
        return Err(JsRuntimeError::ModuleResolution(
            "NUL in module name".into(),
        ));
    }
    let base = base.split('?').next().unwrap_or(base);
    let base_path = Path::new(base);
    let candidate = if Path::new(name).is_absolute() {
        resolve_candidate(Path::new(name))?
    } else if name.starts_with('.') {
        let parent = base_path.parent().ok_or_else(|| {
            JsRuntimeError::ModuleResolution(format!("module {base} has no parent"))
        })?;
        resolve_candidate(&parent.join(name))?
    } else {
        resolve_bare_package(modules, base_path, name)?
    };
    let canonical = candidate.canonicalize().map_err(JsRuntimeError::Io)?;
    if !modules.allowed(&canonical) {
        return Err(JsRuntimeError::ModuleResolution(format!(
            "module {} escapes registered plugin roots",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn resolve_candidate(path: &Path) -> Result<PathBuf, JsRuntimeError> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    for extension in ["js", "mjs", "cjs", "json"] {
        let candidate = path.with_extension(extension);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if path.is_dir() {
        if let Some(entry) = resolve_package_directory(path)? {
            return Ok(entry);
        }
        for name in ["index.js", "index.mjs", "index.cjs"] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(JsRuntimeError::ModuleResolution(format!(
        "module not found: {}",
        path.display()
    )))
}

fn resolve_bare_package(
    modules: &SharedModules,
    base: &Path,
    specifier: &str,
) -> Result<PathBuf, JsRuntimeError> {
    let (package_name, subpath) = split_package_specifier(specifier)?;
    let mut cursor = base.parent();
    while let Some(directory) = cursor {
        let package_root = directory.join("node_modules").join(&package_name);
        if package_root.is_dir()
            && modules.allowed(&package_root.canonicalize().map_err(JsRuntimeError::Io)?)
        {
            if let Some(subpath) = &subpath {
                return resolve_candidate(&package_root.join(subpath));
            }
            if let Some(entry) = resolve_package_directory(&package_root)? {
                return Ok(entry);
            }
        }
        cursor = directory.parent();
    }
    Err(JsRuntimeError::ModuleResolution(format!(
        "npm package not found: {specifier}"
    )))
}

fn split_package_specifier(specifier: &str) -> Result<(String, Option<String>), JsRuntimeError> {
    let parts = specifier.split('/').collect::<Vec<_>>();
    if specifier.starts_with('@') {
        if parts.len() < 2 || parts[0].len() < 2 || parts[1].is_empty() {
            return Err(JsRuntimeError::ModuleResolution(
                "invalid scoped package".into(),
            ));
        }
        Ok((
            format!("{}/{}", parts[0], parts[1]),
            (parts.len() > 2).then(|| parts[2..].join("/")),
        ))
    } else {
        if parts[0].is_empty() {
            return Err(JsRuntimeError::ModuleResolution("invalid package".into()));
        }
        Ok((
            parts[0].to_string(),
            (parts.len() > 1).then(|| parts[1..].join("/")),
        ))
    }
}

fn resolve_package_directory(root: &Path) -> Result<Option<PathBuf>, JsRuntimeError> {
    let package_json = root.join("package.json");
    if !package_json.is_file() {
        return Ok(None);
    }
    let source = fs::read_to_string(package_json).map_err(JsRuntimeError::Io)?;
    let manifest: Value = serde_json::from_str(&source)
        .map_err(|error| JsRuntimeError::ModuleResolution(error.to_string()))?;
    let entry = manifest
        .get("exports")
        .and_then(|exports| {
            exports
                .as_str()
                .or_else(|| exports.get(".").and_then(Value::as_str))
                .or_else(|| exports.get(".")?.get("import")?.as_str())
                .or_else(|| exports.get(".")?.get("default")?.as_str())
        })
        .or_else(|| manifest.get("module").and_then(Value::as_str))
        .or_else(|| manifest.get("main").and_then(Value::as_str));
    match entry {
        Some(entry) => resolve_candidate(&root.join(entry)).map(Some),
        None => Ok(None),
    }
}

fn canonical_entry(root: &Path, entry: &Path) -> Result<PathBuf, JsRuntimeError> {
    if entry.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(JsRuntimeError::InvalidPlugin(
            "entry must be plugin-relative".into(),
        ));
    }
    let entry = resolve_candidate(&root.join(entry))?;
    let canonical = entry.canonicalize().map_err(JsRuntimeError::Io)?;
    if !canonical.starts_with(root) {
        return Err(JsRuntimeError::InvalidPlugin(
            "entry escapes plugin root".into(),
        ));
    }
    Ok(canonical)
}

fn is_commonjs(path: &Path, source: &str) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("cjs")
        || ((source.contains("module.exports") || source.contains("exports."))
            && !source.contains("export ")
            && !source.contains("import "))
}

fn wrap_commonjs(source: &str) -> String {
    let requires = discover_literal_requires(source);
    let mut imports = String::new();
    let mut registrations = String::new();
    for (index, specifier) in requires.iter().enumerate() {
        let quoted = serde_json::to_string(specifier).unwrap_or_else(|_| "\"\"".into());
        imports.push_str(&format!("import * as __mh_cjs_{index} from {quoted};\n"));
        registrations.push_str(&format!(
            "__mh_require_modules.set({quoted}, __mh_cjs_{index});\n"
        ));
    }
    format!(
        "{imports}const __mh_require_modules = new Map();\n{registrations}\
function require(name) {{\n\
  const module = __mh_require_modules.get(String(name));\n\
  if (!module) throw new Error(`dynamic or unresolved CommonJS require is not supported: ${{name}}`);\n\
  const keys = Object.keys(module);\n\
  return keys.length === 1 && keys[0] === 'default' ? module.default : (module.default && module.default.__esModule !== true ? module.default : module);\n\
}}\n\
const module = {{ exports: {{}} }}; const exports = module.exports;\n\
(function(module, exports, require) {{\n{source}\n}})(module, exports, require);\n\
export default module.exports;"
    )
}

fn discover_literal_requires(source: &str) -> BTreeSet<String> {
    let mut modules = BTreeSet::new();
    for marker in ["require(", "require ("] {
        let mut offset = 0usize;
        while let Some(found) = source[offset..].find(marker) {
            let start = offset + found + marker.len();
            if let Some(value) = quoted_string_at(&source[start..]) {
                modules.insert(value);
            }
            offset = start;
            if offset >= source.len() {
                break;
            }
        }
    }
    modules
}

fn has_dynamic_require(source: &str) -> bool {
    let mut total = 0usize;
    let mut literal = 0usize;
    for marker in ["require(", "require ("] {
        let mut offset = 0usize;
        while let Some(found) = source[offset..].find(marker) {
            total += 1;
            let start = offset + found + marker.len();
            if quoted_string_at(&source[start..]).is_some() {
                literal += 1;
            }
            offset = start;
            if offset >= source.len() {
                break;
            }
        }
    }
    total > literal
}

fn bundle_runtime_id(root: &Path, entry_id: &str, name: &str) -> String {
    let digest =
        Sha256::digest(format!("{}\0{entry_id}\0{name}", root.to_string_lossy()).as_bytes());
    format!("dsh-{}", &format!("{digest:x}")[..20])
}

fn resolve_self_package_entry(
    root: &Path,
    specifier: &str,
) -> Result<Option<PathBuf>, JsRuntimeError> {
    let package_path = root.join("package.json");
    if !package_path.is_file() {
        return Ok(None);
    }
    let package: Value = serde_json::from_str(&fs::read_to_string(&package_path)?)
        .map_err(|error| JsRuntimeError::InvalidPlugin(error.to_string()))?;
    if package.get("name").and_then(Value::as_str) != Some(specifier) {
        return Ok(None);
    }
    let export = package
        .get("exports")
        .and_then(|exports| match exports {
            Value::String(value) => Some(value.as_str()),
            Value::Object(map) => map.get(".").and_then(|value| match value {
                Value::String(value) => Some(value.as_str()),
                Value::Object(conditions) => ["import", "default", "require"]
                    .into_iter()
                    .find_map(|key| conditions.get(key).and_then(Value::as_str)),
                _ => None,
            }),
            _ => None,
        })
        .or_else(|| package.get("module").and_then(Value::as_str))
        .or_else(|| package.get("main").and_then(Value::as_str));
    let candidates = export
        .map(|value| vec![value.to_string()])
        .unwrap_or_else(|| vec!["index.js".into(), "index.mjs".into(), "lib/index.js".into()]);
    for candidate in candidates {
        let path = root.join(candidate.trim_start_matches("./"));
        if path.is_file() {
            return path.canonicalize().map(Some).map_err(JsRuntimeError::Io);
        }
    }
    Ok(None)
}

fn merge_injects(module_inject: &[String], entry_inject: &[String]) -> Vec<String> {
    module_inject
        .iter()
        .chain(entry_inject.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn validate_plugin_id(value: &str) -> Result<(), JsRuntimeError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(JsRuntimeError::InvalidPlugin("invalid plugin id".into()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JsRuntimeError {
    #[error("JavaScript runtime error: {0}")]
    Js(#[from] JsError),
    #[error("plugin not found: {0}")]
    PluginNotFound(String),
    #[error("invalid plugin: {0}")]
    InvalidPlugin(String),
    #[error("module resolution failed: {0}")]
    ModuleResolution(String),
    #[error("runtime state lock poisoned")]
    Poisoned,
    #[error("transport error: {0}")]
    Transport(String),
    #[error("failed to apply loader update for {plugin_id}: {message}")]
    UpdateFailed { plugin_id: String, message: String },
    #[error(
        "failed to apply loader update for {plugin_id} ({candidate}); rollback also failed: {rollback}"
    )]
    UpdateRollback {
        plugin_id: String,
        candidate: String,
        rollback: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(root: &Path, file: &str, source: &str) {
        fs::create_dir_all(root).unwrap();
        fs::write(root.join(file), source).unwrap();
    }

    #[test]
    fn runs_deepseek_tool_plugin_without_node_or_cordis() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "plugin.mjs",
            r#"
import { defineTool } from '@deepseek-ai/dsh-tools';
export const name = 'greet-tool';
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'greet',
    description: 'Greet a person',
    parameters: { name: { type: 'string', required: true } },
    async execute(args) { return `Hello, ${args.name}!`; }
  }));
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        let state = host
            .register_plugin(
                "greet-tool",
                temp.path(),
                Path::new("plugin.mjs"),
                &serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(state, PluginState::Active);
        assert_eq!(host.registered_tools().unwrap(), vec!["greet"]);
        let result = host
            .call_tool_json("greet", &serde_json::json!({"name":"Cordis"}))
            .unwrap();
        assert_eq!(result, serde_json::json!("Hello, Cordis!"));
    }

    #[test]
    fn service_dependency_waits_and_reloads_when_provider_changes() {
        let provider = tempfile::tempdir().unwrap();
        write_plugin(
            provider.path(),
            "provider.mjs",
            r#"
import { Service } from '@deepseek-ai/cordis';
export default class Metrics extends Service {
  constructor(ctx) { super(ctx, 'metrics'); }
  record(value) { return `metric:${value}`; }
}
"#,
        );
        let consumer = tempfile::tempdir().unwrap();
        write_plugin(
            consumer.path(),
            "consumer.mjs",
            r#"
export const inject = ['metrics', 'tools'];
export function apply(ctx) {
  ctx.tools.register({
    name: 'metric',
    async execute(args) { return ctx.metrics.record(args.value); }
  });
}
"#,
        );

        let mut host = DeepSeekJsHost::new().unwrap();
        assert_eq!(
            host.register_plugin(
                "consumer",
                consumer.path(),
                Path::new("consumer.mjs"),
                &serde_json::json!({})
            )
            .unwrap(),
            PluginState::Pending
        );
        assert_eq!(
            host.register_plugin(
                "provider",
                provider.path(),
                Path::new("provider.mjs"),
                &serde_json::json!({})
            )
            .unwrap(),
            PluginState::Active
        );
        assert_eq!(host.plugin_state("consumer"), Some(PluginState::Active));
        assert_eq!(
            host.call_tool_json("metric", &serde_json::json!({"value":"one"}))
                .unwrap(),
            serde_json::json!("metric:one")
        );

        host.disable_plugin("provider").unwrap();
        assert_eq!(host.plugin_state("consumer"), Some(PluginState::Pending));
        assert!(
            !host
                .registered_tools()
                .unwrap()
                .contains(&"metric".to_string())
        );
        host.enable_plugin("provider").unwrap();
        assert_eq!(host.plugin_state("provider"), Some(PluginState::Active));
        assert_eq!(host.plugin_state("consumer"), Some(PluginState::Active));
    }

    #[test]
    fn effect_and_event_cleanup_are_scoped_to_plugin_lifecycle() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "events.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.on('custom/event', value => console.log(`seen:${value}`));
  ctx.effect(() => {
    console.log('effect:on');
    return () => console.log('effect:off');
  });
  ctx.tools.register({ name: 'emit-custom', async execute(args) { ctx.emit('custom/event', args.value); return 'ok'; } });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "events",
            temp.path(),
            Path::new("events.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        host.call_tool_json("emit-custom", &serde_json::json!({"value":"x"}))
            .unwrap();
        assert!(
            host.logs()
                .unwrap()
                .iter()
                .any(|log| log.message.contains("seen:x"))
        );
        host.disable_plugin("events").unwrap();
        assert!(
            host.logs()
                .unwrap()
                .iter()
                .any(|log| log.message.contains("effect:off"))
        );
        assert!(
            !host
                .registered_tools()
                .unwrap()
                .contains(&"emit-custom".to_string())
        );
    }

    #[test]
    fn runtime_inject_tracks_service_appearance_and_disappearance() {
        let consumer = tempfile::tempdir().unwrap();
        write_plugin(
            consumer.path(),
            "consumer.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.inject(['late'], child => {
    child.tools.register({ name: 'late-value', async execute() { return child.late.value; } });
  });
}
"#,
        );
        let provider = tempfile::tempdir().unwrap();
        write_plugin(
            provider.path(),
            "provider.mjs",
            r#"
import { Service } from '@deepseek-ai/cordis';
export default class LateService extends Service {
  constructor(ctx) { super(ctx, 'late'); this.value = 'available'; }
}
"#,
        );

        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "dynamic-consumer",
            consumer.path(),
            Path::new("consumer.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert!(
            !host
                .registered_tools()
                .unwrap()
                .contains(&"late-value".to_string())
        );

        host.register_plugin(
            "late-provider",
            provider.path(),
            Path::new("provider.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("late-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("available")
        );

        host.disable_plugin("late-provider").unwrap();
        assert!(
            !host
                .registered_tools()
                .unwrap()
                .contains(&"late-value".to_string())
        );
    }

    #[test]
    fn context_service_store_mixin_and_isolation_are_functional() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "context.mjs",
            r#"
import { Context, symbols } from '@deepseek-ai/cordis';
export const inject = ['tools'];
export function apply(ctx) {
  ctx.provide('counter', { base: 10, add(value) { return this.base + value; } });
  ctx.mixin('counter', ['add']);
  ctx.accessor('answer', { get() { return 42; } });
  const isolated = ctx.isolate('counter');
  isolated.provide('counter', { base: 100, add(value) { return this.base + value; } });
  ctx.tools.register({
    name: 'context-check',
    async execute() {
      return {
        mixed: ctx.add(5),
        isolated: isolated.counter.add(5),
        answer: ctx.answer,
        isContext: Context.is(ctx.root),
        symbolStable: Context.effect === symbols.effect
      };
    }
  });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "context-checker",
            temp.path(),
            Path::new("context.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("context-check", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!({
                "mixed": 15,
                "isolated": 105,
                "answer": 42,
                "isContext": true,
                "symbolStable": true
            })
        );
    }

    #[test]
    fn event_dispatch_modes_match_cordis_contract() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "events-api.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  let onceCount = 0;
  ctx.once('once-event', () => { onceCount += 1; });
  ctx.on('bail-event', () => false);
  ctx.on('bail-event', () => 'stop');
  ctx.on('bail-event', () => 'never');
  ctx.on('serial-event', async () => undefined);
  ctx.on('serial-event', async () => 'serial-stop');
  ctx.on('serial-event', async () => 'never');
  ctx.on('water-event', (value, next) => `a:${value}:${next()}`);
  ctx.on('water-event', (value, next) => `b:${value}:${next()}`);
  ctx.tools.register({
    name: 'dispatch-check',
    async execute() {
      ctx.emit('once-event');
      ctx.emit('once-event');
      return {
        onceCount,
        bail: ctx.bail('bail-event'),
        serial: await ctx.serial('serial-event'),
        waterfall: ctx.waterfall('water-event', 'x', () => 'end')
      };
    }
  });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "dispatch-checker",
            temp.path(),
            Path::new("events-api.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("dispatch-check", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!({
                "onceCount": 1,
                "bail": "stop",
                "serial": "serial-stop",
                "waterfall": "a:x:b:x:end"
            })
        );
    }

    #[test]
    fn deepseek_bundle_patch_discovers_and_runs_entries() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"name":"bundle-sample","dsh":{"bundle":{"patch":"./cordis.patch.yml"}}}"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("cordis.patch.yml"),
            r#"
plugins:
  - id: bundled-tool
    name: ./plugin.mjs
    config:
      value: from-bundle
    inject:
      - tools
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("plugin.mjs"),
            r#"
export function apply(ctx, config) {
  ctx.tools.register({ name: 'bundle-value', async execute() { return config.value; } });
}
"#,
        )
        .unwrap();

        let entries = discover_deepseek_bundle(temp.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "bundled-tool");
        let mut host = DeepSeekJsHost::new().unwrap();
        let states = host
            .register_deepseek_bundle_with_grants(temp.path(), &[])
            .unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, PluginState::Active);
        assert_eq!(
            host.call_tool_json("bundle-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("from-bundle")
        );
    }

    #[test]
    fn hmr_reload_uses_fresh_module_graph_revision() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "hmr.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register({ name: 'hmr-value', async execute() { return 'v1'; } });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "hmr-checker",
            temp.path(),
            Path::new("hmr.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("hmr-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("v1")
        );
        fs::write(
            temp.path().join("hmr.mjs"),
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register({ name: 'hmr-value', async execute() { return 'v2'; } });
}
"#,
        )
        .unwrap();
        assert_eq!(
            host.reload_plugin("hmr-checker").unwrap(),
            PluginState::Active
        );
        assert_eq!(
            host.call_tool_json("hmr-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("v2")
        );
    }

    #[test]
    fn loader_update_rolls_back_failed_candidate() {
        let stable = tempfile::tempdir().unwrap();
        write_plugin(
            stable.path(),
            "plugin.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register({ name: 'stable-value', async execute() { return 'stable'; } });
}
"#,
        );
        let broken = tempfile::tempdir().unwrap();
        write_plugin(
            broken.path(),
            "plugin.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register({ name: 'candidate-only', async execute() { return 'candidate'; } });
  throw new Error('candidate apply failure');
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "transactional",
            stable.path(),
            Path::new("plugin.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        let error = host
            .update_plugin(
                "transactional",
                broken.path(),
                Path::new("plugin.mjs"),
                &serde_json::json!({"candidate":true}),
                true,
                None,
            )
            .unwrap_err();
        assert!(matches!(error, JsRuntimeError::UpdateFailed { .. }));
        assert_eq!(
            host.plugin_state("transactional"),
            Some(PluginState::Active)
        );
        assert_eq!(
            host.call_tool_json("stable-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("stable")
        );
        assert!(
            !host
                .registered_tools()
                .unwrap()
                .contains(&"candidate-only".to_string())
        );
    }

    #[test]
    fn cordis_timer_promises_intervals_and_disposal_are_host_driven() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "timer.mjs",
            r#"
export const inject = ['tools', 'timer'];
export function apply(ctx) {
  ctx.tools.register({
    name: 'timer-check',
    async execute() {
      let ticks = 0;
      const dispose = ctx.interval(() => { ticks += 1; }, 2);
      await ctx.timeout(14);
      dispose();
      return { ticks, timerService: typeof ctx.timer.timeout === 'function' };
    }
  });
  ctx.timeout(() => console.log('should-not-fire-after-dispose'), 100);
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "timer-checker",
            temp.path(),
            Path::new("timer.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        let result = host
            .call_tool_json("timer-check", &serde_json::json!({}))
            .unwrap();
        assert!(result.get("ticks").and_then(Value::as_u64).unwrap_or(0) >= 2);
        assert_eq!(
            result.get("timerService").and_then(Value::as_bool),
            Some(true)
        );
        host.disable_plugin("timer-checker").unwrap();
        std::thread::sleep(Duration::from_millis(110));
        host.pump_timers().unwrap();
        assert!(
            !host
                .logs()
                .unwrap()
                .iter()
                .any(|log| log.message.contains("should-not-fire-after-dispose"))
        );
    }

    #[test]
    fn compatibility_scanner_distinguishes_portable_and_mobile_blockers() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "portable.mjs",
            "import path from 'node:path'; import crypto from 'node:crypto'; export default [path.sep, crypto.randomUUID];",
        );
        let report = scan_package_compatibility(temp.path()).unwrap();
        assert!(report.portable_compatible);
        assert!(report.mobile_compatible);
        assert_eq!(
            report.supported_modules,
            vec!["node:crypto".to_string(), "node:path".to_string()]
        );

        fs::write(
            temp.path().join("desktop.cjs"),
            "const cp = require('node:child_process'); module.exports = cp;",
        )
        .unwrap();
        fs::write(temp.path().join("addon.node"), b"native").unwrap();
        let report = scan_package_compatibility(temp.path()).unwrap();
        assert!(!report.mobile_compatible);
        assert!(
            report
                .desktop_only_modules
                .contains(&"node:child_process".to_string())
        );
        assert!(report.native_addons.contains(&"addon.node".to_string()));
        assert!(report.commonjs_require_files.is_empty());
    }

    #[test]
    fn rust_host_grants_gate_plugin_storage_calls() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "permissions.mjs",
            r#"
export const inject = ['tools', 'storage'];
export function apply(ctx) {
  ctx.tools.register({
    name: 'permission-storage',
    async execute() {
      ctx.storage.set('key', { ok: true });
      return ctx.storage.get('key');
    }
  });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin_with_grants(
            "permission-checker",
            temp.path(),
            Path::new("permissions.mjs"),
            &serde_json::json!({}),
            &[],
        )
        .unwrap();
        assert!(
            host.call_tool_json("permission-storage", &serde_json::json!({}))
                .is_err()
        );
        host.set_plugin_grants("permission-checker", ["storage.local".to_string()])
            .unwrap();
        assert_eq!(
            host.call_tool_json("permission-storage", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!({"ok": true})
        );
    }

    #[test]
    fn static_commonjs_require_runs_without_node() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("helper.cjs"),
            "module.exports = { suffix: 'ok' };",
        )
        .unwrap();
        fs::write(
            temp.path().join("plugin.cjs"),
            r#"
const path = require('node:path');
const helper = require('./helper.cjs');
module.exports = {
  inject: ['tools'],
  apply(ctx) {
    ctx.tools.register({
      name: 'cjs-check',
      async execute() { return path.join('cjs', helper.suffix); }
    });
  }
};
"#,
        )
        .unwrap();
        let report = scan_package_compatibility(temp.path()).unwrap();
        assert!(report.commonjs_require_files.is_empty());
        assert!(report.portable_compatible);
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "cjs-checker",
            temp.path(),
            Path::new("plugin.cjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("cjs-check", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("cjs/ok")
        );
    }

    #[test]
    fn common_node_compat_modules_run_inside_rust_host() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "node-compat.mjs",
            r#"
import path from 'node:path';
import crypto from 'node:crypto';
import util from 'node:util';
import querystring from 'node:querystring';
import assert from 'node:assert';
export const inject = ['tools'];
export function apply(ctx) {
  ctx.tools.register({
    name: 'node-compat',
    async execute() {
      assert.strictEqual(path.join('a', 'b'), 'a/b');
      const digest = crypto.createHash('sha256').update('abc').digest('hex');
      const query = querystring.stringify({ a: 1, b: 'x' });
      return { digest, query, text: util.format('ok', 1), uuidLength: crypto.randomUUID().length };
    }
  });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "node-compat-checker",
            temp.path(),
            Path::new("node-compat.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        let result = host
            .call_tool_json("node-compat", &serde_json::json!({}))
            .unwrap();
        assert_eq!(
            result.get("digest").and_then(Value::as_str),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(result.get("query").and_then(Value::as_str), Some("a=1&b=x"));
        assert_eq!(result.get("text").and_then(Value::as_str), Some("ok 1"));
        assert_eq!(result.get("uuidLength").and_then(Value::as_u64), Some(36));
    }

    #[test]
    fn child_fiber_restart_and_update_replace_scoped_effects() {
        let temp = tempfile::tempdir().unwrap();
        write_plugin(
            temp.path(),
            "fiber.mjs",
            r#"
export const inject = ['tools'];
export function apply(ctx) {
  const child = ctx.plugin({
    name: 'child',
    apply(childCtx, config) {
      childCtx.tools.register({ name: 'child-value', async execute() { return config.value; } });
    }
  }, { value: 'a' });
  ctx.tools.register({
    name: 'update-child',
    async execute(args) {
      await child.update({ value: args.value });
      return child.state;
    }
  });
}
"#,
        );
        let mut host = DeepSeekJsHost::new().unwrap();
        host.register_plugin(
            "fiber-checker",
            temp.path(),
            Path::new("fiber.mjs"),
            &serde_json::json!({}),
        )
        .unwrap();
        assert_eq!(
            host.call_tool_json("child-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("a")
        );
        assert_eq!(
            host.call_tool_json("update-child", &serde_json::json!({"value":"b"}))
                .unwrap(),
            serde_json::json!("active")
        );
        assert_eq!(
            host.call_tool_json("child-value", &serde_json::json!({}))
                .unwrap(),
            serde_json::json!("b")
        );
    }
}
