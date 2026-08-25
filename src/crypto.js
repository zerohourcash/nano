import { createHash, generateKeyPairSync, sign, verify, createPrivateKey, createPublicKey, timingSafeEqual, createHmac } from 'node:crypto';

export function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`;
  if (value && typeof value === 'object') return `{${Object.keys(value).sort().map(k => `${JSON.stringify(k)}:${stable(value[k])}`).join(',')}}`;
  return JSON.stringify(value);
}

export function hash(value) {
  return createHash('blake2b512').update(typeof value === 'string' ? value : stable(value)).digest('hex').slice(0, 64);
}

export function newIdentity(name = 'node') {
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const pub = publicKey.export({ type: 'spki', format: 'der' }).toString('base64url');
  return { name, pub, actor: hash(pub), privateKey: privateKey.export({ type: 'pkcs8', format: 'pem' }) };
}

export function signEvent(unsigned, privatePem) {
  const bytes = Buffer.from(stable(unsigned));
  const signature = sign(null, bytes, createPrivateKey(privatePem)).toString('base64url');
  return { ...unsigned, id: hash(bytes), signature };
}

export function verifyEvent(event) {
  const { id, signature, ...unsigned } = event;
  if (!id || !signature || id !== hash(Buffer.from(stable(unsigned)))) return false;
  try {
    const key = createPublicKey({ key: Buffer.from(event.pub, 'base64url'), type: 'spki', format: 'der' });
    return event.actor === hash(event.pub) && verify(null, Buffer.from(stable(unsigned)), key, Buffer.from(signature, 'base64url'));
  } catch { return false; }
}

export function mac(value, secret) { return createHmac('sha256', secret).update(stable(value)).digest('base64url'); }
export function safeEqual(a='', b='') { const x=Buffer.from(a),y=Buffer.from(b);return x.length===y.length&&timingSafeEqual(x,y); }
export function verifyWorkerProof(proof) {
  try { const { signature, publicKey, request }=proof||{};const key=createPublicKey({key:{kty:'OKP',crv:'Ed25519',x:publicKey},format:'jwk'});return verify(null,Buffer.from(stable(request)),key,Buffer.from(signature,'base64url')); } catch{return false}
}
