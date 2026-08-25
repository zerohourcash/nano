import fs from 'node:fs';
import path from 'node:path';
import { newIdentity, signEvent, verifyEvent, verifyWorkerProof, hash } from './crypto.js';

const TYPES=new Set(['PERSON_CREATE','ASSET_CREATE','CHECKOUT','RETURN','MOVE','RESOLVE']);
const text=(v,n)=>typeof v==='string'&&v.trim().length>0&&v.length<=n;
function validatePayload(type,p={}) {
  if(!TYPES.has(type)||!p||typeof p!=='object'||Array.isArray(p))throw Error('Недопустимый тип операции');
  if(type==='PERSON_CREATE'&&(!text(p.personId,128)||!text(p.name,200)||!text(p.publicKey,128)))throw Error('Некорректный сотрудник');
  if(type==='ASSET_CREATE'&&(!text(p.assetId,128)||!text(p.name,300)||String(p.serial||'').length>200||String(p.location||'').length>300))throw Error('Некорректное оборудование');
  if(['CHECKOUT','RETURN','MOVE','RESOLVE'].includes(type)&&!text(p.assetId,128))throw Error('Некорректный идентификатор предмета');
  if(type==='CHECKOUT'&&(!text(p.personId,128)||!p.proof))throw Error('Для выдачи требуется подпись получателя');
  if(['RETURN','MOVE'].includes(type)&&String(p.location||'').length>300)throw Error('Некорректное место');
}

export class Ledger {
  constructor(dir, name = 'node', options = {}) {
    this.dir = dir; fs.mkdirSync(dir, { recursive: true });
    this.lockFile=path.join(dir,'.lock');
    if(fs.existsSync(this.lockFile)){const pid=Number(fs.readFileSync(this.lockFile,'utf8'));try{process.kill(pid,0);throw Error(`Хранилище уже открыто процессом ${pid}`)}catch(e){if(e.code!=='ESRCH')throw e;fs.unlinkSync(this.lockFile)}}
    fs.writeFileSync(this.lockFile,String(process.pid),{flag:'wx',mode:0o600});
    const unlock=()=>{try{if(fs.readFileSync(this.lockFile,'utf8')===String(process.pid))fs.unlinkSync(this.lockFile)}catch{}};process.once('exit',unlock);process.once('SIGTERM',()=>{unlock();process.exit(0)});
    this.identityFile = path.join(dir, 'identity.json');
    this.logFile = path.join(dir, 'events.ndjson');
    this.identity = fs.existsSync(this.identityFile) ? JSON.parse(fs.readFileSync(this.identityFile)) : newIdentity(name);
    if (!fs.existsSync(this.identityFile)) fs.writeFileSync(this.identityFile, JSON.stringify(this.identity, null, 2), { mode: 0o600 });
    this.trustedActors=options.trustedActors?new Set(options.trustedActors):null;
    this.events = new Map();
    if(fs.existsSync(this.logFile)){const rows=fs.readFileSync(this.logFile,'utf8').split('\n').filter(Boolean).map((line,i)=>{try{return JSON.parse(line)}catch{throw Error(`Повреждён JSON журнала, строка ${i+1}`)}}),result=this.import(rows,false);if(result.rejected)throw Error(`Журнал не прошёл полную проверку: отклонено ${result.rejected}`)}
  }

