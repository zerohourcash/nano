use bit_core::{AccountId, CommunityId, Hash};
use blake3::Hasher;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

const SIGN_DOMAIN: &[u8] = b"bit-community/knowledge/v1/sign";
const HASH_DOMAIN: &str = "bit-community/knowledge/v1/hash";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub blob: Hash,
    pub media_type: String,
    pub caption: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnsignedRevision {
    pub version: u16,
    pub community: CommunityId,
    pub page_id: Uuid,
    pub author: AccountId,
    pub parents: Vec<Hash>,
    pub title: String,
    pub body_markdown: String,
    pub tags: Vec<String>,
    pub attachments: Vec<AttachmentRef>,
    pub created_at_ms: i64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub unsigned: UnsignedRevision,
    pub signature: Vec<u8>,
}
#[derive(Debug, Error, Eq, PartialEq)]
pub enum KnowledgeError {
    #[error("invalid signature")]
    Signature,
    #[error("wrong community")]
    Community,
    #[error("invalid page revision")]
    Invalid,
    #[error("missing or cross-page parent")]
    Parent,
    #[error("duplicate revision")]
    Duplicate,
    #[error("author is not an authorized editor")]
    Unauthorized,
}

impl Revision {
    pub fn sign(unsigned: UnsignedRevision, key: &SigningKey) -> Self {
        Self {
            signature: key.sign(&bytes(&unsigned)).to_bytes().to_vec(),
            unsigned,
        }
    }
    pub fn verify(&self) -> Result<(), KnowledgeError> {
        let key = VerifyingKey::from_bytes(&self.unsigned.author.0)
            .map_err(|_| KnowledgeError::Signature)?;
        let signature = Signature::try_from(self.signature.as_slice())
            .map_err(|_| KnowledgeError::Signature)?;
        key.verify(&bytes(&self.unsigned), &signature)
            .map_err(|_| KnowledgeError::Signature)
    }
    pub fn hash(&self) -> Hash {
        let mut h = Hasher::new_derive_key(HASH_DOMAIN);
        h.update(&postcard::to_allocvec(self).expect("revision serializes"));
        Hash(*h.finalize().as_bytes())
    }
}

