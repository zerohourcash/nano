use bit_core::*;
use uuid::Uuid;

fn genesis(l: &mut Ledger, root: &Identity) -> CommunityId {
    let c = CommunityId(*Uuid::now_v7().as_bytes());
    l.append(
        root,
        Operation::Genesis {
            community: c,
            name: "Объект".into(),
            bit_decimals: 2,
        },
        1,
    )
    .unwrap();
    c
}
fn authorize(l: &mut Ledger, root: &Identity, id: &Identity, name: &str, roles: Vec<Role>) {
    l.append(
        root,
        Operation::MemberAuthorize {
            account: id.account(),
            name: name.into(),
            roles,
        },
        2,
    )
    .unwrap();
}

#[test]
fn bit_is_send_receive_and_cannot_double_spend() {
    let mut l = Ledger::new();
    let root = Identity::generate();
    let alice = Identity::generate();
    let bob = Identity::generate();
    genesis(&mut l, &root);
    authorize(&mut l, &root, &alice, "Alice", vec![Role::Member]);
    authorize(&mut l, &root, &bob, "Bob", vec![Role::Member]);
    l.append(
        &root,
        Operation::BitIssue {
            recipient: alice.account(),
            amount_minor: 1000,
            memo: "start".into(),
        },
        3,
    )
    .unwrap();
    let send = l
        .append(
            &alice,
            Operation::BitSend {
                recipient: bob.account(),
                amount_minor: 400,
                memo: "tools".into(),
                sale_id: None,
            },
            4,
        )
        .unwrap();
    assert_eq!(l.state().bit_balances[&alice.account()], 600);
    assert_eq!(l.state().bit_balances.get(&bob.account()), None);
    l.append(&bob, Operation::BitReceive { send }, 5).unwrap();
    assert_eq!(l.state().bit_balances[&bob.account()], 400);
    assert_eq!(
        l.append(
            &alice,
            Operation::BitSend {
                recipient: bob.account(),
                amount_minor: 700,
                memo: "bad".into(),
                sale_id: None
            },
            6
        ),
        Err(LedgerError::InsufficientFunds)
    );
}

#[test]
fn custody_requires_recipient_signature() {
    let mut l = Ledger::new();
    let root = Identity::generate();
    let worker = Identity::generate();
    let stranger = Identity::generate();
    genesis(&mut l, &root);
    authorize(&mut l, &root, &worker, "Worker", vec![Role::Member]);
    let asset = Uuid::now_v7();
    l.append(
        &root,
        Operation::AssetCreate {
            asset_id: asset,
            name: "Дрель".into(),
            serial: "D-1".into(),
            location: "Склад".into(),
            value_minor: 50000,
        },
        3,
    )
    .unwrap();
    let offer = l
        .append(
            &root,
            Operation::CustodyOffer {
                asset_id: asset,
                recipient: worker.account(),
                note: "смена".into(),
            },
            4,
        )
        .unwrap();
    assert_eq!(
        l.append(&stranger, Operation::CustodyAccept { offer }, 5),
        Err(LedgerError::Unauthorized)
    );
    l.append(&worker, Operation::CustodyAccept { offer }, 6)
        .unwrap();
    assert_eq!(l.state().assets[&asset].holder, Some(worker.account()));
}

#[test]
fn journal_must_balance() {
    let mut l = Ledger::new();
    let root = Identity::generate();
    genesis(&mut l, &root);
    let bad = Operation::JournalEntry {
        entry_id: Uuid::now_v7(),
        description: "purchase".into(),
        postings: vec![
            Posting {
                account: "inventory".into(),
                debit_minor: 100,
                credit_minor: 0,
            },
            Posting {
                account: "cash".into(),
                debit_minor: 0,
                credit_minor: 99,
            },
        ],
    };
    assert_eq!(
        l.append(&root, bad, 2),
        Err(LedgerError::Invalid("unbalanced journal"))
    );
}

#[test]
fn tampering_breaks_signature() {
    let mut l = Ledger::new();
    let root = Identity::generate();
    let c = CommunityId(*Uuid::now_v7().as_bytes());
    let unsigned = l
        .next_unsigned(
            &root,
            Operation::Genesis {
                community: c,
                name: "A".into(),
                bit_decimals: 2,
            },
            1,
        )
        .unwrap();
    let mut block = root.sign(unsigned);
    if let Operation::Genesis { name, .. } = &mut block.unsigned.operation {
        *name = "B".into()
    }
    assert_eq!(l.insert(block), Err(LedgerError::Signature));
}

#[test]
fn organization_membership_requires_governance_quorum() {
    let mut l = Ledger::new();
    let root = Identity::generate();
    let auditor = Identity::generate();
    let worker = Identity::generate();
    genesis(&mut l, &root);
    authorize(&mut l, &root, &auditor, "Auditor", vec![Role::Auditor]);
    assert_eq!(
        l.append(
            &root,
            Operation::MemberAuthorize {
                account: worker.account(),
                name: "Worker".into(),
                roles: vec![Role::Member]
            },
            3
        ),
        Err(LedgerError::Invalid(
            "membership requires governance approvals"
        ))
    );
    let proposal = l
        .append(
            &root,
            Operation::MembershipProposal {
                account: worker.account(),
                name: "Worker".into(),
                roles: vec![Role::Member],
            },
            4,
        )
        .unwrap();
    let approval1 = l
        .append(&root, Operation::GovernanceApproval { proposal }, 5)
        .unwrap();
    assert_eq!(
        l.append(
            &root,
            Operation::MembershipActivate {
                proposal,
                approvals: vec![approval1]
            },
            6
        ),
        Err(LedgerError::Invalid("insufficient governance approvals"))
    );
    let approval2 = l
        .append(&auditor, Operation::GovernanceApproval { proposal }, 7)
        .unwrap();
    l.append(
        &auditor,
        Operation::MembershipActivate {
            proposal,
            approvals: vec![approval1, approval2],
        },
        8,
    )
    .unwrap();
    assert!(l.state().members.contains_key(&worker.account()));
}