  actorEvents(actor) { return [...this.events.values()].filter(e => e.actor === actor).sort((a,b) => a.seq-b.seq); }
  frontiers() { const out={}; for(const e of this.events.values())if(!out[e.actor]||out[e.actor].seq<e.seq)out[e.actor]={seq:e.seq,id:e.id};return out; }
  eventsAfter(frontiers={}) { return [...this.events.values()].filter(e=>e.seq>(frontiers[e.actor]?.seq||0)).sort((a,b)=>a.actor.localeCompare(b.actor)||a.seq-b.seq); }
  assetHeads(assetId) {
    const es = [...this.events.values()].filter(e => e.payload?.assetId === assetId && e.type !== 'RESOLVE');
    const referenced = new Set(es.map(e => e.assetPrev).filter(Boolean));
    return es.filter(e => !referenced.has(e.id)).map(e => e.id).sort();
  }
  append(type, payload) {
    validatePayload(type,payload);
    if(type==='CHECKOUT')this.verifyCustodyProof(type,payload);
    const own = this.actorEvents(this.identity.actor); const last = own.at(-1);
    const assetId = payload.assetId;
    const heads = assetId ? this.assetHeads(assetId) : [];
    if (assetId && heads.length > 1 && type !== 'RESOLVE') throw new Error('У предмета конфликтующая история; сначала разрешите конфликт');
    const unsigned = { version: 1, actor: this.identity.actor, pub: this.identity.pub, seq: (last?.seq ?? 0)+1,
      prev: last?.id ?? null, assetPrev: type === 'ASSET_CREATE' ? null : (heads[0] ?? null), timestamp: new Date().toISOString(), type, payload };
    const event = signEvent(unsigned, this.identity.privateKey); this.accept(event); return event;
  }
  verifyCustodyProof(type,payload){const q=payload.proof?.request||{},person=this.state().people.find(x=>x.id===payload.personId);if(!person||person.publicKey!==payload.proof?.publicKey)throw Error('Ключ сотрудника не зарегистрирован');if(q.action!==type||q.assetId!==payload.assetId||q.personId!==payload.personId||!text(q.nonce,128)||!text(q.timestamp,64))throw Error('Подпись не соответствует операции');if(this.eventsAfter({}).some(e=>e.payload?.proof?.request?.nonce===q.nonce))throw Error('Повтор транзакции сотрудника');if(!verifyWorkerProof(payload.proof))throw Error('Неверная подпись сотрудника');}
  accept(event, persist=true) {
    if (!verifyEvent(event)) throw new Error('Неверная подпись события');
    validatePayload(event.type,event.payload);
    if(this.trustedActors&&!this.trustedActors.has(event.actor))throw Error('Участник не входит в доверенный контур');
    if(event.type==='CHECKOUT')this.verifyCustodyProof(event.type,event.payload);
    if (this.events.has(event.id)) return false;
    const same = this.actorEvents(event.actor);
    if (event.seq === 1 && event.prev !== null) throw new Error('Неверное начало цепочки участника');
    if (event.seq > 1 && !this.events.has(event.prev)) throw new Error('Не хватает предыдущего события участника');
    if (same.some(e => e.seq === event.seq && e.id !== event.id)) throw new Error('Fork цепочки участника');
    if (event.assetPrev && !this.events.has(event.assetPrev)) throw new Error('Не хватает истории предмета');
    this.events.set(event.id, event);
    if(persist){const fd=fs.openSync(this.logFile,'a',0o600);try{fs.writeSync(fd,JSON.stringify(event)+'\n');fs.fsyncSync(fd)}finally{fs.closeSync(fd)}} return true;
  }
  import(events,persist=true) {
    let pending = events.filter(e => !this.events.has(e.id)), added = 0, progress = true;
    while (pending.length && progress) { progress = false; pending = pending.filter(e => { try { if (this.accept(e,persist)) added++; progress=true; return false; } catch { return true; } }); }
    return { added, rejected: pending.length };
  }
  state() {
    const assets = {}, people = {};
    const ordered = [...this.events.values()].sort((a,b) => a.timestamp.localeCompare(b.timestamp)||a.id.localeCompare(b.id));
    for (const e of ordered) {
      const p=e.payload||{};
      if(e.type==='PERSON_CREATE') people[p.personId]={id:p.personId,name:p.name,publicKey:p.publicKey};
      if(e.type==='ASSET_CREATE') assets[p.assetId]={id:p.assetId,name:p.name,serial:p.serial||'',location:p.location||'',holder:null,status:'available'};
      const a=assets[p.assetId]; if(!a) continue;
      if(e.type==='CHECKOUT'){a.holder=p.personId;a.status='checked_out';}
      if(e.type==='RETURN'){a.holder=null;a.status='available';if(p.location)a.location=p.location;}
      if(e.type==='MOVE') a.location=p.location;
    }
    const conflicts=[];
    for(const a of Object.values(assets)){const heads=this.assetHeads(a.id);if(heads.length>1){a.status='conflict';conflicts.push({assetId:a.id,heads});}}
    return { node:{id:this.identity.actor,name:this.identity.name}, assets:Object.values(assets),people:Object.values(people),conflicts,eventCount:this.events.size };
  }
}
