// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-git-transport::send_pack::pkt
// Responsibility:   Packet line framing shared by the send-pack client and the in-process receive-pack service.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Packet line framing shared by the send-pack client and the in-process
//! receive-pack service.

use std::io::{self, Read, Write};

/// Largest payload of one packet line (`65520` minus the 4-byte length).
pub(crate) const MAX_DATA: usize = 65516;

/// Largest payload of one side-band packet, one byte less for the band.
pub(crate) const MAX_BAND_DATA: usize = MAX_DATA - 1;

/// Length prefix size in bytes.
const PREFIX: usize = 4;

/// Side band carrying protocol data.
pub(crate) const BAND_DATA: u8 = 1;

/// Side band carrying progress and messages.
pub(crate) const BAND_PROGRESS: u8 = 2;

/// One decoded packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Packet {
    /// `0000`.
    Flush,
    /// `0001`.
    Delimiter,
    /// Payload of a data packet.
    Data(Vec<u8>),
}

/// Writes one data packet.
pub(crate) fn write_data(out: &mut dyn Write, data: &[u8]) -> io::Result<()> {
    if data.len() > MAX_DATA {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "packet line payload exceeds 65516 bytes",
        ));
    }
    write!(out, "{:04x}", data.len() + PREFIX)?;
    out.write_all(data)
}

/// Writes one text packet terminated by a line feed.
pub(crate) fn write_text(out: &mut dyn Write, text: &[u8]) -> io::Result<()> {
    let mut line = Vec::with_capacity(text.len() + 1);
    line.extend_from_slice(text);
    line.push(b'\n');
    write_data(out, &line)
}

/// Writes a flush packet.
pub(crate) fn write_flush(out: &mut dyn Write) -> io::Result<()> {
    out.write_all(b"0000")
}

/// Writes `data` on side band `band`, split into packets of the maximal size.
pub(crate) fn write_band(out: &mut dyn Write, band: u8, data: &[u8]) -> io::Result<()> {
    for chunk in data.chunks(MAX_BAND_DATA) {
        let mut packet = Vec::with_capacity(chunk.len() + 1);
        packet.push(band);
        packet.extend_from_slice(chunk);
        write_data(out, &packet)?;
    }
    Ok(())
}

/// Reads one packet; `None` at the end of the input before a packet starts.
pub(crate) fn read_packet(input: &mut dyn Read) -> io::Result<Option<Packet>> {
    let mut prefix = [0_u8; PREFIX];
    let mut filled = 0;
    while filled < PREFIX {
        let read = input.read(&mut prefix[filled..])?;
        if read == 0 {
            if filled == 0 {
                return Ok(None);
            }
            return Err(truncated());
        }
        filled += read;
    }
    let length = decode_length(&prefix)?;
    match length {
        0 => Ok(Some(Packet::Flush)),
        1 => Ok(Some(Packet::Delimiter)),
        2 | 3 => Err(invalid("reserved packet length")),
        _ => {
            let mut data = vec![0; length - PREFIX];
            input.read_exact(&mut data).map_err(|_| truncated())?;
            Ok(Some(Packet::Data(data)))
        }
    }
}

/// Decodes all packets of a complete buffer.
pub(crate) fn parse_all(mut bytes: &[u8]) -> io::Result<Vec<Packet>> {
    let mut packets = Vec::new();
    while let Some(packet) = read_packet(&mut bytes)? {
        packets.push(packet);
    }
    Ok(packets)
}

/// The payload without one trailing line feed.
pub(crate) fn trim_newline(data: &[u8]) -> &[u8] {
    data.strip_suffix(b"\n").unwrap_or(data)
}

fn decode_length(prefix: &[u8; PREFIX]) -> io::Result<usize> {
    if !prefix.iter().all(u8::is_ascii_hexdigit) {
        return Err(invalid("packet length is not hexadecimal"));
    }
    let text =
        std::str::from_utf8(prefix).map_err(|_| invalid("packet length is not hexadecimal"))?;
    let length =
        usize::from_str_radix(text, 16).map_err(|_| invalid("packet length is not hexadecimal"))?;
    if length > MAX_DATA + PREFIX {
        return Err(invalid("packet length exceeds 65520 bytes"));
    }
    Ok(length)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn truncated() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "packet line is truncated")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_text_flush_and_delimiter() {
        let mut out = Vec::new();
        write_text(&mut out, b"unpack ok").unwrap();
        out.extend_from_slice(b"0001");
        write_flush(&mut out).unwrap();
        assert_eq!(&out[..4], b"000e");
        let packets = parse_all(&out).unwrap();
        assert_eq!(
            packets,
            vec![
                Packet::Data(b"unpack ok\n".to_vec()),
                Packet::Delimiter,
                Packet::Flush
            ]
        );
        assert_eq!(trim_newline(b"ok\n"), b"ok");
        assert_eq!(trim_newline(b"ok"), b"ok");
    }

    #[test]
    fn splits_side_band_data_into_maximal_packets() {
        let data = vec![7_u8; MAX_BAND_DATA + 10];
        let mut out = Vec::new();
        write_band(&mut out, BAND_DATA, &data).unwrap();
        let packets = parse_all(&out).unwrap();
        assert_eq!(packets.len(), 2);
        let Packet::Data(first) = &packets[0] else {
            panic!("data packet expected")
        };
        assert_eq!(first.len(), MAX_DATA);
        assert_eq!(first[0], BAND_DATA);
    }

    #[test]
    fn rejects_length_prefixes_with_a_sign() {
        assert!(parse_all(b"+004").is_err());
        assert!(parse_all(b"+000").is_err());
        assert_eq!(parse_all(b"0004").unwrap(), vec![Packet::Data(Vec::new())]);
    }

    #[test]
    fn rejects_malformed_and_truncated_packets() {
        assert!(parse_all(b"zzzz").is_err());
        assert!(parse_all(b"0002").is_err());
        assert!(parse_all(b"00").is_err());
        assert!(parse_all(b"0009ab").is_err());
        assert!(parse_all(b"fff0").is_err());
        let too_long = vec![0_u8; MAX_DATA + 1];
        assert!(write_data(&mut Vec::new(), &too_long).is_err());
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
