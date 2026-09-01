//! Transport-neutral framing for encrypted sync bundles and CAS payloads.
//!
//! BLE GATT, Bluetooth Mesh, Wi-Fi Direct, LoRa and serial adapters only need
//! to move these opaque frames. They never parse or re-sign ledger data.

use anyhow::{bail, ensure, Context};
use rand::RngCore;
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 4] = b"MKST";
const VERSION: u8 = 1;
const HEADER_BYTES: usize = 76;
pub const MAX_TRANSFER_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_FRAMES: usize = 262_144;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PayloadKind {
    EncryptedBundle = 1,
    CasObject = 2,
}

impl TryFrom<u8> for PayloadKind {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::EncryptedBundle),
            2 => Ok(Self::CasObject),
            _ => bail!("unknown stream payload kind"),
        }
    }
}

#[derive(Debug)]
struct Frame<'a> {
    kind: PayloadKind,
    transfer_id: [u8; 16],
    total_bytes: usize,
    sequence: usize,
    frame_count: usize,
    digest: [u8; 32],
    payload: &'a [u8],
}

fn chunk_tag(sequence: usize, payload: &[u8]) -> [u8; 8] {
    let mut hash = Sha256::new();
    hash.update(b"meshkeeper/stream-frame/v1\0");
    hash.update((sequence as u32).to_be_bytes());
    hash.update(payload);
    hash.finalize()[..8].try_into().expect("fixed slice")
}

fn parse_frame(bytes: &[u8]) -> anyhow::Result<Frame<'_>> {
    ensure!(bytes.len() >= HEADER_BYTES, "stream frame is truncated");
    ensure!(&bytes[..4] == MAGIC, "invalid stream frame magic");
    ensure!(bytes[4] == VERSION, "unsupported stream frame version");
    let kind = PayloadKind::try_from(bytes[5])?;
    let transfer_id = bytes[6..22].try_into().expect("fixed slice");
    let total_bytes = u32::from_be_bytes(bytes[22..26].try_into().unwrap()) as usize;
    let sequence = u32::from_be_bytes(bytes[26..30].try_into().unwrap()) as usize;
    let frame_count = u32::from_be_bytes(bytes[30..34].try_into().unwrap()) as usize;
    let payload_bytes = u16::from_be_bytes(bytes[34..36].try_into().unwrap()) as usize;
    let digest = bytes[36..68].try_into().expect("fixed slice");
    let supplied_tag: [u8; 8] = bytes[68..76].try_into().expect("fixed slice");
    ensure!(
        total_bytes <= MAX_TRANSFER_BYTES,
        "stream transfer exceeds size limit"
    );
    ensure!(
        (1..=MAX_FRAMES).contains(&frame_count),
        "invalid stream frame count"
    );
    ensure!(
        total_bytes == 0 || frame_count <= total_bytes,
        "stream frame count exceeds payload size"
    );
    ensure!(
        sequence < frame_count,
        "stream frame sequence is out of range"
    );
    ensure!(
        bytes.len() == HEADER_BYTES + payload_bytes,
        "stream frame length mismatch"
    );
    let payload = &bytes[HEADER_BYTES..];
    ensure!(
        (total_bytes == 0 && frame_count == 1 && payload.is_empty())
            || (total_bytes > 0 && !payload.is_empty() && payload.len() <= total_bytes),
        "invalid stream payload dimensions"
    );
    ensure!(
        chunk_tag(sequence, payload) == supplied_tag,
        "stream frame checksum mismatch"
    );
    Ok(Frame {
        kind,
        transfer_id,
        total_bytes,
        sequence,
        frame_count,
        digest,
        payload,
    })
}

/// Performs bounded structural and per-chunk verification without allocating
/// an assembly buffer. Platform radio layers call this before retaining bytes.
pub fn validate_frame(bytes: &[u8]) -> anyhow::Result<()> {
    parse_frame(bytes).map(|_| ())
}

/// Splits opaque encrypted data into independently verifiable MTU-sized frames.
pub fn fragment(kind: PayloadKind, payload: &[u8], mtu: usize) -> anyhow::Result<Vec<Vec<u8>>> {
    ensure!(
        payload.len() <= MAX_TRANSFER_BYTES,
        "stream transfer exceeds size limit"
    );
    ensure!(
        mtu > HEADER_BYTES && mtu <= u16::MAX as usize,
        "invalid transport MTU"
    );
    let capacity = mtu - HEADER_BYTES;
    let frame_count = payload.len().max(1).div_ceil(capacity);
    ensure!(
        frame_count <= MAX_FRAMES,
        "stream transfer has too many frames"
    );
    let mut transfer_id = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut transfer_id);
    let digest: [u8; 32] = Sha256::digest(payload).into();
    let mut frames = Vec::with_capacity(frame_count);
    for sequence in 0..frame_count {
        let start = sequence * capacity;
        let end = payload.len().min(start + capacity);
        let chunk = &payload[start..end];
        let mut frame = Vec::with_capacity(HEADER_BYTES + chunk.len());
        frame.extend_from_slice(MAGIC);
        frame.push(VERSION);
        frame.push(kind as u8);
        frame.extend_from_slice(&transfer_id);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(sequence as u32).to_be_bytes());
        frame.extend_from_slice(&(frame_count as u32).to_be_bytes());
        frame.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        frame.extend_from_slice(&digest);
        frame.extend_from_slice(&chunk_tag(sequence, chunk));
        frame.extend_from_slice(chunk);
        frames.push(frame);
    }
    Ok(frames)
}

