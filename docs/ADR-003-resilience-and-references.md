# ADR-003: Resilience references and missing mechanisms

Status: accepted, 2026-08-25.

## Adopt

- Nano: per-account frontier scans, explicit dependency tracking, duplicate filters, bounded unconfirmed backlog, fork-storm tests and irreversible confirmed frontiers. Voting weight is replaced by organization governance, not copied.
- MLS RFC 9420: group epochs, KeyPackages, Welcome, Proposal and Commit for asynchronous E2E group chat with forward secrecy and post-compromise security.
- Signal: PQXDH/Double Ratchet/Sesame for asynchronous multi-device direct messages. No custom ratchet.
- Briar: authenticated contacts, transport plugins for Bluetooth/Wi-Fi/Internet/removable media, local encrypted storage and mailbox-style delayed delivery.
- Waku: separate Relay, Filter, Store and Light Push roles; GossipSub scoring and optionally RLN for privacy-preserving rate limits.
- GNU Taler: explicit offline risk limits, later double-spend detection, audited issuance and separation of payer privacy from merchant/accounting transparency.
- Cyclos: configurable account/transfer types, approvals, marketplace and operational reporting.

## Required additions

1. Keep tentative, confirmed and conflicting states separate. Arrival time never finalizes value.
2. Bound pending blocks per account/community/peer and request missing dependencies by hash.
3. Preserve signed forks as evidence while excluding them from spendable balances.
4. Cement finalized frontiers so later votes cannot rewrite accepted accounting history.
5. Set per-person and per-device offline spending/custody limits. High-value actions require online or local M-of-N cosigning.
6. Use platform hardware-backed keys when available, encrypted recovery shares and revocable device certificates.
7. Separate permanent financial retention, operational history, chat retention and encrypted attachment caches.
8. Protect metadata: organization-scoped personas, padded message classes, private contact discovery and optional Tor when Internet exists.
9. Maintain cryptographic agility through versioned suites; protocol upgrades require governance approval and downgrade protection.
10. Require reproducible builds, SBOM, dependency audits, fuzz/property tests, corruption recovery drills and external security review before stable release.

## Explicit limitations

No software-only phone system can guarantee prevention of all double spending while every witness and authority is unreachable. It can constrain exposure, require counterparty signatures, detect forks later and refuse final settlement until policy quorum is observed. BLE and LoRa cannot provide live voice bandwidth; they can carry signaling and voice notes.
