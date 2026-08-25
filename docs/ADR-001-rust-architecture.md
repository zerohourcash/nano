# ADR-001: Rust core, account lattice and transport boundary

Status: accepted, 2026-08-25.

## Decision

The canonical implementation is a Rust workspace. `bit-core` owns canonical serialization, cryptography, authorization, validation and deterministic projections. It must not depend on HTTP, Bluetooth, clocks, UI or a specific database. `bit-node` is the first online/LAN host. Kotlin and Swift shells will call the same core through UniFFI and provide camera, secure key storage and Nearby/Core Bluetooth transports.

A device may participate in any number of organizations. A secure device master secret derives a different signing/encryption persona for each `community_id`; public identifiers are therefore not linkable across organizations. Storage, frontiers, roles, Bit balances, chats and finality policies are partitioned by community. Cross-community value transfer is a separate bridge/escrow protocol, never an implicit merge of ledgers.

Each identity has one signed account-chain. Cross-account operations use explicit links: custody and Bit transfers require offer/send plus accept/receive. A disconnected node may accept locally valid tentative blocks. A block becomes final only under the community's configured authority policy; wall-clock order or arrival order never resolves forks.

## Non-negotiable invariants

1. Canonical bytes are versioned, deterministic and domain-separated.
2. No floating point values: Bit and money use signed integer minor units.
3. Only a key owner extends its account-chain.
4. Sequence and previous hash must match the frontier.
5. Every debit has an equal credit in the accounting projection.
6. Currency issuance requires an explicit role and remains auditable from genesis.
7. Custody acceptance is signed by the recipient; assignment alone does not prove receipt.
8. Unknown operations and protocol versions fail closed.
9. Transport is untrusted; imported blocks receive exactly the same validation as local blocks.
10. Conflicting account heads are retained as evidence and excluded from finalized state.

## Influences

- Nano: per-account chains, state transitions, cross-chain links and frontier synchronization.
- Erachain: community genesis, native assets, rights and an independently deployable local chain.
- Cyclos: member/system accounts, configurable transfer types, approvals, marketplace and community currency operations.
- Double-entry accounting: immutable journal entries whose debits equal credits.

The project does not copy Erachain code, consensus or AGPL implementation. It does not call activity rewards "mining" and does not claim offline global finality.