#[derive(Debug, Eq, PartialEq)]
pub struct CompletedTransfer {
    pub kind: PayloadKind,
    pub transfer_id: [u8; 16],
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Assembler {
    metadata: Option<(PayloadKind, [u8; 16], usize, usize, [u8; 32])>,
    frames: Vec<Option<Vec<u8>>>,
    received_bytes: usize,
}

impl Assembler {
    /// Accepts frames in any order. Identical retries are idempotent; a
    /// conflicting retry or a second transfer is rejected without mutation.
    pub fn accept(&mut self, encoded: &[u8]) -> anyhow::Result<Option<CompletedTransfer>> {
        let frame = parse_frame(encoded)?;
        let metadata = (
            frame.kind,
            frame.transfer_id,
            frame.total_bytes,
            frame.frame_count,
            frame.digest,
        );
        if let Some(existing) = self.metadata {
            ensure!(
                existing == metadata,
                "frame belongs to another or conflicting transfer"
            );
        } else {
            self.metadata = Some(metadata);
            self.frames.resize_with(frame.frame_count, || None);
        }
        if let Some(existing) = &self.frames[frame.sequence] {
            ensure!(
                existing.as_slice() == frame.payload,
                "conflicting duplicate stream frame"
            );
            return Ok(None);
        }
        let received_bytes = self
            .received_bytes
            .checked_add(frame.payload.len())
            .context("stream byte counter overflow")?;
        ensure!(
            received_bytes <= frame.total_bytes,
            "stream payload exceeds declared size"
        );
        self.received_bytes = received_bytes;
        self.frames[frame.sequence] = Some(frame.payload.to_vec());
        if self.frames.iter().any(Option::is_none) {
            return Ok(None);
        }
        ensure!(
            self.received_bytes == frame.total_bytes,
            "stream payload size mismatch"
        );
        let mut bytes = Vec::with_capacity(frame.total_bytes);
        for chunk in &self.frames {
            bytes.extend_from_slice(chunk.as_ref().unwrap());
        }
        ensure!(
            <[u8; 32]>::from(Sha256::digest(&bytes)) == frame.digest,
            "completed stream digest mismatch"
        );
        Ok(Some(CompletedTransfer {
            kind: frame.kind,
            transfer_id: frame.transfer_id,
            bytes,
        }))
    }

    /// Compact inclusive ranges requested from an unreliable/store-and-forward channel.
    pub fn missing_ranges(&self) -> Vec<(u32, u32)> {
        let mut ranges = Vec::new();
        let mut start = None;
        for (index, frame) in self.frames.iter().enumerate() {
            match (start, frame.is_none()) {
                (None, true) => start = Some(index),
                (Some(from), false) => {
                    ranges.push((from as u32, (index - 1) as u32));
                    start = None;
                }
                _ => {}
            }
        }
        if let Some(from) = start {
            ranges.push((from as u32, (self.frames.len() - 1) as u32));
        }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_loss_duplicate_and_resume_round_trip() {
        let payload: Vec<u8> = (0..20_000).map(|index| (index % 251) as u8).collect();
        let frames = fragment(PayloadKind::EncryptedBundle, &payload, 185).unwrap();
        let missing = [2usize, 7, frames.len() - 1];
        let mut assembler = Assembler::default();
        for index in (0..frames.len())
            .rev()
            .filter(|index| !missing.contains(index))
        {
            assert!(assembler.accept(&frames[index]).unwrap().is_none());
        }
        assert!(!assembler.missing_ranges().is_empty());
        assert!(assembler.accept(&frames[0]).unwrap().is_none()); // retry
        for index in missing {
            let result = assembler.accept(&frames[index]).unwrap();
            if index == frames.len() - 1 {
                assert_eq!(result.unwrap().bytes, payload);
            }
        }
        assert!(assembler.missing_ranges().is_empty());
    }

    #[test]
    fn corruption_conflicting_retry_and_interleaving_are_rejected() {
        let first = fragment(PayloadKind::EncryptedBundle, b"first payload over mesh", 96).unwrap();
        let second = fragment(PayloadKind::CasObject, b"another payload", 96).unwrap();
        let mut corrupted = first[0].clone();
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(parse_frame(&corrupted)
            .unwrap_err()
            .to_string()
            .contains("checksum"));
        let mut assembler = Assembler::default();
        assembler.accept(&first[0]).unwrap();
        assert!(assembler
            .accept(&second[0])
            .unwrap_err()
            .to_string()
            .contains("another"));
        let mut conflicting = first[0].clone();
        conflicting[68] ^= 1;
        assert!(assembler.accept(&conflicting).is_err());
    }

    #[test]
    fn bounds_and_empty_payload_are_safe() {
        assert!(fragment(PayloadKind::EncryptedBundle, &[0; 10], HEADER_BYTES).is_err());
        assert!(fragment(
            PayloadKind::EncryptedBundle,
            &[0; MAX_TRANSFER_BYTES + 1],
            512
        )
        .is_err());
        let frames = fragment(PayloadKind::EncryptedBundle, b"", 96).unwrap();
        let completed = Assembler::default().accept(&frames[0]).unwrap().unwrap();
        assert!(completed.bytes.is_empty());
    }

    #[test]
    fn malformed_prefixes_and_header_bits_never_panic_or_allocate_unboundedly() {
        let frame = fragment(PayloadKind::EncryptedBundle, b"bounded parser input", 128)
            .unwrap()
            .remove(0);
        for length in 0..frame.len() {
            assert!(parse_frame(&frame[..length]).is_err());
        }
        for index in 0..HEADER_BYTES {
            let mut changed = frame.clone();
            changed[index] ^= 0x80;
            let _ = parse_frame(&changed);
        }
        let mut absurd_count = frame;
        absurd_count[30..34].copy_from_slice(&(MAX_FRAMES as u32).to_be_bytes());
        assert!(parse_frame(&absurd_count).is_err());
    }
}
