# ADR-002: Organization chat over the mesh

Status: accepted, 2026-08-25.

Chat is a separate signed event log partitioned by `community_id` and `channel_id`. It deliberately does not extend the financial account frontier: delayed, pruned or corrupt media must never prevent custody or payment validation.

- Public organization channels are readable by authorized organization members.
- Private groups use an epoch channel key wrapped independently to each member's published X25519 key.
- Direct messages use asynchronous prekeys and a ratcheting session. The protocol target is X3DH/PQXDH plus Double Ratchet; inventing a custom ratchet is forbidden.
- Message content uses XChaCha20-Poly1305. Author, channel, epoch, sequence and reply reference are authenticated associated data.
- Events remain Ed25519-signed outside encryption, allowing untrusted relay nodes to reject forged traffic without reading content.
- Attachments are encrypted separately, addressed by ciphertext BLAKE3 hash, chunked, size-limited and optional. Text delivery cannot depend on downloading an attachment.
- Store-and-forward relays exchange per-feed frontiers, retain bounded mailboxes and acknowledge message hashes. Duplicates are safe.
- Deletion is a signed tombstone and UI policy, not a false claim that already replicated ciphertext vanished.
- Chat retention is configurable and independent from the permanent accounting ledger.
- Files are encrypted independently, split into 64 KiB chunks and addressed by ciphertext BLAKE3 hash. A signed encrypted manifest carries filename, media type, sizes and ordered chunk hashes. Downloads resume from any peer and reject altered chunks before decryption.
- Call events carry only encrypted WebRTC signaling (offer, answer, ICE, ringing, hangup). Audio/video uses WebRTC SRTP over LAN, Wi-Fi Direct, Nearby or Internet; optional TURN/SFU improves Internet and group calls. BLE/LoRa/Meshtastic carry signaling or recorded voice notes, never a falsely advertised real-time media stream.

Meshtastic-inspired channels and delayed relay are used as UX/network patterns. For cryptography, direct messages follow reviewed Signal specifications rather than a new construction.
