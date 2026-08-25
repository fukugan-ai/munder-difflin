'use strict';
const fs = require('node:fs');
const readline = require('node:readline');
const crypto = require('node:crypto');
const { Pool } = require('pg');
const { consumeConfig } = require('./db-env.cjs');

const eventId = (sourceId, kind, offset) => crypto.createHash('sha256').update(`${sourceId}:${kind}:${offset}`).digest('hex').slice(0,32).replace(/^(.{8})(.{4})(.{4})(.{4})(.{12}).*/, '$1-$2-$3-$4-$5');
const logicalSourceId = (value) => {
  if (!value || !/^[A-Za-z0-9._-]{1,128}$/.test(value)) throw new Error('invalid --source-id');
  return value;
};

async function fingerprint(file) {
  const hash=crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

function validCost(row) {
  const numbers=['ts','input','output','cache_read','cache_creation','usd'];
  return Boolean(row && typeof row.agent_id==='string' && row.agent_id && typeof row.session_id==='string' && row.session_id
    && numbers.every((key)=>Number.isFinite(Number(row[key]??0)) && Number(row[key]??0)>=0));
}

async function preflightJsonl(file) {
  let lines=0;
  const input=readline.createInterface({input:fs.createReadStream(file),crlfDelay:Infinity});
  for await (const line of input) {
    if (!line) continue;
    let row; try { row=JSON.parse(line); } catch { throw new Error('invalid JSONL'); }
    if (!validCost(row)) throw new Error('invalid cost row');
    lines++;
  }
  return lines;
}

function preflightSqlite(file) {
  const { DatabaseSync }=require('node:sqlite');
  const db=new DatabaseSync(file,{readOnly:true});
  try {
    for(const row of db.prepare('SELECT value FROM kv').iterate()) JSON.parse(row.value);
    for(const row of db.prepare('SELECT agent_id,text,ts FROM command_history').iterate()) {
      if(typeof row.agent_id!=='string'||!row.agent_id||typeof row.text!=='string'||!Number.isFinite(Number(row.ts))) throw new Error('invalid SQLite row');
    }
  } finally { db.close(); }
}

async function assertSource(client, namespace, sourceId, kind, contentFingerprint) {
  const prior=await client.query('SELECT source_kind,content_fingerprint,completed_at FROM munder_difflin.legacy_imports WHERE namespace=$1 AND source_id=$2',[namespace,sourceId]);
  if (prior.rows[0] && prior.rows[0].source_kind!==kind) throw new Error('source kind changed');
  if (prior.rows[0] && prior.rows[0].content_fingerprint!==contentFingerprint) throw new Error('source content changed');
  return prior.rows[0]?.completed_at ? 'done' : 'pending';
}

async function importJsonl(client, namespace, sourceId, file, contentFingerprint) {
  if (await assertSource(client,namespace,sourceId,'cost_jsonl',contentFingerprint)==='done') return 0;
  const checkpoint=await client.query('SELECT checkpoint FROM munder_difflin.legacy_imports WHERE namespace=$1 AND source_id=$2',[namespace,sourceId]);
  const start=Number(checkpoint.rows[0]?.checkpoint??0);
  const input=readline.createInterface({input:fs.createReadStream(file),crlfDelay:Infinity});
  let lineNo=0, count=0, chunk=[];
  const flush=async(complete=false)=>{
    if (!chunk.length) return;
    const batch=chunk.splice(0,complete?chunk.length:250);
    await client.query('BEGIN');
    try {
      for (const {row,n} of batch) await client.query(
        `INSERT INTO munder_difflin.cost_ledger(namespace,event_id,agent_id,session_id,occurred_at,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,model,usd)
         VALUES($1,$2,$3,$4,to_timestamp($5/1000.0),$6,$7,$8,$9,$10,$11) ON CONFLICT(namespace,event_id) DO NOTHING`,
        [namespace,eventId(sourceId,'cost',n),row.agent_id,row.session_id,Number(row.ts)||0,Number(row.input)||0,Number(row.output)||0,Number(row.cache_read)||0,Number(row.cache_creation)||0,row.model==null?null:String(row.model),Number(row.usd)||0]
      );
      await client.query(`INSERT INTO munder_difflin.legacy_imports(namespace,source_id,source_kind,content_fingerprint,checkpoint,updated_at)
        VALUES($1,$2,'cost_jsonl',$3,$4,now()) ON CONFLICT(namespace,source_id) DO UPDATE SET checkpoint=EXCLUDED.checkpoint,updated_at=now()`,[namespace,sourceId,contentFingerprint,String(batch.at(-1).n+1)]);
      if(complete)await client.query('UPDATE munder_difflin.legacy_imports SET completed_at=now(),updated_at=now() WHERE namespace=$1 AND source_id=$2',[namespace,sourceId]);
      await client.query('COMMIT'); count+=batch.length;
    } catch(error){await client.query('ROLLBACK');throw error;}
  };
  for await (const line of input) {
    if (!line) { lineNo++; continue; }
    if (lineNo>=start) { chunk.push({row:JSON.parse(line),n:lineNo}); if(chunk.length===251) await flush(false); }
    lineNo++;
  }
  if(chunk.length)await flush(true);
  else await client.query(`INSERT INTO munder_difflin.legacy_imports(namespace,source_id,source_kind,content_fingerprint,checkpoint,completed_at)
    VALUES($1,$2,'cost_jsonl',$3,$4,now()) ON CONFLICT(namespace,source_id) DO UPDATE SET completed_at=now(),updated_at=now()`,[namespace,sourceId,contentFingerprint,String(lineNo)]);
  return count;
}

async function importSqlite(client, namespace, sourceId, file, contentFingerprint) {
  if (await assertSource(client,namespace,sourceId,'sqlite',contentFingerprint)==='done') return 0;
  const { DatabaseSync }=require('node:sqlite');
  const db=new DatabaseSync(file,{readOnly:true}); let count=0, completed=false;
  try {
    const historyTotal=Number(db.prepare('SELECT count(*) AS n FROM command_history').get().n);
    for(const spec of [
      {kind:'kv',sql:'SELECT key,value,updated_at FROM kv ORDER BY key',write:async(row)=>{
        let value; try{value=JSON.parse(row.value);}catch{throw new Error('invalid SQLite JSON');}
        await client.query(`INSERT INTO munder_difflin.kv(namespace,key,value,updated_at) VALUES($1,$2,$3::jsonb,to_timestamp($4/1000.0)) ON CONFLICT(namespace,key) DO UPDATE SET value=EXCLUDED.value,updated_at=EXCLUDED.updated_at`,[namespace,row.key,JSON.stringify(value),row.updated_at]);
      }},
      {kind:'history',sql:'SELECT id,agent_id,cwd,text,ts FROM command_history ORDER BY id',write:async(row)=>{
        await client.query(`INSERT INTO munder_difflin.command_history(namespace,event_id,agent_id,cwd,text,occurred_at) VALUES($1,$2,$3,$4,$5,to_timestamp($6/1000.0)) ON CONFLICT(namespace,event_id) DO NOTHING`,[namespace,eventId(sourceId,'history',row.id),row.agent_id,row.cwd,row.text,row.ts]);
      }}
    ]){
      let offset=0;
      for(;;){
        const rows=db.prepare(`${spec.sql} LIMIT ? OFFSET ?`).all(251,offset);
        if(!rows.length)break;
        const batch=rows.slice(0,250), isLast=rows.length<=250;
        await client.query('BEGIN');
        try{
          for(const row of batch){await spec.write(row);count++;}
          offset+=batch.length;
          await client.query(`INSERT INTO munder_difflin.legacy_imports(namespace,source_id,source_kind,content_fingerprint,checkpoint,updated_at)
            VALUES($1,$2,'sqlite',$3,$4,now()) ON CONFLICT(namespace,source_id) DO UPDATE SET checkpoint=EXCLUDED.checkpoint,updated_at=now()`,[namespace,sourceId,contentFingerprint,`${spec.kind}:${offset}`]);
          if(isLast&&(spec.kind==='history'||historyTotal===0)){
            await client.query('UPDATE munder_difflin.legacy_imports SET completed_at=now(),updated_at=now() WHERE namespace=$1 AND source_id=$2',[namespace,sourceId]);
            completed=true;
          }
          await client.query('COMMIT');
        }catch(error){await client.query('ROLLBACK');throw error;}
      }
    }
    if(!completed)await client.query(`INSERT INTO munder_difflin.legacy_imports(namespace,source_id,source_kind,content_fingerprint,completed_at)
      VALUES($1,$2,'sqlite',$3,now()) ON CONFLICT(namespace,source_id) DO UPDATE SET completed_at=now(),updated_at=now()`,[namespace,sourceId,contentFingerprint]);
  }finally{db.close();}
  return count;
}

async function main(){
  const args=process.argv.slice(2), sourceId=logicalSourceId(args[args.indexOf('--source-id')+1]);
  const sqlite=args.includes('--sqlite')?args[args.indexOf('--sqlite')+1]:null;
  const jsonl=args.includes('--cost-jsonl')?args[args.indexOf('--cost-jsonl')+1]:null;
  if((sqlite?1:0)+(jsonl?1:0)!==1||(sqlite&&!fs.existsSync(sqlite))||(jsonl&&!fs.existsSync(jsonl)))throw new Error('invalid source');
  if(jsonl)await preflightJsonl(jsonl); else preflightSqlite(sqlite);
  const file=jsonl||sqlite, digest=await fingerprint(file);
  const {namespace,pool:config}=consumeConfig(); const pool=new Pool(config); const client=await pool.connect();
  try{
    const lock=await client.query('SELECT pg_try_advisory_lock(hashtext($1),hashtext($2)) locked',['munder-difflin',namespace]);
    if(lock.rows[0]?.locked!==true)throw new Error('namespace busy');
    const count=jsonl?await importJsonl(client,namespace,sourceId,jsonl,digest):await importSqlite(client,namespace,sourceId,sqlite,digest);
    process.stdout.write(`Legacy import completed (${count} rows considered).\n`);
  }finally{try{await client.query('SELECT pg_advisory_unlock(hashtext($1),hashtext($2))',['munder-difflin',namespace]);}catch{} client.release(); await pool.end();}
}
if(require.main===module)main().catch(()=>{process.stderr.write('Legacy import failed. Sources were not modified.\n');process.exitCode=1;});
module.exports={logicalSourceId,validCost,preflightJsonl};
