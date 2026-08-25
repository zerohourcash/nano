use bit_core::{AccountId, CommunityId, Hash};
use blake3::Hasher;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use thiserror::Error;
use uuid::Uuid;

const SIGN_DOMAIN: &[u8] = b"bit-community/chat/v1/sign";
const HASH_DOMAIN: &str = "bit-community/chat/v1/hash";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ChannelId(pub [u8; 16]);
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ChatKind {
    ChannelCreate {
        name: String,
        private: bool,
    },
    MemberSet {
        member: AccountId,
        enabled: bool,
        key_epoch: u32,
        wrapped_key: Vec<u8>,
    },
    Message {
        key_epoch: u32,
        nonce: [u8; 24],
        ciphertext: Vec<u8>,
        content_type: String,
        reply_to: Option<Hash>,
        attachments: Vec<Hash>,
    },
    Receipt {
        message: Hash,
    },
    Tombstone {
        message: Hash,
    },
    CallSignal {
        call_id: [u8; 16],
        signal_type: CallSignalType,
        nonce: [u8; 24],
        ciphertext: Vec<u8>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallSignalType {
    Offer,
    Answer,
    IceCandidate,
    Ringing,
    Hangup,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentChunk {
    pub hash: Hash,
    pub nonce: [u8; 24],
    pub plaintext_len: u32,
    pub ciphertext_len: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentManifest {
    pub version: u16,
    pub attachment_id: [u8; 16],
    pub filename: String,
    pub media_type: String,
    pub total_plaintext_len: u64,
    pub chunks: Vec<AttachmentChunk>,
}
pub type EncryptedAttachment = ([u8; 32], AttachmentManifest, Vec<Vec<u8>>);
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnsignedChatEvent {
    pub version: u16,
    pub community: CommunityId,
    pub channel: ChannelId,
    pub author: AccountId,
    pub sequence: u64,
    pub previous: Option<Hash>,
    pub nonce: [u8; 16],
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub hop_limit: u8,
    pub work_nonce: u64,
    pub kind: ChatKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatEvent {
    pub unsigned: UnsignedChatEvent,
    pub signature: Vec<u8>,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ChatError {
    #[error("invalid signature")]
    Signature,
    #[error("invalid key or ciphertext")]
    Crypto,
    #[error("message exceeds 64 KiB mesh limit")]
    TooLarge,
    #[error("duplicate message")]
    Duplicate,
    #[error("message rate exceeded")]
    RateLimited,
    #[error("invalid ttl or hop limit")]
    Lifetime,
    #[error("insufficient anti-spam work")]
    Work,
}
impl ChatEvent {
    pub fn sign(unsigned: UnsignedChatEvent, key: &SigningKey) -> Self {
        let signature = key.sign(&signing_bytes(&unsigned)).to_bytes().to_vec();
        Self {
            unsigned,
            signature,
        }
    }
    pub fn verify(&self) -> Result<(), ChatError> {
        let key =
            VerifyingKey::from_bytes(&self.unsigned.author.0).map_err(|_| ChatError::Signature)?;
        let sig =
            Signature::try_from(self.signature.as_slice()).map_err(|_| ChatError::Signature)?;
        key.verify(&signing_bytes(&self.unsigned), &sig)
            .map_err(|_| ChatError::Signature)
    }
    pub fn hash(&self) -> Hash {
        let mut h = Hasher::new_derive_key(HASH_DOMAIN);
        h.update(&postcard::to_allocvec(self).expect("chat serializes"));
        Hash(*h.finalize().as_bytes())
    }
}
pub fn encrypt(
    key: &[u8; 32],
    plaintext: &[u8],
    associated_data: &[u8],
) -> Result<([u8; 24], Vec<u8>), ChatError> {
    if plaintext.len() > 65536 {
        return Err(ChatError::TooLarge);
    }
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: associated_data,
            },
        )
        .map_err(|_| ChatError::Crypto)?;
    Ok((nonce, cipher))
}
pub fn decrypt(
    key: &[u8; 32],
    nonce: &[u8; 24],
    ciphertext: &[u8],
    associated_data: &[u8],
) -> Result<Vec<u8>, ChatError> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: associated_data,
            },
        )
        .map_err(|_| ChatError::Crypto)
}
pub fn new_channel_id() -> ChannelId {
    ChannelId(*Uuid::now_v7().as_bytes())
}
pub fn encrypt_attachment(
    data: &[u8],
    filename: &str,
    media_type: &str,
    max_bytes: usize,
) -> Result<EncryptedAttachment, ChatError> {
    if data.len() > max_bytes || filename.len() > 255 || media_type.len() > 127 {
        return Err(ChatError::TooLarge);
    }
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let attachment_id = *Uuid::now_v7().as_bytes();
    let mut refs = Vec::new();
    let mut encrypted = Vec::new();
    for (index, part) in data.chunks(64 * 1024).enumerate() {
        let mut aad = attachment_id.to_vec();
        aad.extend_from_slice(&(index as u32).to_be_bytes());
        let (nonce, ciphertext) = encrypt(&key, part, &aad)?;
        let mut h = Hasher::new_derive_key("bit-community/attachment-chunk/v1");
        h.update(&ciphertext);
        refs.push(AttachmentChunk {
            hash: Hash(*h.finalize().as_bytes()),
            nonce,
            plaintext_len: part.len() as u32,
            ciphertext_len: ciphertext.len() as u32,
        });
        encrypted.push(ciphertext)
    }
    Ok((
        key,
        AttachmentManifest {
            version: 1,
            attachment_id,
            filename: filename.into(),
            media_type: media_type.into(),
            total_plaintext_len: data.len() as u64,
            chunks: refs,
        },
        encrypted,
    ))
}
pub fn decrypt_attachment(
    key: &[u8; 32],
    manifest: &AttachmentManifest,
    chunks: &[Vec<u8>],
) -> Result<Vec<u8>, ChatError> {
    if manifest.version != 1 || chunks.len() != manifest.chunks.len() {
        return Err(ChatError::Crypto);
    }
    let mut out = Vec::with_capacity(manifest.total_plaintext_len as usize);
    for (index, (reference, ciphertext)) in manifest.chunks.iter().zip(chunks).enumerate() {
        let mut h = Hasher::new_derive_key("bit-community/attachment-chunk/v1");
        h.update(ciphertext);
        if Hash(*h.finalize().as_bytes()) != reference.hash
            || ciphertext.len() != reference.ciphertext_len as usize
        {
            return Err(ChatError::Crypto);
        }
        let mut aad = manifest.attachment_id.to_vec();
        aad.extend_from_slice(&(index as u32).to_be_bytes());
        let part = decrypt(key, &reference.nonce, ciphertext, &aad)?;
        if part.len() != reference.plaintext_len as usize {
            return Err(ChatError::Crypto);
        }
        out.extend(part)
    }
    if out.len() != manifest.total_plaintext_len as usize {
        return Err(ChatError::Crypto);
    }
    Ok(out)
}
fn signing_bytes(x: &UnsignedChatEvent) -> Vec<u8> {
    let mut b = SIGN_DOMAIN.to_vec();
    b.extend(postcard::to_allocvec(x).expect("chat serializes"));
    b
}

#[derive(Clone, Debug)]
pub struct AntiSpamPolicy {
    pub window_ms: i64,
    pub authorized_messages_per_window: usize,
    pub guest_messages_per_window: usize,
    pub max_hops: u8,
    pub max_lifetime_ms: i64,
    pub guest_work_bits: u32,
    pub seen_capacity: usize,
}
impl Default for AntiSpamPolicy {
    fn default() -> Self {
        Self {
            window_ms: 60_000,
            authorized_messages_per_window: 30,
            guest_messages_per_window: 2,
            max_hops: 7,
            max_lifetime_ms: 7 * 24 * 60 * 60 * 1000,
            guest_work_bits: 18,
            seen_capacity: 50_000,
        }
    }
}
pub struct AntiSpamGuard {
    policy: AntiSpamPolicy,
    seen: BTreeSet<Hash>,
    seen_order: VecDeque<Hash>,
    rates: BTreeMap<(CommunityId, AccountId), VecDeque<i64>>,
}
impl AntiSpamGuard {
    pub fn new(policy: AntiSpamPolicy) -> Self {
        Self {
            policy,
            seen: BTreeSet::new(),
            seen_order: VecDeque::new(),
            rates: BTreeMap::new(),
        }
    }
    pub fn accept(
        &mut self,
        event: &ChatEvent,
        now_ms: i64,
        authorized: bool,
    ) -> Result<Hash, ChatError> {
        event.verify()?;
        if postcard::to_allocvec(event)
            .map_err(|_| ChatError::TooLarge)?
            .len()
            > 70 * 1024
        {
            return Err(ChatError::TooLarge);
        }
        match &event.unsigned.kind {
            ChatKind::Message {
                attachments,
                ciphertext,
                ..
            } if attachments.len() > 16 || ciphertext.len() > 65_552 => {
                return Err(ChatError::TooLarge);
            }
            ChatKind::CallSignal { ciphertext, .. } if ciphertext.len() > 16 * 1024 => {
                return Err(ChatError::TooLarge);
            }
            _ => {}
        }
        let h = event.hash();
        if self.seen.contains(&h) {
            return Err(ChatError::Duplicate);
        }
        let u = &event.unsigned;
        if u.hop_limit == 0
            || u.hop_limit > self.policy.max_hops
            || u.expires_at_ms <= u.created_at_ms
            || u.expires_at_ms - u.created_at_ms > self.policy.max_lifetime_ms
            || now_ms > u.expires_at_ms
            || u.created_at_ms > now_ms + 300_000
        {
            return Err(ChatError::Lifetime);
        }
        if !authorized && leading_zero_bits(&h.0) < self.policy.guest_work_bits {
            return Err(ChatError::Work);
        }
        let q = self.rates.entry((u.community, u.author)).or_default();
        while q
            .front()
            .is_some_and(|t| *t <= now_ms - self.policy.window_ms)
        {
            q.pop_front();
        }
        let limit = if authorized {
            self.policy.authorized_messages_per_window
        } else {
            self.policy.guest_messages_per_window
        };
        if q.len() >= limit {
            return Err(ChatError::RateLimited);
        }
        q.push_back(now_ms);
        self.seen.insert(h);
        self.seen_order.push_back(h);
        while self.seen_order.len() > self.policy.seen_capacity {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen.remove(&old);
            }
        }
        Ok(h)
    }
}
fn leading_zero_bits(bytes: &[u8; 32]) -> u32 {
    let mut n = 0;
    for b in bytes {
        if *b == 0 {
            n += 8
        } else {
            n += b.leading_zeros();
            break;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_message_is_signed_and_tamper_evident() {
        let signing = SigningKey::generate(&mut OsRng);
        let author = AccountId(signing.verifying_key().to_bytes());
        let community = CommunityId(*Uuid::now_v7().as_bytes());
        let channel = new_channel_id();
        let key = [7u8; 32];
        let aad = b"org/channel/epoch/1";
        let (nonce, ciphertext) = encrypt(&key, "смена началась".as_bytes(), aad).unwrap();
        let unsigned = UnsignedChatEvent {
            version: 1,
            community,
            channel,
            author,
            sequence: 1,
            previous: None,
            nonce: [1; 16],
            created_at_ms: 1,
            expires_at_ms: 60_001,
            hop_limit: 3,
            work_nonce: 0,
            kind: ChatKind::Message {
                key_epoch: 1,
                nonce,
                ciphertext,
                content_type: "text/plain".into(),
                reply_to: None,
                attachments: vec![],
            },
        };
        let event = ChatEvent::sign(unsigned, &signing);
        event.verify().unwrap();
        let ChatKind::Message {
            nonce, ciphertext, ..
        } = &event.unsigned.kind
        else {
            panic!()
        };
        assert_eq!(
            decrypt(&key, nonce, ciphertext, aad).unwrap(),
            "смена началась".as_bytes()
        );
        assert_eq!(
            decrypt(&key, nonce, ciphertext, b"wrong"),
            Err(ChatError::Crypto)
        );
    }

    #[test]
    fn spam_guard_deduplicates_and_rate_limits() {
        let signing = SigningKey::generate(&mut OsRng);
        let author = AccountId(signing.verifying_key().to_bytes());
        let community = CommunityId(*Uuid::now_v7().as_bytes());
        let mut guard = AntiSpamGuard::new(AntiSpamPolicy {
            authorized_messages_per_window: 1,
            ..Default::default()
        });
        let make = |nonce| {
            ChatEvent::sign(
                UnsignedChatEvent {
                    version: 1,
                    community,
                    channel: new_channel_id(),
                    author,
                    sequence: nonce,
                    previous: None,
                    nonce: [nonce as u8; 16],
                    created_at_ms: 1,
                    expires_at_ms: 60_001,
                    hop_limit: 3,
                    work_nonce: 0,
                    kind: ChatKind::Receipt {
                        message: Hash([nonce as u8; 32]),
                    },
                },
                &signing,
            )
        };
        let first = make(1);
        guard.accept(&first, 2, true).unwrap();
        assert_eq!(guard.accept(&first, 2, true), Err(ChatError::Duplicate));
        assert_eq!(guard.accept(&make(2), 2, true), Err(ChatError::RateLimited));
    }
    #[test]
    fn attachment_is_chunked_resumable_and_verified() {
        let data = vec![42u8; 150_000];
        let (key, manifest, chunks) =
            encrypt_attachment(&data, "plan.pdf", "application/pdf", 1_000_000).unwrap();
        assert_eq!(chunks.len(), 3);
        assert_eq!(decrypt_attachment(&key, &manifest, &chunks).unwrap(), data);
        let mut bad = chunks.clone();
        bad[1][0] ^= 1;
        assert_eq!(
            decrypt_attachment(&key, &manifest, &bad),
            Err(ChatError::Crypto)
        );
    }
}
