// Clock wakeups only. The Rust automation domain and D1 remain authoritative.
import RustEntrypoint from './build/worker/shim.mjs';

const CAPABILITY = '__FABUSHI_CLOCK_CAPABILITY';
const USER_HEADER = 'x-fabushi-automation-clock-user';
const AT_HEADER = 'x-fabushi-automation-clock-at';
const validUser = value => typeof value === 'string' && /^[A-Za-z0-9._:-]{1,200}$/.test(value);

// worker-build 0.8.5 exports a WorkerEntrypoint class, not a handler object.
// Inherit scheduled/queue/email and the generated Wasm recovery proxy.
function dispatch(request, env, context) {
  return RustEntrypoint.prototype.fetch.call({ env, ctx: context }, request);
}
export default class FabushiNativeEntrypoint extends RustEntrypoint {
  async fetch(request) {
    const env = this.env;
    const context = this.ctx;
    // No deployment variable or incoming HTTP header may supply the private
    // per-invocation capability used by the object alarm below.
    const { [CAPABILITY]: _discarded, ...publicEnv } = env;
    const headers = new Headers(request.headers);
    headers.delete('x-fabushi-clock-proof');
    const result = await dispatch(new Request(request, { headers }), publicEnv, context);
    const user = result.headers.get(USER_HEADER);
    if (user === null) return result;
    if (!validUser(user) || !env.FABUSHI_AUTOMATION_CLOCK) return unavailable();
    // An arm asks for reconciliation, never installs a client-provided cursor.
    // This also repairs a prior commit whose response/arm was lost.
    const stub = env.FABUSHI_AUTOMATION_CLOCK.get(env.FABUSHI_AUTOMATION_CLOCK.idFromName(user));
    let armed;
    try { armed = await stub.fetch(new Request('https://clock.internal/arm', { method: 'POST', body: JSON.stringify({ userId: user }) })); }
    catch { return unavailable(); }
    if (!armed.ok) return unavailable();
    const publicHeaders = new Headers(result.headers);
    publicHeaders.delete(USER_HEADER); publicHeaders.delete(AT_HEADER);
    return new Response(result.body, { status: result.status, statusText: result.statusText, headers: publicHeaders });
  }
}

function unavailable() {
  return new Response(JSON.stringify({ error: 'automation_clock_unavailable', message: 'Reconcile the persisted automation before retrying; publication may have committed.' }), {
    status: 503, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' },
  });
}

export class FabushiAutomationClock {
  constructor(state, env) { this.state = state; this.env = env; }
  async fetch(request) {
    if (request.method !== 'POST' || new URL(request.url).pathname !== '/arm') return new Response(null, { status: 405 });
    const text = await request.text();
    if (text.length > 512) return new Response(null, { status: 400 });
    let input; try { input = JSON.parse(text); } catch { return new Response(null, { status: 400 }); }
    if (!validUser(input.userId) || Object.keys(input).length !== 1) return new Response(null, { status: 400 });
    const user = await this.state.storage.get('userId');
    if (user !== undefined && user !== input.userId) return new Response(null, { status: 409 });
    await this.state.storage.put('userId', input.userId);
    const next = Date.now() + 1000;
    const existing = await this.state.storage.getAlarm();
    if (existing === null || existing > next) await this.state.storage.setAlarm(next);
    return new Response(null, { status: 204 });
  }
  async alarm() {
    const userId = await this.state.storage.get('userId');
    if (!validUser(userId)) throw Error('native_clock_identity_missing');
    const capability = crypto.randomUUID();
    const response = await dispatch(new Request('https://clock.internal/internal/native-automation-clock/tick', {
      method: 'POST', headers: { 'content-type': 'application/json', 'x-fabushi-clock-proof': capability }, body: JSON.stringify({ userId }),
    }), { ...this.env, [CAPABILITY]: capability }, { waitUntil: promise => this.state.waitUntil(promise) });
    if (!response.ok) throw Error('native_clock_tick_failed');
    const result = await response.json();
    if (result.protocol !== 'fabushi.automation.definition.v1') throw Error('native_clock_protocol_invalid');
    if (result.nextWakeAtMs !== null) {
      if (!Number.isSafeInteger(result.nextWakeAtMs) || result.nextWakeAtMs < 0) throw Error('native_clock_position_invalid');
      const next = Math.max(Date.now() + 100, result.nextWakeAtMs);
      const existing = await this.state.storage.getAlarm();
      if (existing === null || existing <= Date.now() || next < existing) await this.state.storage.setAlarm(next);
    }
  }
}