#[derive(Clone, Default)]
pub struct KnowledgeBase {
    community: Option<CommunityId>,
    revisions: BTreeMap<Hash, Revision>,
    heads: BTreeMap<Uuid, BTreeSet<Hash>>,
}
impl KnowledgeBase {
    pub fn new(community: CommunityId) -> Self {
        Self {
            community: Some(community),
            ..Default::default()
        }
    }
    pub fn heads(&self, page: &Uuid) -> BTreeSet<Hash> {
        self.heads.get(page).cloned().unwrap_or_default()
    }
    pub fn revision(&self, hash: &Hash) -> Option<&Revision> {
        self.revisions.get(hash)
    }
    pub fn insert(&mut self, revision: Revision, authorized: bool) -> Result<Hash, KnowledgeError> {
        revision.verify()?;
        if self.community != Some(revision.unsigned.community) {
            return Err(KnowledgeError::Community);
        }
        if !authorized {
            return Err(KnowledgeError::Unauthorized);
        }
        if revision.unsigned.version != 1
            || revision.unsigned.title.trim().is_empty()
            || revision.unsigned.title.len() > 300
            || revision.unsigned.body_markdown.len() > 2 * 1024 * 1024
            || revision.unsigned.tags.len() > 64
            || revision.unsigned.attachments.len() > 128
        {
            return Err(KnowledgeError::Invalid);
        }
        let hash = revision.hash();
        if self.revisions.contains_key(&hash) {
            return Err(KnowledgeError::Duplicate);
        }
        let mut unique = BTreeSet::new();
        for parent_hash in &revision.unsigned.parents {
            if !unique.insert(*parent_hash) {
                return Err(KnowledgeError::Invalid);
            }
            let parent = self
                .revisions
                .get(parent_hash)
                .ok_or(KnowledgeError::Parent)?;
            if parent.unsigned.page_id != revision.unsigned.page_id {
                return Err(KnowledgeError::Parent);
            }
        }
        if revision.unsigned.parents.is_empty()
            && self.heads.contains_key(&revision.unsigned.page_id)
        {
            return Err(KnowledgeError::Parent);
        }
        let heads = self.heads.entry(revision.unsigned.page_id).or_default();
        for parent in &revision.unsigned.parents {
            heads.remove(parent);
        }
        heads.insert(hash);
        self.revisions.insert(hash, revision);
        Ok(hash)
    }
    pub fn len(&self) -> usize {
        self.revisions.len()
    }
    pub fn is_empty(&self) -> bool {
        self.revisions.is_empty()
    }
}
fn bytes(revision: &UnsignedRevision) -> Vec<u8> {
    let mut value = SIGN_DOMAIN.to_vec();
    value.extend(postcard::to_allocvec(revision).expect("revision serializes"));
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;
    fn signed(
        key: &SigningKey,
        community: CommunityId,
        page_id: Uuid,
        parents: Vec<Hash>,
        body: &str,
        time: i64,
    ) -> Revision {
        Revision::sign(
            UnsignedRevision {
                version: 1,
                community,
                page_id,
                author: AccountId(key.verifying_key().to_bytes()),
                parents,
                title: "Safety".into(),
                body_markdown: body.into(),
                tags: vec!["work".into()],
                attachments: vec![],
                created_at_ms: time,
            },
            key,
        )
    }
    #[test]
    fn offline_edits_converge_and_merge_without_data_loss() {
        let community = CommunityId(*Uuid::now_v7().as_bytes());
        let page = Uuid::now_v7();
        let alice = SigningKey::generate(&mut OsRng);
        let bob = SigningKey::generate(&mut OsRng);
        let mut base = KnowledgeBase::new(community);
        let root_hash = base
            .insert(signed(&alice, community, page, vec![], "v1", 1), true)
            .unwrap();
        let mut a = base.clone();
        let mut b = base.clone();
        let alice_revision = signed(
            &alice,
            community,
            page,
            vec![root_hash],
            "Alice offline edit",
            2,
        );
        let alice_hash = a.insert(alice_revision.clone(), true).unwrap();
        let bob_revision = signed(
            &bob,
            community,
            page,
            vec![root_hash],
            "Bob offline edit",
            2,
        );
        let bob_hash = b.insert(bob_revision.clone(), true).unwrap();
        a.insert(bob_revision, true).unwrap();
        b.insert(alice_revision, true).unwrap();
        assert_eq!(a.heads(&page), b.heads(&page));
        assert_eq!(a.heads(&page).len(), 2);
        let merged = signed(
            &alice,
            community,
            page,
            vec![alice_hash, bob_hash],
            "Merged verified text",
            3,
        );
        let merged_hash = a.insert(merged.clone(), true).unwrap();
        b.insert(merged, true).unwrap();
        assert_eq!(a.heads(&page), BTreeSet::from([merged_hash]));
        assert_eq!(a.heads(&page), b.heads(&page));
    }
    #[test]
    fn unauthorized_or_cross_organization_revision_is_rejected() {
        let community = CommunityId(*Uuid::now_v7().as_bytes());
        let other = CommunityId(*Uuid::now_v7().as_bytes());
        let page = Uuid::now_v7();
        let key = SigningKey::generate(&mut OsRng);
        let mut kb = KnowledgeBase::new(community);
        assert_eq!(
            kb.insert(signed(&key, community, page, vec![], "x", 1), false),
            Err(KnowledgeError::Unauthorized)
        );
        assert_eq!(
            kb.insert(signed(&key, other, page, vec![], "x", 1), true),
            Err(KnowledgeError::Community)
        );
    }
}
