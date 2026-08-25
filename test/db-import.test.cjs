'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {logicalSourceId,validCost,preflightJsonl}=require('../tools/db-import.cjs');

test('legacy import requires an immutable operator source id',()=>{
  assert.equal(logicalSourceId('backup-2026-08'),'backup-2026-08');
  assert.throws(()=>logicalSourceId('../mutable'));
  assert.throws(()=>logicalSourceId(''));
});

test('JSONL preflight stops on invalid input without modifying its source',async()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'md-import-'));
  const file=path.join(dir,'ledger.jsonl');
  const original='{"agent_id":"a","session_id":"s","usd":0}\nnot-json\n';
  fs.writeFileSync(file,original);
  try{await assert.rejects(preflightJsonl(file));assert.equal(fs.readFileSync(file,'utf8'),original);}
  finally{fs.rmSync(dir,{recursive:true,force:true});}
});

test('cost preflight rejects invalid, negative, and non-finite rows',()=>{
  const good={agent_id:'a',session_id:'s',ts:1,input:0,output:0,cache_read:0,cache_creation:0,usd:0};
  assert.equal(validCost(good),true);
  assert.equal(validCost({...good,usd:-1}),false);
  assert.equal(validCost({...good,input:Infinity}),false);
  assert.equal(validCost({...good,session_id:''}),false);
});
