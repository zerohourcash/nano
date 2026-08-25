import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import dgram from 'node:dgram';
import { fileURLToPath } from 'node:url';
import { Ledger } from './ledger.js';
import { mac, safeEqual } from './crypto.js';
import QRCode from 'qrcode';

const PORT=Number(process.env.PORT||8787), MESH_PORT=Number(process.env.MESH_PORT||47871);
const STATIC_PEERS=(process.env.PEERS||'').split(',').map(x=>x.trim()).filter(Boolean);
const PROD=process.env.NODE_ENV==='production', API_TOKEN=process.env.API_TOKEN||'', MESH_SECRET=process.env.MESH_SECRET||'';
if(PROD&&(!API_TOKEN||API_TOKEN.length<24||!MESH_SECRET||MESH_SECRET.length<24))throw Error('Production требует API_TOKEN и MESH_SECRET длиной не менее 24 символов');
const DATA=path.resolve(process.env.DATA_DIR||'data'), PUBLIC=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../public');
const trusted=(process.env.TRUSTED_ACTORS||'').split(',').map(x=>x.trim()).filter(Boolean);
const ledger=new Ledger(DATA,process.env.NODE_NAME||`node-${PORT}`,{trustedActors:PROD?trusted:undefined}), peers=new Map();
if(PROD&&!trusted.includes(ledger.identity.actor))throw Error(`Добавьте локальный actor в TRUSTED_ACTORS: ${ledger.identity.actor}`);
const json=(res,status,value)=>{const body=JSON.stringify(value);res.writeHead(status,{'content-type':'application/json; charset=utf-8','access-control-allow-origin':'*'});res.end(body)};
const body=req=>new Promise((resolve,reject)=>{let s='';req.on('data',c=>{s+=c;if(s.length>2e6)reject(Error('too large'))});req.on('end',()=>{try{resolve(s?JSON.parse(s):{})}catch(e){reject(e)}})});
const server=http.createServer(async(req,res)=>{try{
  const u=new URL(req.url,'http://local');
  if(req.method==='OPTIONS'){res.writeHead(204,{'access-control-allow-origin':'*','access-control-allow-methods':'GET,POST,OPTIONS','access-control-allow-headers':'content-type'});return res.end()}
  if(req.method==='GET'&&u.pathname==='/api/qr'){const id=u.searchParams.get('assetId');if(!id||id.length>128)throw Error('Некорректный assetId');const svg=await QRCode.toString(`nano-inventory:asset:${id}`,{type:'svg',errorCorrectionLevel:'H',margin:2});res.writeHead(200,{'content-type':'image/svg+xml','content-disposition':`inline; filename="asset-${id.replace(/[^a-zA-Z0-9_-]/g,'_')}.svg"`});return res.end(svg)}
  const auth=req.headers.authorization?.replace(/^Bearer /,'')||'';
  if(u.pathname.startsWith('/api/')&&API_TOKEN&&!safeEqual(auth,API_TOKEN))return json(res,401,{error:'Требуется токен доступа'});
  if(req.method==='GET'&&u.pathname==='/api/state')return json(res,200,ledger.state());
  if(req.method==='GET'&&u.pathname==='/api/events')return json(res,200,{events:[...ledger.events.values()]});
  if(req.method==='GET'&&u.pathname==='/api/frontiers')return json(res,200,{frontiers:ledger.frontiers()});
  if(req.method==='POST'&&u.pathname==='/api/pull'){const b=await body(req);return json(res,200,{events:ledger.eventsAfter(b.frontiers||{})});}
  if(req.method==='GET'&&u.pathname==='/api/peers')return json(res,200,{peers:[...peers.values()]});
  if(req.method==='POST'&&u.pathname==='/api/events'){const b=await body(req);return json(res,200,ledger.import(b.events||[]));}
  if(req.method==='POST'&&u.pathname==='/api/sync'){const b=await body(req),target=new URL(b.url);if(!['http:','https:'].includes(target.protocol))throw Error('Допустим только HTTP(S) адрес');const r=await fetch(new URL('/api/events',target),{signal:AbortSignal.timeout(5000)});return json(res,200,ledger.import((await r.json()).events||[]));}
  if(req.method==='POST'&&u.pathname==='/api/action'){const b=await body(req);return json(res,201,ledger.append(b.type,b.payload||{}));}
  let f=u.pathname==='/'?'/index.html':u.pathname; f=path.normalize(f).replace(/^(\.\.[/\\])+/, ''); const full=path.join(PUBLIC,f);
  if(!full.startsWith(PUBLIC)||!fs.existsSync(full))return json(res,404,{error:'not found'});
  const ext=path.extname(full),ct={'.html':'text/html','.js':'text/javascript','.css':'text/css','.json':'application/json'}[ext]||'application/octet-stream';res.writeHead(200,{'content-type':ct+'; charset=utf-8'});fs.createReadStream(full).pipe(res);
}catch(e){json(res,400,{error:e.message})}});
server.listen(PORT,'0.0.0.0',()=>console.log(`Nano Inventory: http://localhost:${PORT} (${ledger.identity.actor.slice(0,12)})`));

async function sync(host,port){try{const r=await fetch(`http://${host}:${port}/api/pull`,{method:'POST',headers:{'content-type':'application/json',...(API_TOKEN?{authorization:`Bearer ${API_TOKEN}`}:{})},body:JSON.stringify({frontiers:ledger.frontiers()}),signal:AbortSignal.timeout(2500)});const x=await r.json();ledger.import(x.events||[])}catch{}}
const udp=dgram.createSocket({type:'udp4',reuseAddr:true});
udp.on('message',(buf,r)=>{try{const m=JSON.parse(buf),{tag,...unsigned}=m;if(MESH_SECRET&&!safeEqual(tag,mac(unsigned,MESH_SECRET)))return;if(m.protocol!=='nano-inventory/1'||m.id===ledger.identity.actor)return;const p={id:m.id,name:m.name,host:r.address,port:m.port,lastSeen:new Date().toISOString()};peers.set(m.id,p);sync(p.host,p.port)}catch{}});
udp.bind(MESH_PORT,()=>{try{udp.addMembership('239.42.78.71')}catch(e){console.warn('Multicast disabled:',e.message)}});
setInterval(()=>{const x={protocol:'nano-inventory/1',id:ledger.identity.actor,name:ledger.identity.name,port:PORT},m=Buffer.from(JSON.stringify({...x,...(MESH_SECRET?{tag:mac(x,MESH_SECRET)}:{})}));udp.send(m,MESH_PORT,'239.42.78.71')},3000).unref();
setInterval(()=>STATIC_PEERS.forEach(u=>{try{const x=new URL(u);sync(x.hostname,Number(x.port||80))}catch{}}),5000).unref();
process.on('SIGINT',()=>{udp.close();server.close(()=>process.exit())});
