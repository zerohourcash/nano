import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { generateKeyPairSync, sign } from 'node:crypto';
import { Ledger } from '../src/ledger.js';
import { stable } from '../src/crypto.js';

const tmp=()=>fs.mkdtempSync(path.join(os.tmpdir(),'nano-inv-'));
function worker(personId='p1'){
  const {publicKey,privateKey}=generateKeyPairSync('ed25519');
  return {personId,publicKey:publicKey.export({format:'jwk'}).x,proof(action,assetId){const request={action,assetId,personId,timestamp:new Date().toISOString(),nonce:crypto.randomUUID()};return{request,publicKey:this.publicKey,signature:sign(null,Buffer.from(stable(request)),privateKey).toString('base64url')}}};
}

test('подпись сотрудника, выдача и синхронизация',()=>{
  const a=new Ledger(tmp(),'A'),b=new Ledger(tmp(),'B'),w=worker();
  const person=a.append('PERSON_CREATE',{personId:w.personId,name:'Анна',publicKey:w.publicKey});
  a.append('ASSET_CREATE',{assetId:'x1',name:'Ноутбук'});
  a.append('CHECKOUT',{assetId:'x1',personId:w.personId,proof:w.proof('CHECKOUT','x1')});
  assert.equal(b.import([...a.events.values()]).added,3);
  assert.equal(b.state().assets[0].holder,'p1');
  assert.throws(()=>b.accept({...person,payload:{...person.payload,name:'Зло'}}),/подпись/i);
});

test('поддельная и повторная транзакция сотрудника отклоняется',()=>{
  const a=new Ledger(tmp(),'A'),w=worker();
  a.append('PERSON_CREATE',{personId:w.personId,name:'Анна',publicKey:w.publicKey});
  a.append('ASSET_CREATE',{assetId:'x1',name:'Ноутбук'});
  const proof=w.proof('CHECKOUT','x1');
  assert.throws(()=>a.append('CHECKOUT',{assetId:'other',personId:w.personId,proof}),/не соответствует/);
  a.append('CHECKOUT',{assetId:'x1',personId:w.personId,proof});
  assert.throws(()=>a.append('CHECKOUT',{assetId:'x1',personId:w.personId,proof}),/Повтор/);
});

test('параллельная офлайн-выдача обнаруживается как конфликт',()=>{
  const seed=new Ledger(tmp(),'seed'),w=worker();
  seed.append('ASSET_CREATE',{assetId:'x',name:'Камера'});
  seed.append('PERSON_CREATE',{personId:w.personId,name:'Иван',publicKey:w.publicKey});
  const a=new Ledger(tmp(),'A'),b=new Ledger(tmp(),'B');a.import([...seed.events.values()]);b.import([...seed.events.values()]);
  a.append('CHECKOUT',{assetId:'x',personId:w.personId,proof:w.proof('CHECKOUT','x')});
  b.append('CHECKOUT',{assetId:'x',personId:w.personId,proof:w.proof('CHECKOUT','x')});
  a.import([...b.events.values()]);assert.equal(a.state().assets[0].status,'conflict');
});

test('полная проверка журнала при перезапуске',()=>{
  const dir=tmp(),a=new Ledger(dir,'A');
  a.append('ASSET_CREATE',{assetId:'x',name:'Дрель'});
  fs.unlinkSync(path.join(dir,'.lock'));
  const restored=new Ledger(dir,'A');assert.equal(restored.state().eventCount,1);
  fs.unlinkSync(path.join(dir,'.lock'));
  fs.appendFileSync(path.join(dir,'events.ndjson'),'{broken}\n');
  assert.throws(()=>new Ledger(dir,'A'),/Повреждён JSON/);
});
