# ADR-004: local AI is an untrusted proposal generator

Status: accepted for prototype.

## Decision

The phone or desktop may load a local model and let a member discuss work, search only the data available to that persona, summarize events and propose actions. The first inference adapter is `llama.cpp` with a user-supplied GGUF model because it supports Android, iOS and major desktop systems. The protocol remains runtime-independent so MLC, ExecuTorch or a remote model can be added without changing authorization.

The trust boundary is fixed:

```text
untrusted model output
  -> bounded DraftIntent schema
  -> deterministic PolicyEngine
  -> exact human-readable preview
  -> OS keystore/biometric confirmation when required
  -> user-signed domain command
  -> normal ledger/chat validation and audit replication
```

The model process never receives identity seed material, database master keys, raw capability credentials, a shell, arbitrary network access or a direct ledger append function. Retrieved chat and knowledge text is data, never policy or tool instructions. Every evaluated proposal and its outcome is intended to become a signed audit event visible under the organization's normal owner/auditor policy.

## Risk rules

- A message draft, task draft or knowledge draft may be automated only with an explicit, expiring, use-limited capability scoped to one organization and allowed channels.
- A purchase request may have a strict integer amount ceiling; it is not a payment.
- Bit transfers/issuance, custody transfer, QR checkout acceptance, membership, roles, governance, key operations and audit deletion always require human confirmation and the regular domain authorization path.
- Community/persona mismatch, expiry, replay, unknown versions and capability limit violations fail closed.
- The interface must display the model identity, target organization, action, recipients, amounts and evidence before confirmation. Model prose cannot obscure those fields.

## Mobile operation

Models and indexes are optional encrypted local blobs. The application must continue all ledger, inventory and communication functions when inference is absent or the device lacks memory. Model downloading is never required for organization sync. Per-organization retrieval indexes prevent cross-organization context leakage; private-message content is excluded unless the current persona explicitly selects it.

## Remaining production work

- Isolated inference worker and OS sandbox adapters for Android/iOS/desktop.
- Signed capability issuance/revocation in governance and a durable proposal audit stream.
- Memory/thermal benchmarks and model compatibility manifest with hash, license and provenance.
- Red-team corpus for direct/indirect prompt injection, poisoned knowledge, Unicode ambiguity and denial-of-service output.
- Accessible transaction previews and platform biometric/key-store integration.

This design follows OWASP guidance on prompt injection and excessive agency: minimize extensions, permissions and autonomy, validate outputs, and require user approval for high-impact actions.
