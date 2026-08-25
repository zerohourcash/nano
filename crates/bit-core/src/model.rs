use blake3::Hasher;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: u16 = 1;
const SIGN_DOMAIN: &[u8] = b"bit-community/block/v1/sign";
const HASH_DOMAIN: &str = "bit-community/block/v1/hash";

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Hash(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct AccountId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct CommunityId(pub [u8; 16]);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Role {
    Admin,
    Member,
    Inventory,
    Issuer,
    Accountant,
    Merchant,
    Auditor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Posting {
    pub account: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    Genesis {
        community: CommunityId,
        name: String,
        bit_decimals: u8,
    },
    MemberAuthorize {
        account: AccountId,
        name: String,
        roles: Vec<Role>,
    },
    MembershipProposal {
        account: AccountId,
        name: String,
        roles: Vec<Role>,
    },
    GovernanceApproval {
        proposal: Hash,
    },
    MembershipActivate {
        proposal: Hash,
        approvals: Vec<Hash>,
    },
    ChatKeyPublish {
        encryption_key: [u8; 32],
    },
    AssetCreate {
        asset_id: Uuid,
        name: String,
        serial: String,
        location: String,
        value_minor: i64,
    },
    CustodyOffer {
        asset_id: Uuid,
        recipient: AccountId,
        note: String,
    },
    CustodyAccept {
        offer: Hash,
    },
    BitIssue {
        recipient: AccountId,
        amount_minor: i64,
        memo: String,
    },
    BitSend {
        recipient: AccountId,
        amount_minor: i64,
        memo: String,
        sale_id: Option<Uuid>,
    },
    BitReceive {
        send: Hash,
    },
    SaleRecord {
        sale_id: Uuid,
        seller: AccountId,
        buyer: AccountId,
        amount_minor: i64,
        description: String,
    },
    JournalEntry {
        entry_id: Uuid,
        description: String,
        postings: Vec<Posting>,
    },
    ChatMessage {
        channel: String,
        body: String,
        reply_to: Option<Hash>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UnsignedBlock {
    pub version: u16,
    pub community: CommunityId,
    pub author: AccountId,
    pub sequence: u64,
    pub previous: Option<Hash>,
    pub wall_time_ms: i64,
    pub nonce: [u8; 16],
    pub operation: Operation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub unsigned: UnsignedBlock,
    pub signature: Vec<u8>,
}

impl Block {
    pub fn signing_bytes(unsigned: &UnsignedBlock) -> Vec<u8> {
        let mut out = SIGN_DOMAIN.to_vec();
        out.extend(postcard::to_allocvec(unsigned).expect("serializable block"));
        out
    }
    pub fn hash(&self) -> Hash {
        let mut h = Hasher::new_derive_key(HASH_DOMAIN);
        h.update(&postcard::to_allocvec(self).expect("serializable block"));
        Hash(*h.finalize().as_bytes())
    }
    pub fn verify_signature(&self) -> bool {
        let Ok(key) = VerifyingKey::from_bytes(&self.unsigned.author.0) else {
            return false;
        };
        let Ok(sig) = Signature::try_from(self.signature.as_slice()) else {
            return false;
        };
        key.verify_strict(&Self::signing_bytes(&self.unsigned), &sig)
            .is_ok()
    }
}

pub struct Identity(SigningKey);
impl Identity {
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut OsRng))
    }
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(bytes))
    }
    pub fn secret_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }
    pub fn account(&self) -> AccountId {
        AccountId(self.0.verifying_key().to_bytes())
    }
    pub fn sign(&self, unsigned: UnsignedBlock) -> Block {
        Block {
            signature: self
                .0
                .sign(&Block::signing_bytes(&unsigned))
                .to_bytes()
                .to_vec(),
            unsigned,
        }
    }
}
