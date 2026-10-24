// Actual canonical wasm routes + disposable local D1. No production credentials or deployment.
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { generateKeyPairSync, randomUUID, sign, createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { spawn, execFileSync } from 'node:child_process';
if (process.env.GITHUB_ACTIONS !== 'true' || !process.env.RUNNER_TEMP || process.env.CLOUDFLARE_API_TOKEN || process.env.CF_API_TOKEN) throw Error('isolated_actions_required');
const dir = join(process.env.RUNNER_TEMP, `native-automation-${process.env.GITHUB_RUN_ID}-${process.env.GITHUB_RUN_ATTEMPT}`);
mkdirSync(dir, { recursive: true, mode: 0o700 });
const wrangler = join(process.env.RUNNER_TEMP, 'automation-fixture-tools/node_modules/.bin/wrangler');
const configPath = join(dir, 'wrangler.json');
const state = join(dir, 'state');
const workerRoot = resolve('third_party/mahayana/mahayana-rs/mahayana-platform-worker');
const wasmPath = resolve('deploy/platform-worker/build/worker/index.wasm');
const { privateKey, publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
writeFileSync(configPath, JSON.stringify({ name: 'native-automation-journal-test',
  main: resolve('deploy/platform-worker/native-automation-entry.mjs'), compatibility_date: '2026-07-18',
  durable_objects: { bindings: [{ name: 'FABUSHI_AUTOMATION_CLOCK', class_name: 'FabushiAutomationClock' }] },
  migrations: [{ tag: 'fabushi-automation-clock-v1', new_sqlite_classes: ['FabushiAutomationClock'] }],
  vars: { DEFAULT_AI_TOKEN_LIMIT: "1000", ACCESS_TOKEN_PUBLIC_KEY_PEM: publicKey.export({ type: 'spki', format: 'pem' }) },
  d1_databases: [
    { binding: 'ACCOUNT_DB', database_name: 'automation-auth', database_id: randomUUID() },
    { binding: 'PLATFORM_DB', database_name: 'automation-platform', database_id: randomUUID() },
  ],
}), { mode: 0o600 });
function sql(binding, text) {
  const filename = join(dir, `${binding}.sql`);
  writeFileSync(filename, text, { mode: 0o600 });
  const raw = execFileSync(wrangler, ['d1', 'execute', binding, '--local', '--persist-to', state, '--config', configPath, '--json', '--file', filename], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 60000 });
  const result = JSON.parse(raw);
  assert.ok(result.every(v => v.success));
  return result.flatMap(v => v.results);
}
const now = Math.floor(Date.now() / 1000);
function token(user, sid, extra = {}) {
  const header = Buffer.from(JSON.stringify({ alg: 'RS256', typ: 'JWT' })).toString('base64url');
  const payload = Buffer.from(JSON.stringify({ iss: 'https://api.ombhrum.com', aud: 'mahayana-platform',
    sub: user, sid, scope: [], device_id: 'fixture-device', jti: randomUUID(), iat: now, exp: now + 1800, token_use: 'access', ...extra })).toString('base64url');
  const signed = `${header}.${payload}`;
  return `${signed}.${sign('RSA-SHA256', Buffer.from(signed), privateKey).toString('base64url')}`;
}
const a = token('a','session-a');
const b = token('b','session-b');
const missing = token('a','missing-session');
const expiredSession = token('a','expired-session');
let worker;
let startupOutput='';
const assertions = [];
const evidence = { kind: 'native-automation-actual-wasm-local-d1', sourceSha: process.env.SUBJECT_SHA,
  runId: process.env.GITHUB_RUN_ID, runAttempt: process.env.GITHUB_RUN_ATTEMPT,
  wasmSha256: createHash('sha256').update(readFileSync(wasmPath)).digest('hex'),
  accountFixture: 'ephemeral locally-signed RS256 plus actual account_sessions; does not exercise login issuance',
  deployedCloud: 'NOT_VERIFIED', packagedDesktop: 'NOT_VERIFIED', assertions, overall: 'FAIL' };
async function start() {
  startupOutput='';
  worker = spawn(wrangler, ['dev','--local','--port','8787','--ip','127.0.0.1','--persist-to',state,'--config',configPath], { stdio: ['ignore','pipe','pipe'], detached: true });
  const capture=data=>{if(startupOutput.length<65536)startupOutput+=data.toString();};
  worker.stdout.on('data',capture);worker.stderr.on('data',capture);
  for (let n=0; n<60; n++) {
    if (worker.exitCode != null) throw Error('fixture_worker_exited');
    try { if ((await fetch('http://127.0.0.1:8787/health', { signal: AbortSignal.timeout(1000) })).ok) return; } catch {}
    await new Promise(r=>setTimeout(r,1000));
  }
  // Diagnostic messages only, never fixture keys, token values or config dumps.
  const safe=startupOutput.replaceAll(publicKey.export({type:'spki',format:'pem'}),'[public-key]')
    .split('\n').filter(line=>/error|failed|cannot|could|exception|invalid|no matching export|not found/i.test(line)
      && !/PRIVATE_KEY|PUBLIC_KEY|ACCESS_TOKEN|authorization|bearer/i.test(line)).slice(-20);
  console.error(safe.join('\n'));
  throw Error('fixture_worker_start_timeout');
}
async function stop() {
  if (!worker || worker.exitCode != null || worker.signalCode != null) return;
  const owned = worker;
  const exited = new Promise(resolve => owned.once('exit',resolve));
  // The detached process group was created by this exact child launch. Verify
  // its leader before signalling, and never clean a foreign name/PID set.
  const pgid = execFileSync('ps',['-o','pgid=','-p',String(owned.pid)],{encoding:'utf8'}).trim();
  assert.equal(pgid,String(owned.pid),'fixture ownership changed');
  process.kill(-owned.pid,'SIGTERM');
  let timeout;
  try { await Promise.race([exited,new Promise((_,reject)=>{timeout=setTimeout(()=>reject(Error('fixture_worker_stop_timeout')),10000);})]); }
  finally { clearTimeout(timeout); }
  let stopped = false;
  for (let n=0; n<10; n++) {
    try { await fetch('http://127.0.0.1:8787/health',{signal:AbortSignal.timeout(300)}); }
    catch { stopped = true; break; }
    await new Promise(r=>setTimeout(r,200));
  }
  assert.ok(stopped,'owned Worker remained reachable after stop; restart cannot be claimed');
}
async function call(path, bearer=a, body, method=body===undefined?'GET':'POST') {
  const r=await fetch(`http://127.0.0.1:8787${path}`, { method, redirect:'manual',
    headers: { authorization:`Bearer ${bearer}`, 'content-type':'application/json' },
    body:body===undefined?undefined:JSON.stringify(body), signal:AbortSignal.timeout(15000) });
  const value=await r.json();
  return { status:r.status, value };
}
function pass(name) { assertions.push(name); console.log(`PASS ${name}`); }
const event={ runUuid:'run-1',sandAgentId:'agent-1',automationId:'automation-1',timestampMs:100 };
const poll='/v1/automation-events/poll';
const complete='/v1/automation-runs/complete';
try {
  sql('ACCOUNT_DB',readFileSync(join(workerRoot,'account-migrations/0001_account_auth.sql'),'utf8'));
  sql('ACCOUNT_DB',`INSERT INTO account_sessions VALUES
  ('session-a','family-a','a','fixture-device','refresh-a',${now},${now},${now+3600},NULL,NULL),
  ('session-b','family-b','b','fixture-device','refresh-b',${now},${now},${now+3600},NULL,NULL),
  ('expired-session','expired-family','a','fixture-device','expired-refresh',${now-100},${now-100},${now-1},NULL,NULL);`);
  sql('PLATFORM_DB',readFileSync(join(workerRoot,'migrations/0001_platform.sql'),'utf8'));
  sql('PLATFORM_DB',readFileSync(join(workerRoot,'migrations/0035_automation_runs.sql'),'utf8'));
  sql('PLATFORM_DB',readFileSync(join(workerRoot,'migrations/0036_automation_definitions.sql'),'utf8'));
  const windowStart=Math.floor(now/(30*86400))*(30*86400);
  sql('PLATFORM_DB',`INSERT INTO ai_usage_budgets VALUES
    ('a',${windowStart},${windowStart+30*86400},1000,250,100,${now}),
    ('b',${windowStart},${windowStart+30*86400},1000,50,0,${now});`);
  await start();
  for (const bad of ['invalid',missing,expiredSession,token('a','session-b'),token('a','session-a',{iss:'https://foreign.invalid'})]) {
    assert.equal((await call('/v1/automation-runs',bad,event)).status,401);
    assert.equal((await call('/v1/ai/usage',bad)).status,401);
  }
  pass('actual signature, issuer, session existence, expiry and account binding deny invalid requests');
  const usageA=await call('/v1/ai/usage',a); const usageB=await call('/v1/ai/usage',b);
  assert.equal(usageA.status,200); assert.equal(usageA.value.usedTokens,250); assert.equal(usageA.value.remainingTokens,650);
  assert.equal(usageB.status,200); assert.equal(usageB.value.usedTokens,50); assert.equal(usageB.value.remainingTokens,950);
  pass('native account usage projects distinct authoritative token budgets without cross-account data');
  assert.equal((await call('/v1/automation-runs',a,{...event,extra:'invalid'})).status,400);
  assert.equal((await call('/v1/automation-runs',a,event)).status,200);
  assert.equal((await call('/v1/automation-runs',a,event)).status,200);
  assert.equal((await call('/v1/automation-runs',a,{...event,automationId:'different'})).status,409);
  pass('enqueue identical retry is idempotent and conflicting UUID is rejected');
  const before=await call(poll,a,{ackRunUuids:[]});
  assert.equal(before.status,200); assert.equal(before.value.events[0].id,'run-1');
  assert.equal((await call(poll,a,{ackRunUuids:['run-1']})).status,409);
  assert.equal((await call(poll,a,{ackRunUuids:[]})).value.events.length,1);
  pass('pending ACK is refused without losing pending delivery');
  assert.equal((await call('/v1/automation-runs/run-1',b)).status,404);
  assert.equal((await call(complete,b,{runUuid:'run-1',status:'succeeded'})).status,404);
  assert.equal((await call(poll,b,{ackRunUuids:['run-1']})).status,409);
  pass('foreign account cannot read, complete or ACK another account run');
  // Drop the client connection only after the real upstream service has
  // durably completed. The caller receives no successful response or receipt.
  let lostReceipt; let forwarded=0; let lossFailure;
  const lossProxy=createServer(async (req,res)=>{
    try {
      const chunks=[]; for await (const chunk of req) chunks.push(chunk);
      forwarded++;
      const upstream=await fetch(`http://127.0.0.1:8787${complete}`,{
        method:'POST',headers:{authorization:`Bearer ${a}`,'content-type':'application/json'},
        body:Buffer.concat(chunks),signal:AbortSignal.timeout(10000),
      });
      assert.equal(upstream.status,200);
      lostReceipt=(await upstream.json()).receipt;
      res.destroy();
    } catch(error) { lossFailure=error; res.destroy(); }
  });
  try {
    await new Promise((resolve,reject)=>{lossProxy.once('error',reject);lossProxy.listen(0,'127.0.0.1',resolve);});
    await assert.rejects(fetch(`http://127.0.0.1:${lossProxy.address().port}/complete`,{
      method:'POST',headers:{'content-type':'application/json'},
      body:JSON.stringify({runUuid:'run-1',status:'succeeded'}),signal:AbortSignal.timeout(15000),
    }));
    assert.equal(lossFailure,undefined); assert.equal(forwarded,1);
  } finally {
    lossProxy.closeAllConnections();
    await new Promise(resolve=>lossProxy.close(resolve));
  }
  const done=await call('/v1/automation-runs/run-1',a);
  assert.equal(done.status,200); assert.match(done.value.receipt,/^[a-f0-9]{64}$/);
  assert.equal(done.value.receipt,lostReceipt);
  const retry=await call(complete,a,{runUuid:'run-1',status:'succeeded'});
  assert.equal(retry.status,200); assert.equal(retry.value.receipt,done.value.receipt); assert.equal(retry.value.completedAt,done.value.completedAt);
  assert.equal((await call(complete,a,{runUuid:'run-1',status:'failed',errorMessage:'conflict'})).status,409);
  assert.equal((await call(complete,a,{runUuid:'absent',status:'succeeded'})).status,404);
  pass('connection dropped after server completion; retry preserves receipt, conflicts and missing runs stay errors');
  assert.equal((await call(poll,a,{ackRunUuids:['run-1','missing']})).status,409);
  assert.equal((await call(poll,a,{ackRunUuids:[]})).value.events.length,1);
  assert.equal((await call(poll,a,{ackRunUuids:['run-1']})).value.events.length,0);
  pass('mixed valid-invalid ACK changes nothing, valid terminal ACK drains delivery');
  await stop(); await start();
  const restored=await call('/v1/automation-runs/run-1');
  assert.equal(restored.status,200); assert.equal(restored.value.receipt,done.value.receipt); assert.ok(restored.value.acknowledgedAt);
  assert.equal((await call(poll,a,{ackRunUuids:[]})).value.events.length,0);
  assert.equal((await call('/v1/automation-runs',a,{...event,automationId:'different'})).status,409);
  pass('actual Worker restart retains completion receipt, ACK and conflicting-UUID tombstone');
  const definition={id:'clock-1',agentId:'agent-1',name:'Native clock',description:null,enabled:true,
    workflow:{triggers:[{trigger:{Cron:{cron:'@every 1s'}}}],prompts:[{prompt:'isolated clock fixture'}]}};
  const putDefinition=(value=definition,precondition={expectAbsent:true})=>call(`/v1/automations/${value.id}`,a,{definition:value,...precondition},'PUT');
  const published=await putDefinition();assert.equal(published.status,200);
  const revision=published.value.record.revision;
  assert.equal((await putDefinition()).value.record.revision,revision,'identical retry preserves revision and cursor');
  assert.equal((await putDefinition({...definition,name:'conflicting'})).status,409);
  assert.equal((await call('/v1/automations/clock-1',b)).status,404);
  assert.equal((await call('/v1/automations?agentId=agent-1',b)).value.records.length,0);
  assert.equal((await call('/internal/native-automation-clock/tick',a,{userId:'a'})).status,401);
  const forged=await fetch('http://127.0.0.1:8787/internal/native-automation-clock/tick',{method:'POST',
    headers:{'content-type':'application/json','x-fabushi-clock-proof':'x'.repeat(36)},body:JSON.stringify({userId:'a'})});
  assert.equal(forged.status,401);
  pass('native definitions have account isolation, CAS receipts, and unforgeable internal clock access');
  const scheduledCount=()=>sql('PLATFORM_DB',"SELECT count(*) n FROM automation_runs WHERE user_id='a' AND json_extract(event_json,'$.automationId')='clock-1'")[0].n;
  for(let i=0;i<40&&scheduledCount()<2;i++) await new Promise(r=>setTimeout(r,250));
  assert.ok(scheduledCount()>=2,'real one-second Durable Object alarms must enqueue occurrences');
  await Promise.all(Array.from({length:8},()=>call('/v1/automations/clock-1',a)));
  const beforeClockRestart=scheduledCount();
  await stop(); await new Promise(r=>setTimeout(r,1200)); await start();
  for(let i=0;i<40&&scheduledCount()<=beforeClockRestart;i++) await new Promise(r=>setTimeout(r,250));
  assert.ok(scheduledCount()>beforeClockRestart,'persisted alarm/cursor must recover after actual shutdown');
  const scheduledRows=sql('PLATFORM_DB',"SELECT run_uuid,event_json FROM automation_runs WHERE user_id='a' AND json_extract(event_json,'$.automationId')='clock-1'");
  const slots=scheduledRows.map(row=>{const event=JSON.parse(row.event_json);assert.equal(event.definitionRevision,revision);assert.equal(event.event,null);return `${event.scheduledForMs}`;});
  assert.equal(new Set(slots).size,slots.length,'concurrent arms and restart cannot duplicate occurrences');
  pass('actual one-second Durable Object alarm, concurrent arms and process restart retain unique clock occurrences');
  const disabled=await putDefinition({...definition,enabled:false},{baseRevision:revision});assert.equal(disabled.status,200);
  const countAfterDisable=scheduledCount();await new Promise(r=>setTimeout(r,1400));assert.equal(scheduledCount(),countAfterDisable);
  const deleted=await call('/v1/automations/clock-1',a,{baseRevision:disabled.value.record.revision},'DELETE');assert.equal(deleted.status,200);
  assert.equal(deleted.value.record.deleted,true);
  assert.equal((await call('/v1/automations/clock-1',a,{baseRevision:disabled.value.record.revision},'DELETE')).value.record.revision,deleted.value.record.revision);
  assert.equal((await putDefinition(definition,{baseRevision:revision})).status,409,'stale pre-delete revision cannot resurrect a definition');
  assert.equal((await putDefinition({...definition,agentId:'different'},{baseRevision:deleted.value.record.revision})).status,409,'agent identity cannot move');
  pass('disable fences stale clocks, delete retains stable tombstones and stale/identity-moving writes fail');
  const quotaDefinition={...definition,id:'clock-quota'};
  const quotaPublished=await putDefinition(quotaDefinition);assert.equal(quotaPublished.status,200);
  sql('PLATFORM_DB',`WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x < 20000-(SELECT count(*) FROM automation_runs WHERE user_id='a'))
    INSERT INTO automation_runs(user_id,run_uuid,event_json,event_digest,created_at)
    SELECT 'a','quota-fixture-'||x,'{}','quota',0 FROM n;`);
  const quotaCursor=(await call('/v1/automations/clock-quota',a)).value.record.nextRunsMs;
  await new Promise(r=>setTimeout(r,1500));
  assert.deepEqual((await call('/v1/automations/clock-quota',a)).value.record.nextRunsMs,quotaCursor,'quota must not advance or lose a due occurrence');
  assert.equal(sql('PLATFORM_DB',"SELECT count(*) n FROM automation_runs WHERE user_id='a'")[0].n,20000);
  assert.equal((await call('/v1/automations/clock-quota',a,{baseRevision:quotaPublished.value.record.revision},'DELETE')).status,200);
  pass('actual D1 journal capacity preserves the blocked cursor instead of skipping its occurrence');
  assert.equal((await call('/api/auth/logout',a,{})).status,200);
  assert.equal((await call('/v1/automation-runs/run-1',a)).status,401);
  assert.equal((await call('/v1/ai/usage',a)).status,401);
  assert.equal((await call('/v1/automations',a)).status,401);
  assert.equal((await putDefinition()).status,401);
  assert.equal((await call('/v1/ai/usage',b)).value.usedTokens,50);
  assert.equal((await call(poll,a,{ackRunUuids:[]})).status,401);
  assert.equal((await call(complete,a,{runUuid:'run-1',status:'succeeded'})).status,401);
  assert.equal((await call('/v1/automation-runs',b,event)).status,200);
  pass('real logout revokes journal access and leaves another account session functional');
  evidence.overall='PASS';
} finally {
  try { await stop(); } finally {
    mkdirSync('evidence',{recursive:true});
    writeFileSync('evidence/native-automation-runtime.json',JSON.stringify(evidence,null,2));
    rmSync(dir,{recursive:true,force:true});
  }
}
