'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const loadTs = require('./load-ts.cjs');
const { PersistStore, consumePgConfig, clampLimit } = loadTs('src/main/db.ts');

const env = () => ({ MD_PG_HOST:'localhost', MD_PG_PORT:'5432', MD_PG_DATABASE:'app', MD_PG_USER:'app', MD_PG_PASSWORD:'secret', MD_PG_NAMESPACE:'operator-1' });

function mockPool(options = {}) {
  const calls = [];
  const client = {
    async query(sql, values) { calls.push({ source:'client', sql, values }); return { rows:[{ locked:true }], rowCount:1 }; },
    release(force) { calls.push({ source:'client', sql:'release', force }); }
  };
  const pool = {
    async connect() { if (options.connectError) throw new Error('offline'); calls.push({ source:'pool', sql:'connect' }); return client; },
    async query(sql, values) {
      calls.push({ source:'pool', sql, values });
      if (sql.includes('schema_migrations')) return { rows:[{ version: options.version ?? 1 }], rowCount:1 };
      if (sql.includes('SELECT key, value')) return { rows: options.kv ?? [], rowCount:0 };
      if (options.retryOnce && sql.includes('INSERT INTO') && !options.retried) { options.retried=true; const e=new Error('retry'); e.code='40001'; throw e; }
      if (sql.includes('command_history') && sql.includes('SELECT')) return { rows:[], rowCount:0 };
      return { rows:[], rowCount:1 };
    },
    async end() { calls.push({ source:'pool', sql:'end' }); }
  };
  return { pool, client, calls };
}

test('missing config is explicit degraded state and consumes no generic env', async () => {
  const store = new PersistStore({ env:{} });
  assert.deepEqual(await store.open(), { state:'degraded', writes:false, code:'missing_config' });
  assert.equal(store.setKv('x',1), false);
});

test('password and all MD_PG connection values are removed from inherited env', () => {
  const e=env(); const config=consumePgConfig(e);
  assert.ok(config); assert.equal(Object.keys(e).some((key)=>key.startsWith('MD_PG_')), false);
});

test('unreachable and schema mismatch degrade without fallback', async () => {
  const offline=mockPool({connectError:true});
  assert.equal((await new PersistStore({env:env(),pool:offline.pool}).open()).code,'unreachable');
  const old=mockPool({version:0});
  assert.equal((await new PersistStore({env:env(),pool:old.pool}).open()).code,'schema_mismatch');
});

test('KV cache hydrates and ordered idempotent queue retries serialization once', async () => {
  const m=mockPool({kv:[{key:'saved',value:{ok:true}}],retryOnce:true});
  const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  assert.deepEqual(store.getKv('saved'),{ok:true});
  store.setKv('a',1); store.setKv('a',2); assert.equal(store.getKv('a'),2);
  assert.equal(await store.drain(),true);
  const inserts=m.calls.filter((c)=>c.sql.includes('INSERT INTO munder_difflin.kv'));
  assert.equal(inserts.length,3); assert.equal(inserts.at(-1).values[2],'2');
});

test('history search is parameterized and limit is bounded', async () => {
  const m=mockPool(); const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  await store.searchHistory("%' OR true --",50000);
  const call=m.calls.find((c)=>c.sql.includes('text ILIKE $2'));
  assert.equal(call.values[1],"%\\%' OR true --%"); assert.equal(call.values[2],1000);
  assert.equal(clampLimit(-1,50),50);
});

test('terminal queue failure is degraded and drain reports false', async () => {
  const m=mockPool(); const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  m.pool.query=async(sql,values)=>{ m.calls.push({source:'pool',sql,values}); throw Object.assign(new Error('terminal'),{code:'23505'}); };
  assert.equal(store.setKv('x',1),true); assert.equal(await store.drain(),false);
  assert.deepEqual(store.status,{state:'degraded',writes:false,code:'write_failed'});
  assert.equal(store.setKv('y',2),false);
  await store.close();
  assert.equal(m.calls.some((c)=>c.sql==='release'&&c.force===true),true);
  assert.equal(m.calls.some((c)=>c.sql==='end'),true);
});

test('unreadable remote CA degrades without constructor throw', async () => {
  const e=env(); e.MD_PG_HOST='db.example.invalid'; e.MD_PG_TLS_CA='/not/present/ca.pem';
  const store=new PersistStore({env:e});
  assert.equal((await store.open()).code,'config_invalid');
  assert.equal(Object.keys(e).some((key)=>key.startsWith('MD_PG_')),false);
});

test('namespace reset is transactional and close drains before lock release and pool end', async () => {
  const m=mockPool(); const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  store.setKv('a',1); assert.equal(await store.resetNamespace(),true); await store.close();
  for (const table of ['cost_ledger','command_history','kv']) {
    const call=m.calls.find((c)=>c.sql.includes(`DELETE FROM munder_difflin.${table}`));
    assert.deepEqual(call.values,['operator-1']);
  }
  const insert=m.calls.findIndex((c)=>c.sql.includes('INSERT INTO munder_difflin.kv'));
  const unlock=m.calls.findIndex((c)=>c.sql.includes('pg_advisory_unlock'));
  const end=m.calls.findIndex((c)=>c.sql==='end');
  assert.ok(insert < unlock && unlock < end);
});

test('reset atomically rejects writes before draining', async () => {
  const m=mockPool(); const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  const resetting=store.resetNamespace();
  assert.equal(store.setKv('late',1),false);
  assert.equal(await resetting,true);
});

test('lifetime SQL preserves append identity order, not sample timestamp order', async () => {
  const m=mockPool(); const store=new PersistStore({env:env(),pool:m.pool}); await store.open();
  await store.lifetimeCostTotals();
  const call=m.calls.find((c)=>c.sql.includes('WITH ordered AS'));
  assert.match(call.sql,/ORDER BY id/);
  assert.doesNotMatch(call.sql,/ORDER BY occurred_at/);
});
