use crate::*;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum LedgerError {
    #[error("unsupported protocol version")]
    Version,
    #[error("wrong community")]
    Community,
    #[error("invalid signature")]
    Signature,
    #[error("duplicate block")]
    Duplicate,
    #[error("account frontier mismatch or fork")]
    Frontier,
    #[error("operation is not authorized")]
    Unauthorized,
    #[error("invalid operation: {0}")]
    Invalid(&'static str),
    #[error("referenced block is missing or incompatible")]
    Link,
    #[error("insufficient Bit balance")]
    InsufficientFunds,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct AssetState {
    pub name: String,
    pub serial: String,
    pub location: String,
    pub holder: Option<AccountId>,
    pub pending_offer: Option<Hash>,
}
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct MembershipProposalState {
    pub account: AccountId,
    pub name: String,
    pub roles: Vec<Role>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize)]
pub struct Projection {
    pub members: BTreeMap<AccountId, (String, BTreeSet<String>)>,
    pub assets: BTreeMap<Uuid, AssetState>,
    pub bit_balances: BTreeMap<AccountId, i64>,
    pub pending_bit: BTreeMap<Hash, (AccountId, AccountId, i64)>,
    pub journal: BTreeMap<String, (i64, i64)>,
    pub chats: Vec<(AccountId, String, String)>,
    pub chat_keys: BTreeMap<AccountId, [u8; 32]>,
    pub membership_proposals: BTreeMap<Hash, MembershipProposalState>,
    pub governance_approvals: BTreeMap<Hash, BTreeSet<AccountId>>,
}

#[derive(Clone)]
pub struct Ledger {
    community: Option<CommunityId>,
    blocks: BTreeMap<Hash, Block>,
    frontiers: BTreeMap<AccountId, (u64, Hash)>,
    state: Projection,
}
impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}
impl Ledger {
    pub fn new() -> Self {
        Self {
            community: None,
            blocks: BTreeMap::new(),
            frontiers: BTreeMap::new(),
            state: Projection::default(),
        }
    }
    pub fn community(&self) -> Option<CommunityId> {
        self.community
    }
    pub fn state(&self) -> &Projection {
        &self.state
    }
    pub fn blocks(&self) -> impl Iterator<Item = (&Hash, &Block)> {
        self.blocks.iter()
    }
    pub fn frontier(&self, a: &AccountId) -> Option<(u64, Hash)> {
        self.frontiers.get(a).copied()
    }
    pub fn next_unsigned(
        &self,
        id: &Identity,
        operation: Operation,
        wall_time_ms: i64,
    ) -> Result<UnsignedBlock, LedgerError> {
        let community = match &operation {
            Operation::Genesis { community, .. } if self.community.is_none() => *community,
            _ => self.community.ok_or(LedgerError::Community)?,
        };
        let (sequence, previous) = self
            .frontier(&id.account())
            .map_or((1, None), |(s, h)| (s + 1, Some(h)));
        Ok(UnsignedBlock {
            version: PROTOCOL_VERSION,
            community,
            author: id.account(),
            sequence,
            previous,
            wall_time_ms,
            nonce: *Uuid::now_v7().as_bytes(),
            operation,
        })
    }
    pub fn append(&mut self, id: &Identity, op: Operation, time: i64) -> Result<Hash, LedgerError> {
        let b = id.sign(self.next_unsigned(id, op, time)?);
        self.insert(b)
    }
    pub fn insert(&mut self, b: Block) -> Result<Hash, LedgerError> {
        if b.unsigned.version != PROTOCOL_VERSION {
            return Err(LedgerError::Version);
        }
        if !b.verify_signature() {
            return Err(LedgerError::Signature);
        }
        let h = b.hash();
        if self.blocks.contains_key(&h) {
            return Err(LedgerError::Duplicate);
        }
        match self.community {
            None => match b.unsigned.operation {
                Operation::Genesis { community, .. }
                    if community == b.unsigned.community
                        && b.unsigned.sequence == 1
                        && b.unsigned.previous.is_none() => {}
                _ => return Err(LedgerError::Community),
            },
            Some(c) if c != b.unsigned.community => return Err(LedgerError::Community),
            _ => {}
        }
        match self.frontier(&b.unsigned.author) {
            None if b.unsigned.sequence == 1 && b.unsigned.previous.is_none() => {}
            Some((s, p)) if b.unsigned.sequence == s + 1 && b.unsigned.previous == Some(p) => {}
            _ => return Err(LedgerError::Frontier),
        }
        self.apply(&b, h)?;
        if self.community.is_none() {
            self.community = Some(b.unsigned.community)
        }
        self.frontiers
            .insert(b.unsigned.author, (b.unsigned.sequence, h));
        self.blocks.insert(h, b);
        Ok(h)
    }
    fn roles(&self, a: &AccountId) -> BTreeSet<String> {
        self.state
            .members
            .get(a)
            .map(|x| x.1.clone())
            .unwrap_or_default()
    }
    fn has(&self, a: &AccountId, r: &str) -> bool {
        self.roles(a).contains(r) || self.roles(a).contains("Admin")
    }
    fn governor_count(&self) -> usize {
        self.state
            .members
            .values()
            .filter(|(_, r)| r.contains("Admin") || r.contains("Auditor"))
            .count()
    }
    fn approval_threshold(&self) -> usize {
        let n = self.governor_count();
        if n <= 1 { 1 } else { (2 * n).div_ceil(3) }
    }
    fn apply(&mut self, b: &Block, h: Hash) -> Result<(), LedgerError> {
        let a = b.unsigned.author;
        match &b.unsigned.operation {
            Operation::Genesis {
                name, bit_decimals, ..
            } => {
                if self.community.is_some() || name.trim().is_empty() || *bit_decimals > 8 {
                    return Err(LedgerError::Invalid("genesis"));
                }
                self.state.members.insert(
                    a,
                    (
                        name.clone(),
                        BTreeSet::from([
                            "Admin".into(),
                            "Issuer".into(),
                            "Accountant".into(),
                            "Inventory".into(),
                        ]),
                    ),
                );
            }
            Operation::MemberAuthorize {
                account,
                name,
                roles,
            } => {
                if !self.has(&a, "Admin") {
                    return Err(LedgerError::Unauthorized);
                }
                if self.governor_count() > 1 {
                    return Err(LedgerError::Invalid(
                        "membership requires governance approvals",
                    ));
                }
                if name.trim().is_empty() || roles.is_empty() {
                    return Err(LedgerError::Invalid("member"));
                }
                self.state.members.insert(
                    *account,
                    (
                        name.clone(),
                        roles.iter().map(|r| format!("{r:?}")).collect(),
                    ),
                );
            }
            Operation::MembershipProposal {
                account,
                name,
                roles,
            } => {
                if !(self.has(&a, "Admin") || self.has(&a, "Auditor"))
                    || name.trim().is_empty()
                    || roles.is_empty()
                {
                    return Err(LedgerError::Unauthorized);
                }
                self.state.membership_proposals.insert(
                    h,
                    MembershipProposalState {
                        account: *account,
                        name: name.clone(),
                        roles: roles.clone(),
                    },
                );
            }
            Operation::GovernanceApproval { proposal } => {
                if !(self.has(&a, "Admin") || self.has(&a, "Auditor"))
                    || !self.state.membership_proposals.contains_key(proposal)
                {
                    return Err(LedgerError::Unauthorized);
                }
                self.state
                    .governance_approvals
                    .entry(*proposal)
                    .or_default()
                    .insert(a);
            }
            Operation::MembershipActivate {
                proposal,
                approvals,
            } => {
                if !(self.has(&a, "Admin") || self.has(&a, "Auditor")) {
                    return Err(LedgerError::Unauthorized);
                }
                let mut signers = BTreeSet::new();
                for approval in approvals {
                    let block = self.blocks.get(approval).ok_or(LedgerError::Link)?;
                    let Operation::GovernanceApproval { proposal: p } = block.unsigned.operation
                    else {
                        return Err(LedgerError::Link);
                    };
                    if p != *proposal {
                        return Err(LedgerError::Link);
                    }
                    signers.insert(block.unsigned.author);
                }
                if signers.len() < self.approval_threshold() {
                    return Err(LedgerError::Invalid("insufficient governance approvals"));
                }
                let p = self
                    .state
                    .membership_proposals
                    .get(proposal)
                    .ok_or(LedgerError::Link)?
                    .clone();
                self.state.members.insert(
                    p.account,
                    (p.name, p.roles.iter().map(|r| format!("{r:?}")).collect()),
                );
            }
            Operation::ChatKeyPublish { encryption_key } => {
                if !self.has(&a, "Member") || *encryption_key == [0; 32] {
                    return Err(LedgerError::Unauthorized);
                }
                self.state.chat_keys.insert(a, *encryption_key);
            }
            Operation::AssetCreate {
                asset_id,
                name,
                serial,
                location,
                value_minor,
            } => {
                if !self.has(&a, "Inventory")
                    || self.state.assets.contains_key(asset_id)
                    || name.trim().is_empty()
                    || *value_minor < 0
                {
                    return Err(LedgerError::Invalid("asset"));
                }
                self.state.assets.insert(
                    *asset_id,
                    AssetState {
                        name: name.clone(),
                        serial: serial.clone(),
                        location: location.clone(),
                        holder: None,
                        pending_offer: None,
                    },
                );
            }
            Operation::CustodyOffer {
                asset_id,
                recipient,
                ..
            } => {
                let can_inventory = self.has(&a, "Inventory");
                let asset = self
                    .state
                    .assets
                    .get_mut(asset_id)
                    .ok_or(LedgerError::Link)?;
                if asset.pending_offer.is_some()
                    || !(asset.holder == Some(a) || (asset.holder.is_none() && can_inventory))
                {
                    return Err(LedgerError::Unauthorized);
                }
                asset.pending_offer = Some(h);
                let _ = recipient;
            }
            Operation::CustodyAccept { offer } => {
                let ob = self.blocks.get(offer).ok_or(LedgerError::Link)?;
                let Operation::CustodyOffer {
                    asset_id,
                    recipient,
                    ..
                } = ob.unsigned.operation
                else {
                    return Err(LedgerError::Link);
                };
                if recipient != a {
                    return Err(LedgerError::Unauthorized);
                }
                let asset = self
                    .state
                    .assets
                    .get_mut(&asset_id)
                    .ok_or(LedgerError::Link)?;
                if asset.pending_offer != Some(*offer) {
                    return Err(LedgerError::Link);
                }
                asset.holder = Some(a);
                asset.pending_offer = None;
            }
            Operation::BitIssue {
                recipient,
                amount_minor,
                ..
            } => {
                if !self.has(&a, "Issuer") || *amount_minor <= 0 {
                    return Err(LedgerError::Unauthorized);
                }
                *self.state.bit_balances.entry(*recipient).or_default() += amount_minor;
            }
            Operation::BitSend {
                recipient,
                amount_minor,
                ..
            } => {
                if *amount_minor <= 0 {
                    return Err(LedgerError::Invalid("amount"));
                }
                let bal = self.state.bit_balances.entry(a).or_default();
                if *bal < *amount_minor {
                    return Err(LedgerError::InsufficientFunds);
                }
                *bal -= amount_minor;
                self.state
                    .pending_bit
                    .insert(h, (a, *recipient, *amount_minor));
            }
            Operation::BitReceive { send } => {
                let Some((_, recipient, amount)) = self.state.pending_bit.remove(send) else {
                    return Err(LedgerError::Link);
                };
                if recipient != a {
                    return Err(LedgerError::Unauthorized);
                }
                *self.state.bit_balances.entry(a).or_default() += amount;
            }
            Operation::SaleRecord {
                seller,
                buyer,
                amount_minor,
                description,
                ..
            } => {
                if a != *seller
                    || *amount_minor <= 0
                    || description.trim().is_empty()
                    || seller == buyer
                {
                    return Err(LedgerError::Invalid("sale"));
                }
            }
            Operation::JournalEntry {
                postings,
                description,
                ..
            } => {
                if !self.has(&a, "Accountant")
                    || description.trim().is_empty()
                    || postings.len() < 2
                {
                    return Err(LedgerError::Unauthorized);
                }
                let (d, c) = postings
                    .iter()
                    .try_fold((0i64, 0i64), |(d, c), p| {
                        if p.account.trim().is_empty()
                            || p.debit_minor < 0
                            || p.credit_minor < 0
                            || (p.debit_minor > 0 && p.credit_minor > 0)
                        {
                            None
                        } else {
                            Some((
                                d.checked_add(p.debit_minor)?,
                                c.checked_add(p.credit_minor)?,
                            ))
                        }
                    })
                    .ok_or(LedgerError::Invalid("journal overflow"))?;
                if d == 0 || d != c {
                    return Err(LedgerError::Invalid("unbalanced journal"));
                }
                for p in postings {
                    let x = self.state.journal.entry(p.account.clone()).or_default();
                    x.0 += p.debit_minor;
                    x.1 += p.credit_minor;
                }
            }
            Operation::ChatMessage { channel, body, .. } => {
                if !self.has(&a, "Member")
                    || channel.is_empty()
                    || body.is_empty()
                    || body.len() > 4096
                {
                    return Err(LedgerError::Invalid("chat"));
                }
                self.state.chats.push((a, channel.clone(), body.clone()));
            }
        };
        Ok(())
    }
}
