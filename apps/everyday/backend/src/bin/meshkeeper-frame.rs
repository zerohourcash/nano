//! Transport-neutral MKST v1 bridge for serial, LoRa, USB and file pipes.
//!
//! The CLI never decrypts or interprets a payload. Each output line is one
//! base64url MKST frame and can be relayed by an untrusted transport adapter.

use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use meshkeeper_node::stream_transport::{
    fragment, validate_frame, Assembler, PayloadKind, MAX_FRAMES, MAX_TRANSFER_BYTES,
};
use std::io::{self, BufRead, BufReader, Read, Write};

const MAX_ENCODED_BYTES: usize = MAX_TRANSFER_BYTES * 2;
const MAX_INPUT_LINES: usize = MAX_FRAMES * 2;

fn usage() -> &'static str {
    "Usage:\n  meshkeeper-frame fragment <bundle|cas|interorg> <mtu>\n  meshkeeper-frame verify\n  meshkeeper-frame assemble\n\nfragment reads an opaque payload from stdin and writes one base64url MKST frame per line.\nverify and assemble read those lines in any order; assemble writes the original bytes to stdout."
}

fn parse_kind(value: &str) -> Result<PayloadKind> {
    match value {
        "bundle" => Ok(PayloadKind::EncryptedBundle),
        "cas" => Ok(PayloadKind::CasObject),
        "interorg" => Ok(PayloadKind::InterorgEnvelope),
        _ => bail!("unknown payload kind {value:?}"),
    }
}

fn read_bounded<R: Read>(reader: R) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((MAX_TRANSFER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_TRANSFER_BYTES,
        "payload exceeds {} byte limit",
        MAX_TRANSFER_BYTES
    );
    Ok(bytes)
}

fn for_each_frame<R: BufRead>(
    mut reader: R,
    mut accept: impl FnMut(&[u8]) -> Result<()>,
) -> Result<usize> {
    let mut line = String::new();
    let mut lines = 0usize;
    let mut encoded_bytes = 0usize;
    loop {
        line.clear();
        let count = reader.read_line(&mut line)?;
        if count == 0 {
            break;
        }
        encoded_bytes = encoded_bytes
            .checked_add(count)
            .context("encoded stream size overflow")?;
        ensure!(
            encoded_bytes <= MAX_ENCODED_BYTES,
            "encoded frame stream is too large"
        );
        let value = line.trim();
        if value.is_empty() {
            continue;
        }
        lines += 1;
        ensure!(lines <= MAX_INPUT_LINES, "too many input frames");
        let frame = URL_SAFE_NO_PAD
            .decode(value)
            .with_context(|| format!("invalid base64url frame on line {lines}"))?;
        accept(&frame).with_context(|| format!("invalid MKST frame on line {lines}"))?;
    }
    ensure!(lines > 0, "no MKST frames supplied");
    Ok(lines)
}

fn fragment_io<R: Read, W: Write>(
    reader: R,
    mut writer: W,
    kind: PayloadKind,
    mtu: usize,
) -> Result<()> {
    let payload = read_bounded(reader)?;
    for frame in fragment(kind, &payload, mtu)? {
        writeln!(writer, "{}", URL_SAFE_NO_PAD.encode(frame))?;
    }
    Ok(())
}

fn verify_io<R: BufRead>(reader: R) -> Result<()> {
    for_each_frame(reader, validate_frame).map(|_| ())
}

fn assemble_io<R: BufRead, W: Write>(reader: R, mut writer: W) -> Result<()> {
    let mut assembler = Assembler::default();
    let mut completed = None;
    for_each_frame(reader, |frame| {
        if let Some(value) = assembler.accept(frame)? {
            ensure!(completed.is_none(), "multiple completed transfers supplied");
            completed = Some(value);
        }
        Ok(())
    })?;
    let completed = completed.ok_or_else(|| {
        anyhow::anyhow!(
            "incomplete MKST transfer; missing ranges: {:?}",
            assembler.missing_ranges()
        )
    })?;
    writer.write_all(&completed.bytes)?;
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, kind, mtu] if command == "fragment" => {
            let kind = parse_kind(kind)?;
            let mtu = mtu.parse::<usize>().context("MTU must be an integer")?;
            fragment_io(io::stdin().lock(), io::stdout().lock(), kind, mtu)
        }
        [command] if command == "verify" => verify_io(BufReader::new(io::stdin().lock())),
        [command] if command == "assemble" => {
            assemble_io(BufReader::new(io::stdin().lock()), io::stdout().lock())
        }
        _ => bail!(usage()),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("meshkeeper-frame: {error:#}");
        std::process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_transport_round_trip_accepts_reorder_and_duplicate() {
        let payload: Vec<u8> = (0..20_000).map(|index| (index % 251) as u8).collect();
        let mut encoded = Vec::new();
        fragment_io(
            &payload[..],
            &mut encoded,
            PayloadKind::EncryptedBundle,
            128,
        )
        .unwrap();
        let mut lines: Vec<&str> = std::str::from_utf8(&encoded).unwrap().lines().collect();
        lines.reverse();
        lines.push(lines[0]);
        let input = lines.join("\n");
        verify_io(BufReader::new(input.as_bytes())).unwrap();
        let mut restored = Vec::new();
        assemble_io(BufReader::new(input.as_bytes()), &mut restored).unwrap();
        assert_eq!(restored, payload);
    }

    #[test]
    fn incomplete_and_corrupt_line_streams_fail_closed() {
        let mut encoded = Vec::new();
        fragment_io(
            &b"payload that spans several constrained radio frames"[..],
            &mut encoded,
            PayloadKind::InterorgEnvelope,
            88,
        )
        .unwrap();
        let text = String::from_utf8(encoded).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let incomplete = lines[..lines.len() - 1].join("\n");
        assert!(
            assemble_io(BufReader::new(incomplete.as_bytes()), Vec::new())
                .unwrap_err()
                .to_string()
                .contains("missing ranges")
        );

        let mut frame = URL_SAFE_NO_PAD.decode(lines[0]).unwrap();
        *frame.last_mut().unwrap() ^= 1;
        let corrupt = URL_SAFE_NO_PAD.encode(frame);
        assert!(verify_io(BufReader::new(corrupt.as_bytes())).is_err());
    }
}
