import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { Diagnostics } from '../src/diagnostics.js';

test('диагностика ограничивает журнал и скрывает токены',()=>{
  const d=new Diagnostics(2,Date.now()-2500);
  d.record('error','http','Bearer super-secret','one');d.record('warning','mesh','offline');d.record('error','db','bad');
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'bit-diag-'));fs.writeFileSync(path.join(dir,'x'),'1234');
  const snapshot=d.snapshot({ledger:{events:new Map([['x',1]])},peers:new Map(),dataDir:dir});
  assert.equal(snapshot.errors.length,2);assert.equal(snapshot.events,1);assert.equal(snapshot.storageBytes,4);
  assert.ok(!JSON.stringify(snapshot).includes('super-secret'));assert.ok(snapshot.uptimeSeconds>=2);
});

test('счётчики синхронизации и очистка ошибок работают',()=>{
  const d=new Diagnostics();d.request();d.sync(true);d.sync(false);d.record('warning','sync','timeout');
  assert.deepEqual(d.counters,{requests:1,syncSuccess:1,syncFailure:1});d.clear();assert.equal(d.items.length,0);
});
