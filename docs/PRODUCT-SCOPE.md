# Product scope: autonomous organization operating system

Bit Community is a modular local-first operating system for organizations. The core defines identity, authorization, immutable evidence, synchronization and cryptographic finality. Domain modules share those primitives but keep independent retention and performance policies.

## Core modules

| Module | Records | Retention / finality |
|---|---|---|
| Identity & governance | organizations, personas, devices, roles, proposals, approvals, revocations | permanent, M-of-N finalized |
| Bit & accounting | issuance, transfers, accounts, journals, budgets, debts, taxes | permanent, double-entry, finalized |
| Inventory | assets, batches, custody, moves, maintenance, write-offs | permanent audit trail |
| Commerce | catalog, offers, orders, sales, receipts, counterparties | permanent financial evidence |
| Work | projects, tasks, shifts, requests, incidents, approvals | configurable archive |
| Documents | contracts, forms, signatures, approval workflows | signed published versions permanent |
| Knowledge | pages, revision DAG, tags, links, images, discussions | revision history, mergeable offline |
| Communication | channels, DM, announcements, files, calls | encrypted, configurable retention |
| Reporting | owner dashboard, auditor replica, exports, reconciliation | derived and reproducible |

## Owner visibility policy

The owner and authorized auditors receive complete replicas of accounting, governance, inventory, commerce, work records, published documents and knowledge. Chat policy is explicit per organization:

- `Auditable`: operational channel epoch keys are wrapped to auditor devices; personal DM remains private.
- `Private`: channel keys are available only to current channel members.
- `Transparent`: private organization messaging is disabled; all channels are auditable.

The interface must never label an auditable conversation as private. Changing an encrypted room to a weaker mode cannot decrypt historical epochs and requires governance approval.

## Extension model

Industry-specific needs use signed schema packages: field definitions, validation rules, role permissions, approval workflows, projection definitions and report templates. Extensions cannot introduce native code into the validation core or bypass ledger invariants. A schema hash and version are approved by organization governance before records using it are accepted.

Examples include construction permits, farm harvest lots, vehicle inspections, medical stock controls and cooperative work-hour accounting.
