use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD},
    Engine,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const MAX_ENCODED_ACT_BYTES: usize = 8 * 1024 * 1024;
const DOMAIN: &str = "everyday/inventory-act/v1";

#[derive(Debug)]
pub struct CryptographicCheck {
    pub valid: bool,
    pub act: Option<Value>,
    pub public_key: Option<[u8; 32]>,
    pub hash: String,
    pub reason: &'static str,
}

pub fn decode_base64(value: &str) -> Option<Vec<u8>> {
    STANDARD
        .decode(value)
        .ok()
        .or_else(|| STANDARD_NO_PAD.decode(value).ok())
        .or_else(|| URL_SAFE_NO_PAD.decode(value).ok())
}

pub fn verify(document: &Value) -> CryptographicCheck {
    let Some(document) = document.as_object() else {
        return invalid("document_not_object");
    };
    let canonical_text = document
        .get("canonical")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if canonical_text.len() > MAX_ENCODED_ACT_BYTES {
        return invalid("document_too_large");
    }
    let canonical = decode_base64(canonical_text).unwrap_or_default();
    if canonical.is_empty() {
        return invalid("canonical_encoding_invalid");
    }
    let parsed = serde_json::from_slice::<Value>(&canonical).ok();
    if parsed.as_ref() != document.get("act") {
        return invalid("canonical_document_mismatch");
    }
    let claimed_hash = document
        .get("hash")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let actual_hash = format!("{:x}", Sha256::digest(&canonical));
    if claimed_hash.len() != 64 || claimed_hash != actual_hash {
        return invalid("hash_mismatch");
    }
    if document.get("format").and_then(Value::as_str) != Some("everyday-inventory-act")
        || document.get("version").and_then(Value::as_i64) != Some(1)
        || document.get("signatureDomain").and_then(Value::as_str) != Some(DOMAIN)
    {
        return invalid("format_or_domain_invalid");
    }
    let public_key = document
        .get("publicKey")
        .and_then(Value::as_str)
        .and_then(decode_base64)
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok());
    let signature = document
        .get("signature")
        .and_then(Value::as_str)
        .and_then(decode_base64)
        .and_then(|bytes| <[u8; 64]>::try_from(bytes).ok());
    let signature_valid = match (public_key, signature) {
        (Some(key), Some(signature)) => VerifyingKey::from_bytes(&key)
            .and_then(|verifier| {
                verifier.verify(
                    format!("{DOMAIN}\n{claimed_hash}").as_bytes(),
                    &Signature::from_bytes(&signature),
                )
            })
            .is_ok(),
        _ => false,
    };
    if !signature_valid {
        return invalid("signature_invalid");
    }
    CryptographicCheck {
        valid: true,
        act: parsed,
        public_key,
        hash: actual_hash,
        reason: "verified",
    }
}

fn invalid(reason: &'static str) -> CryptographicCheck {
    CryptographicCheck {
        valid: false,
        act: None,
        public_key: None,
        hash: String::new(),
        reason,
    }
}

pub fn standalone_report(document: &Value) -> Value {
    let check = verify(document);
    let act = check.act.as_ref();
    json!({
        "format":"everyday-inventory-act-verification","version":1,
        "cryptographicValid":check.valid,"reason":check.reason,
        "hash":if check.valid { Some(check.hash.as_str()) } else { None },
        "workspaceGuid":act.and_then(|value| value.get("workspaceGuid")).and_then(Value::as_str),
        "sessionGuid":act.and_then(|value| value.get("sessionGuid")).and_then(Value::as_str),
        "number":act.and_then(|value| value.get("number")).and_then(Value::as_str),
        "nodeTrustChecked":false,"localHistoryChecked":false,
        "warning":"Standalone-проверка подтверждает целостность и подпись файла; доверие к ноде и совпадение с летописью проверяются внутри организации"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn document() -> Value {
        let key = SigningKey::from_bytes(&[7_u8; 32]);
        let act =
            json!({"domain":DOMAIN,"workspaceGuid":"ws","sessionGuid":"session","number":"INV-1"});
        let canonical = serde_json::to_vec(&act).unwrap();
        let hash = format!("{:x}", Sha256::digest(&canonical));
        json!({
            "format":"everyday-inventory-act","version":1,"act":act,
            "canonical":STANDARD.encode(canonical),"hash":hash,
            "signature":STANDARD.encode(key.sign(format!("{DOMAIN}\n{hash}").as_bytes()).to_bytes()),
            "publicKey":STANDARD.encode(key.verifying_key().as_bytes()),"signatureDomain":DOMAIN
        })
    }

    #[test]
    fn standalone_verifier_accepts_exact_bytes_and_rejects_tampering() {
        let valid = document();
        assert!(verify(&valid).valid);
        assert_eq!(standalone_report(&valid)["number"], "INV-1");
        let mut tampered = valid;
        tampered["act"]["number"] = json!("FORGED");
        let result = verify(&tampered);
        assert!(!result.valid);
        assert_eq!(result.reason, "canonical_document_mismatch");
    }
}
