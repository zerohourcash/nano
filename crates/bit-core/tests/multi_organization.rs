use bit_core::*;
use std::collections::BTreeSet;
use uuid::Uuid;

fn add_member(
    ledger: &mut Ledger,
    root: &Identity,
    auditor: &Identity,
    member: &Identity,
    index: i64,
) {
    let proposal = ledger
        .append(
            root,
            Operation::MembershipProposal {
                account: member.account(),
                name: format!("Worker {index}"),
                roles: vec![Role::Member],
            },
            index * 10,
        )
        .unwrap();
    let first = ledger
        .append(
            root,
            Operation::GovernanceApproval { proposal },
            index * 10 + 1,
        )
        .unwrap();
    let second = ledger
        .append(
            auditor,
            Operation::GovernanceApproval { proposal },
            index * 10 + 2,
        )
        .unwrap();
    ledger
        .append(
            root,
            Operation::MembershipActivate {
                proposal,
                approvals: vec![first, second],
            },
            index * 10 + 3,
        )
        .unwrap();
}

fn organization(name: &str, count: usize) -> (Ledger, Identity, Identity, Vec<Identity>) {
    let mut ledger = Ledger::new();
    let root = Identity::generate();
    let auditor = Identity::generate();
    let community = CommunityId(*Uuid::now_v7().as_bytes());
    ledger
        .append(
            &root,
            Operation::Genesis {
                community,
                name: name.into(),
                bit_decimals: 2,
            },
            1,
        )
        .unwrap();
    ledger
        .append(
            &root,
            Operation::MemberAuthorize {
                account: auditor.account(),
                name: "Auditor".into(),
                roles: vec![Role::Auditor, Role::Member],
            },
            2,
        )
        .unwrap();
    let members: Vec<_> = (0..count).map(|_| Identity::generate()).collect();
    for (index, member) in members.iter().enumerate() {
        add_member(&mut ledger, &root, &auditor, member, index as i64 + 1);
    }
    (ledger, root, auditor, members)
}

#[test]
fn three_organizations_with_thirty_people_remain_isolated_and_consistent() {
    let mut organizations: Vec<_> = (0..3)
        .map(|i| organization(&format!("Organization {i}"), 30))
        .collect();
    let mut communities = Vec::new();
    for (ledger, root, _, members) in &mut organizations {
        communities.push(ledger.community().unwrap());
        for member in members.iter() {
            ledger
                .append(
                    root,
                    Operation::BitIssue {
                        recipient: member.account(),
                        amount_minor: 10_000,
                        memo: "monthly allocation".into(),
                    },
                    1000,
                )
                .unwrap();
        }
        for index in 0..members.len() {
            let recipient = &members[(index + 1) % members.len()];
            let send = ledger
                .append(
                    &members[index],
                    Operation::BitSend {
                        recipient: recipient.account(),
                        amount_minor: 125,
                        memo: "canteen".into(),
                        sale_id: Some(Uuid::now_v7()),
                    },
                    1100 + index as i64,
                )
                .unwrap();
            ledger
                .append(
                    recipient,
                    Operation::BitReceive { send },
                    1200 + index as i64,
                )
                .unwrap();
        }
        for (index, worker) in members.iter().take(10).enumerate() {
            let asset = Uuid::now_v7();
            ledger
                .append(
                    root,
                    Operation::AssetCreate {
                        asset_id: asset,
                        name: format!("Tool {index}"),
                        serial: format!("SN-{index}"),
                        location: "Warehouse".into(),
                        value_minor: 50_000,
                    },
                    1300 + index as i64,
                )
                .unwrap();
            let offer = ledger
                .append(
                    root,
                    Operation::CustodyOffer {
                        asset_id: asset,
                        recipient: worker.account(),
                        note: "shift".into(),
                    },
                    1400 + index as i64,
                )
                .unwrap();
            ledger
                .append(
                    worker,
                    Operation::CustodyAccept { offer },
                    1500 + index as i64,
                )
                .unwrap();
        }
        assert_eq!(ledger.state().members.len(), 32);
        assert_eq!(ledger.state().assets.len(), 10);
        assert_eq!(ledger.state().bit_balances.values().sum::<i64>(), 300_000);
        assert!(
            ledger
                .state()
                .assets
                .values()
                .all(|asset| asset.holder.is_some())
        );
    }
    assert_eq!(communities.iter().collect::<BTreeSet<_>>().len(), 3);
    let foreign = organizations[0].0.blocks().next().unwrap().1.clone();
    assert_eq!(
        organizations[1].0.insert(foreign),
        Err(LedgerError::Community)
    );
}

#[test]
fn delayed_dependencies_recover_but_account_fork_is_rejected() {
    let (common, root, _auditor, members) = organization("Partition", 3);
    let mut common = common;
    for member in &members {
        common
            .append(
                &root,
                Operation::BitIssue {
                    recipient: member.account(),
                    amount_minor: 1000,
                    memo: "seed".into(),
                },
                100,
            )
            .unwrap();
    }
    let mut source = common.clone();
    let mut delayed = common.clone();
    let first = source
        .append(
            &members[0],
            Operation::BitSend {
                recipient: members[1].account(),
                amount_minor: 100,
                memo: "one".into(),
                sale_id: None,
            },
            200,
        )
        .unwrap();
    let second = source
        .append(
            &members[0],
            Operation::BitSend {
                recipient: members[2].account(),
                amount_minor: 100,
                memo: "two".into(),
                sale_id: None,
            },
            201,
        )
        .unwrap();
    let first_block = source
        .blocks()
        .find(|(hash, _)| **hash == first)
        .unwrap()
        .1
        .clone();
    let second_block = source
        .blocks()
        .find(|(hash, _)| **hash == second)
        .unwrap()
        .1
        .clone();
    assert_eq!(
        delayed.insert(second_block.clone()),
        Err(LedgerError::Frontier)
    );
    delayed.insert(first_block).unwrap();
    delayed.insert(second_block).unwrap();
    let mut partition_a = common.clone();
    let mut partition_b = common.clone();
    let a = partition_a
        .append(
            &members[0],
            Operation::BitSend {
                recipient: members[1].account(),
                amount_minor: 50,
                memo: "A".into(),
                sale_id: None,
            },
            300,
        )
        .unwrap();
    let b = partition_b
        .append(
            &members[0],
            Operation::BitSend {
                recipient: members[2].account(),
                amount_minor: 60,
                memo: "B".into(),
                sale_id: None,
            },
            300,
        )
        .unwrap();
    let block_b = partition_b
        .blocks()
        .find(|(hash, _)| **hash == b)
        .unwrap()
        .1
        .clone();
    assert_eq!(partition_a.insert(block_b), Err(LedgerError::Frontier));
    assert_ne!(a, b);
}
