import fs from 'node:fs';

export class Diagnostics {
  constructor(limit=200, startedAt=Date.now()) { this.limit=limit; this.startedAt=startedAt; this.items=[]; this.counters={requests:0,syncSuccess:0,syncFailure:0}; }
  record(level, source, message, detail='') {
    const clean=v=>String(v??'').replace(/Bearer\s+\S+/gi,'Bearer [REDACTED]').slice(0,1000);
    const item={id:crypto.randomUUID(),at:new Date().toISOString(),level,source:clean(source),message:clean(message),detail:clean(detail)};
    this.items.unshift(item); if(this.items.length>this.limit)this.items.length=this.limit; return item;
  }
  request(){this.counters.requests++}
  sync(ok){this.counters[ok?'syncSuccess':'syncFailure']++}
  snapshot({ledger,peers,dataDir}) {
    let storageBytes=0; try{storageBytes=fsSize(dataDir)}catch{}
    return {status:this.items.some(x=>x.level==='error')?'attention':'healthy',uptimeSeconds:Math.floor((Date.now()-this.startedAt)/1000),storageBytes,events:ledger.events.size,peers:[...peers.values()].filter(p=>Date.now()-Date.parse(p.lastSeen)<15000).length,counters:{...this.counters},errors:this.items};
  }
  clear(){this.items=[]}
}
function fsSize(dir){let n=0;for(const e of fs.readdirSync(dir,{withFileTypes:true})){const p=`${dir}/${e.name}`;n+=e.isDirectory()?fsSize(p):fs.statSync(p).size}return n}
