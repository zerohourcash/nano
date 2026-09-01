"""Ed25519 request proof used by HTTP smoke tests."""

import base64
import hashlib
import time
import uuid

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

DOMAIN = "everyday/device-request/v1"
CRITICAL = {
    "items.create", "items.update", "items.remove", "items.addPhoto", "items.addComment", "items.reportFault", "items.resolveFault", "items.requestChange", "items.decideChange",
    "transfers.take", "transfers.takeMany", "transfers.returnItem",
    "transfers.prepare", "transfers.accept", "transfers.reject", "transfers.acceptAll",
    "history.writeOff", "history.replenish", "history.move",
    "inventory.create", "inventory.checkItem", "inventory.complete",
    "chat.send", "items.addDocument", "bit.transfer", "bit.sale", "bit.mint", "knowledge.save", "sync.importBundle",
    "interorg.ensureIdentity", "interorg.trustContact", "interorg.revokeContact", "interorg.send", "interorg.accept",
    "sync.approveNodeKey", "sync.revokeNodeKey", "sync.addPeer", "sync.removePeer", "sync.pullNow", "sync.resolveConflict",
    "sync.clearDiagnostics", "sync.reportTransportStatus", "content.setMode", "content.ingest", "content.pin", "content.unpin",
    "backup.export", "backup.import", "profile.update", "profile.changePassword", "profile.leaveWorkspace", "profile.deleteAccount", "auth.revokeDevice",
    "admin.users.create", "admin.users.update", "admin.users.remove", "admin.users.invite",
    "admin.workspaces.create", "admin.workspaces.update", "admin.workspaces.remove",
    "admin.workspaces.createInvite", "admin.organizationNodes.create",
    "admin.organizationNodes.update", "admin.organizationNodes.remove",
    "admin.storages.create", "admin.storages.update", "admin.storages.remove",
    "admin.buildingSites.create", "admin.buildingSites.update", "admin.buildingSites.remove",
    "admin.dictionaries.create", "admin.dictionaries.update", "admin.dictionaries.remove",
}


def b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


class DeviceSigner:
    def __init__(self, name: str):
        self.private = Ed25519PrivateKey.generate()
        self.device_id = f"test-device-{uuid.uuid4().hex}"
        self.name = name

    @property
    def public_key(self) -> str:
        raw = self.private.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        return b64url(raw)

    def headers(self, path: str, body: bytes) -> dict[str, str]:
        timestamp = str(int(time.time()))
        nonce = uuid.uuid4().hex
        digest = hashlib.sha256(body).hexdigest()
        message = f"{DOMAIN}\nPOST\n{path}\n{timestamp}\n{nonce}\n{digest}".encode()
        return {
            "x-everyday-device": self.device_id,
            "x-everyday-timestamp": timestamp,
            "x-everyday-nonce": nonce,
            "x-everyday-signature": b64url(self.private.sign(message)),
        }
