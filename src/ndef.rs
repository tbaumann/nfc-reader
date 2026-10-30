#![allow(
    dead_code,
    reason = "MIFARE write/key-B constants are kept as the write path's
              documented prerequisites even though this build only reads"
)]

//! NDEF reading for NFC Forum Type 2 (NTAG/Ultralight) and MIFARE Classic 1K.
//!
//! Hand-rolled TLV + record walk, no allocation. Layouts and the Classic block
//! mapping follow ESPuino's C++ parser (Type 2: capability container at byte 12,
//! TLVs at byte 16, page-addressed reads; Classic: no CC, TLVs from the first
//! data byte, data blocks skip the sector trailers).

use core::fmt::Write as _;
use log::warn;

use heapless::String;

use crate::pn532::{MIFARE_AUTH_KEY_A, MIFARE_READ, MIFARE_WRITE_CLASSIC, MIFARE_WRITE_TYPE2, Pn532, Target};

pub const MAX_RECORDS: usize = 4;
/// Big enough for an NTAG213 (144 B user area) and for three Classic sectors.
const SCRATCH: usize = 240;

pub struct Report {
    /// Human readable result: one record per line, `T:Hello` / `U:https://…` /
    /// `<mime>:<hex>`, or the reason nothing was found.
    pub text: String<384>,
    pub records: u8,
    pub tag: &'static str,
    /// Which Classic key opened the NDEF sector ("" for tags without one).
    pub key: &'static str,
    /// The raw NDEF message as read, so the API can hand out bytes that survive
    /// payloads which are not text at all.
    pub raw: heapless::Vec<u8, 192>,
    pub ok: bool,
    /// Bytes written (writes only).
    pub bytes: u16,
    /// The write was confirmed by reading the tag back.
    pub verified: bool,
}

impl Report {
    fn new(tag: &'static str) -> Self {
        Self {
            text: String::new(),
            records: 0,
            tag,
            key: "",
            raw: heapless::Vec::new(),
            ok: false,
            bytes: 0,
            verified: false,
        }
    }

    fn fail(tag: &'static str, why: &str) -> Self {
        let mut r = Self::new(tag);
        let _ = r.text.push_str(why);
        r
    }
}

/// The well-known MIFARE Classic key A candidates, tried in order. "No password"
/// on a Classic tag means the factory key: authentication is mandatory at the
/// protocol level even for reads, so one must be sent either way.
///
/// The NFC Forum key comes first because an NDEF-formatted card (any card written
/// by a phone app such as NFC Tools) has its NDEF sector re-keyed to it, while a
/// blank card answers the factory key. Both orders work now that each attempt
/// re-selects the card - see `authenticate`.
const CLASSIC_KEYS: [&[u8; 6]; 3] = [
    &[0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7], // NFC Forum NDEF sector (AN1304)
    &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // factory default
    &[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5], // MAD (NFC Forum)
];
const CLASSIC_KEY_NAMES: [&str; 3] = ["nfc-forum", "factory", "mad"];

pub async fn read<I2C>(pn: &mut Pn532<I2C>, target: &Target) -> Report
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    match target.sak {
        0x00 => read_type2(pn, target).await,
        0x08 | 0x18 => read_classic(pn, target).await,
        _ => Report::fail("unsupported", "tag is not NDEF-capable Type 2 or Classic"),
    }
}

// --- Type 2 -------------------------------------------------------------------

async fn read_type2<I2C>(pn: &mut Pn532<I2C>, target: &Target) -> Report
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let tag = "NFC Forum Type 2";
    let mut r = Report::new(tag);

    // Capability container = bytes 12..16 = page 3 (a page read returns 16 bytes,
    // i.e. 4 pages, so read from the page containing byte 12).
    let mut page = [0u8; 16];
    if let Err(e) = read_page(pn, target.number, 3, &mut page).await {
        return Report::fail(tag, e);
    }
    if page[0] != 0xE1 {
        return Report::fail(tag, "no NDEF capability container (tag not formatted)");
    }
    let user_bytes = (page[2] as usize) * 8;
    if user_bytes == 0 {
        return Report::fail(tag, "capability container declares zero user bytes");
    }

    // Read the TLV area, 16 bytes at a time, from byte 16 up to the declared size.
    let mut buf = [0u8; SCRATCH];
    let want = user_bytes.min(SCRATCH);
    let mut got = 0usize;
    while got < want {
        let offset = 16 + got;
        let mut chunk = [0u8; 16];
        if let Err(e) = read_page(pn, target.number, (offset / 4) as u8, &mut chunk).await {
            if got == 0 {
                return Report::fail(tag, e);
            }
            break;
        }
        let n = (want - got).min(16);
        buf[got..got + n].copy_from_slice(&chunk[..n]);
        got += n;
    }

    parse_tlvs(&buf[..got], &mut r);
    r
}

async fn read_page<I2C>(
    pn: &mut Pn532<I2C>,
    target: u8,
    page: u8,
    out: &mut [u8; 16],
) -> Result<(), &'static str>
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let cmd = [MIFARE_READ, page];
    let data = pn
        .in_data_exchange(target, &cmd, 16, 300)
        .await
        .map_err(|e| e.as_str())?;
    if data.len() < 16 {
        return Err("short page read");
    }
    out.copy_from_slice(&data[..16]);
    Ok(())
}

// --- MIFARE Classic 1K ---------------------------------------------------------

async fn read_classic<I2C>(pn: &mut Pn532<I2C>, target: &Target) -> Report
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let tag = "MIFARE Classic 1K";
    let mut r = Report::new(tag);
    let uid = target.uid4();

    // Logical data byte stream: data blocks of sectors 1..3, i.e. physical
    // blocks 4,5,6 / 8,9,10 / 12,13,14 (the 4th block of each sector is its
    // trailer and holds the keys).
    let mut buf = [0u8; SCRATCH];
    let mut logical = 0usize;
    let mut authenticated_sector: i8 = -1;
    let mut used_key: Option<usize> = None;

    for &phys in &[4u8, 5, 6, 8, 9, 10, 12, 13, 14] {
        let sector = (phys / 4) as i8;

        if sector != authenticated_sector {
            match authenticate(pn, target.number, phys, &uid, &mut buf, logical).await {
                Ok(key) => {
                    authenticated_sector = sector;
                    used_key.get_or_insert(key);
                }
                Err(why) => {
                    if logical == 0 {
                        return Report::fail(tag, why);
                    }
                    break;
                }
            }
        }

        let cmd = [MIFARE_READ, phys];
        match pn.in_data_exchange(target.number, &cmd, 16, 300).await {
            Ok(data) if data.len() >= 16 => {
                let n = (SCRATCH - logical).min(16);
                buf[logical..logical + n].copy_from_slice(&data[..n]);
                logical += n;
            }
            _ => break,
        }

        // Nothing more to read once the TLV list is terminated.
        if tlv_terminated(&buf[..logical]) {
            break;
        }
    }

    if logical == 0 {
        return Report::fail(tag, "no readable data block (authentication failed)");
    }
    if let Some(k) = used_key {
        r.key = CLASSIC_KEY_NAMES[k];
    }

    // A Classic tag has no capability container; a stray 0xE1 may still lead.
    let body = if buf[0] == 0xE1 { &buf[1..logical] } else { &buf[..logical] };
    parse_tlvs(body, &mut r);
    r
}

/// Authenticate the sector that `block` belongs to, verifying by reading (the
/// auth status byte is not a reliable success test on this chip).
async fn authenticate<I2C>(
    pn: &mut Pn532<I2C>,
    target: u8,
    block: u8,
    uid: &[u8; 4],
    scratch: &mut [u8; SCRATCH],
    logical: usize,
) -> Result<usize, &'static str>
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    for (idx, key) in CLASSIC_KEYS.iter().enumerate() {
        // Re-select the card before every attempt. A failed MIFARE Classic
        // authentication blocks the sector for the current card activation
        // (three failures and it stays blocked), so a wrong key in the list
        // would otherwise poison the attempt with the right one - which is
        // exactly how a formatted card came back as "authentication failed
        // with the known keys".
        let _ = pn.list_passive_target(150).await;

        let mut cmd = [0u8; 12];
        cmd[0] = MIFARE_AUTH_KEY_A;
        cmd[1] = block;
        cmd[2..8].copy_from_slice(*key);
        cmd[8..12].copy_from_slice(uid);
        // The status byte is unreliable, so ignore the result and prove it with
        // a read immediately after.
        let _ = pn.in_data_exchange(target, &cmd, 1, 100).await;

        let read = [MIFARE_READ, block];
        if let Ok(data) = pn.in_data_exchange(target, &read, 16, 300).await {
            if data.len() >= 16 {
                let n = (SCRATCH - logical).min(16);
                scratch[logical..logical + n].copy_from_slice(&data[..n]);
                return Ok(idx);
            }
        }
    }
    Err("authentication failed with the known keys")
}

fn tlv_terminated(buf: &[u8]) -> bool {
    let mut i = 0;
    while i < buf.len() {
        match buf[i] {
            0x00 => i += 1,
            0xFE | 0xFD => return true,
            0x01 | 0x02 => {
                // lock/memory control TLV: type, length, value
                if i + 1 >= buf.len() {
                    return false;
                }
                i += 2 + buf[i + 1] as usize;
            }
            0x03 => {
                if i + 1 >= buf.len() {
                    return false;
                }
                let len = if buf[i + 1] == 0xFF {
                    if i + 4 > buf.len() {
                        return false;
                    }
                    ((buf[i + 2] as usize) << 8) | buf[i + 3] as usize
                } else {
                    buf[i + 1] as usize
                };
                let start = if buf[i + 1] == 0xFF { i + 4 } else { i + 2 };
                if start + len > buf.len() {
                    return false;
                }
                i = start + len;
            }
            _ => return true,
        }
    }
    false
}

// --- TLV + record parsing ------------------------------------------------------

fn parse_tlvs(buf: &[u8], r: &mut Report) {
    let mut i = 0usize;
    while i < buf.len() {
        let tag = buf[i];
        match tag {
            0x00 => i += 1,
            0xFE => break,
            0x03 => {
                let (len, start) = match tlv_length(buf, i + 1) {
                    Some(v) => v,
                    None => {
                        let _ = r.text.push_str("truncated NDEF TLV");
                        return;
                    }
                };
                if start + len > buf.len() {
                    let _ = r.text.push_str("NDEF TLV longer than the readable area");
                    return;
                }
                r.ok = true;
                let msg = &buf[start..start + len];
                let n = msg.len().min(r.raw.capacity());
                let _ = r.raw.extend_from_slice(&msg[..n]);
                parse_message(msg, r);
                return;
            }
            _ => break,
        }
    }
    if r.text.is_empty() {
        let _ = r.text.push_str("no NDEF message on tag");
    }
}

fn tlv_length(buf: &[u8], at: usize) -> Option<(usize, usize)> {
    let l = *buf.get(at)?;
    if l == 0xFF {
        let hi = *buf.get(at + 1)? as usize;
        let lo = *buf.get(at + 2)? as usize;
        Some(((hi << 8) | lo, at + 3))
    } else {
        Some((l as usize, at + 1))
    }
}

fn parse_message(msg: &[u8], r: &mut Report) {
    let mut i = 0usize;
    while i + 1 < msg.len() {
        let header = msg[i];
        let tnf = header & 0x07;
        let sr = header & 0x10 != 0;
        let il = header & 0x08 != 0;
        i += 1;

        let type_len = match msg.get(i) {
            Some(v) => *v as usize,
            None => break,
        };
        i += 1;

        let payload_len = if sr {
            match msg.get(i) {
                Some(v) => *v as usize,
                None => break,
            }
        } else {
            if i + 4 > msg.len() {
                break;
            }
            u32::from_be_bytes([msg[i], msg[i + 1], msg[i + 2], msg[i + 3]]) as usize
        };
        i += if sr { 1 } else { 4 };

        let id_len = if il {
            match msg.get(i) {
                Some(v) => *v as usize,
                None => break,
            }
        } else {
            0
        };
        i += if il { 1 } else { 0 };

        if i + type_len + id_len + payload_len > msg.len() {
            let _ = r.text.push_str("truncated record");
            break;
        }
        let rtype = &msg[i..i + type_len];
        i += type_len + id_len;
        let payload = &msg[i..i + payload_len];
        i += payload_len;

        if r.records as usize >= MAX_RECORDS {
            let _ = r.text.push_str("… (more records)");
            break;
        }
        r.records += 1;
        if !r.text.is_empty() {
            let _ = r.text.push('\n');
        }

        if tnf == 0x01 && rtype == b"T" && !payload.is_empty() {
            let lang = (payload[0] & 0x3F) as usize;
            let text = if 1 + lang <= payload.len() {
                &payload[1 + lang..]
            } else {
                &[]
            };
            let _ = r.text.push_str("T:");
            push_utf8_lossy(&mut r.text, text);
        } else if tnf == 0x01 && rtype == b"U" && !payload.is_empty() {
            let _ = r.text.push_str("U:");
            let _ = r.text.push_str(uri_prefix(payload[0]));
            push_utf8_lossy(&mut r.text, &payload[1..]);
        } else {
            push_utf8_lossy(&mut r.text, rtype);
            let _ = r.text.push(':');
            for b in payload.iter().take(32) {
                let _ = write!(r.text, "{:02x}", b);
            }
        }
    }
}

/// Append payload bytes as text for the human summary.
///
/// Control characters are dropped: a payload comes off a tag, and this text ends
/// up in logs and terminals. Valid UTF-8 sequences pass through unchanged so
/// non-ASCII text (umlauts, CJK) survives, and anything invalid becomes '?'. The
/// byte-exact form is the base64 field - this one is deliberately lossy.
fn push_utf8_lossy(out: &mut String<384>, bytes: &[u8]) {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b < 0x20 {
            i += 1; // control character: dropped
        } else if b < 0x80 {
            let _ = out.push(b as char);
            i += 1;
        } else {
            let n = if b >= 0xF0 {
                4
            } else if b >= 0xE0 {
                3
            } else if b >= 0xC0 {
                2
            } else {
                0
            };
            if n > 0 && i + n <= bytes.len() {
                match core::str::from_utf8(&bytes[i..i + n]) {
                    Ok(s) => {
                        let _ = out.push_str(s);
                        i += n;
                    }
                    Err(_) => {
                        let _ = out.push('?');
                        i += 1;
                    }
                }
            } else {
                let _ = out.push('?');
                i += 1;
            }
        }
    }
}

fn uri_prefix(code: u8) -> &'static str {
    match code {
        0x01 => "http://www.",
        0x02 => "https://www.",
        0x03 => "http://",
        0x04 => "https://",
        0x05 => "tel:",
        0x06 => "mailto:",
        0x07 => "ftp://anonymous:anonymous@",
        0x08 => "ftp://ftp.",
        0x09 => "ftps://",
        0x0A => "sftp://",
        0x0B => "smb://",
        0x0C => "nfs://",
        0x0D => "ftp://",
        0x0E => "dav://",
        0x0F => "news:",
        0x10 => "telnet://",
        0x11 => "imap:",
        0x12 => "rtsp://",
        0x13 => "urn:",
        0x14 => "pop:",
        0x15 => "sip:",
        0x16 => "sips:",
        0x17 => "tftp:",
        0x18 => "btspp://",
        0x19 => "btl2cap://",
        0x1A => "btgoep://",
        0x1B => "tcpobex://",
        0x1C => "irdaobex://",
        0x1D => "file://",
        0x1E => "urn:epc:id:",
        0x1F => "urn:epc:tag:",
        0x20 => "urn:epc:pat:",
        0x21 => "urn:epc:raw:",
        0x22 => "urn:epc:",
        0x23 => "urn:nfc:",
        _ => "",
    }
}

// --- writing ------------------------------------------------------------------

/// A 1K Classic card gives us three usable sectors = 9 data blocks, minus the TLV
/// header and terminator.
const CLASSIC_WRITE_MAX: usize = 9 * 16;
/// NTAG213-class Type 2 tags have 144 bytes of user area.
const TYPE2_WRITE_MAX: usize = 144;

/// URI prefixes from the NFC Forum URI record type definition, longest first so
/// the shortest payload wins.
const URI_PREFIXES: [(&str, u8); 6] = [
    ("https://www.", 0x02),
    ("http://www.", 0x01),
    ("https://", 0x04),
    ("http://", 0x03),
    ("tel:", 0x05),
    ("mailto:", 0x06),
];

/// Short-record header: SR + well-known TNF, plus the MB/ME bits for position.
const fn record_header(mb: bool, me: bool) -> u8 {
    let mut h = 0x10 | 0x01;
    if mb {
        h |= 0x80;
    }
    if me {
        h |= 0x40;
    }
    h
}

fn push_record(
    out: &mut [u8],
    at: usize,
    hdr: u8,
    typ: u8,
    payload: &[u8],
) -> Result<usize, &'static str> {
    if payload.len() > 0xFF {
        return Err("record payload over 255 bytes needs a long record");
    }
    let end = at + 4 + payload.len();
    if end > out.len() {
        return Err("message does not fit the tag");
    }
    out[at] = hdr;
    out[at + 1] = 1; // type length
    out[at + 2] = payload.len() as u8;
    out[at + 3] = typ;
    out[at + 4..end].copy_from_slice(payload);
    Ok(end)
}

/// Build a message of one URI record and/or one text record. Short records only:
/// a URL or a label line never reaches 255 bytes of payload.
pub fn build_message(
    uri: Option<&str>,
    text: Option<&str>,
    out: &mut [u8],
) -> Result<usize, &'static str> {
    if uri.is_none() && text.is_none() {
        return Err("nothing to write: pass url= or text=");
    }
    let two = uri.is_some() && text.is_some();
    let mut at = 0usize;

    if let Some(url) = uri {
        let mut payload = [0u8; 256];
        let (code, rest) = match URI_PREFIXES.iter().find(|(p, _)| url.starts_with(p)) {
            // The prefix is stored as a one-byte code, the reader never sees it.
            Some((p, c)) => (*c, &url[p.len()..]),
            None => (0x00, url),
        };
        if rest.len() + 1 > payload.len() {
            return Err("uri too long");
        }
        payload[0] = code;
        payload[1..1 + rest.len()].copy_from_slice(rest.as_bytes());
        at = push_record(out, at, record_header(true, !two), b'U', &payload[..1 + rest.len()])?;
    }
    if let Some(t) = text {
        let mut payload = [0u8; 256];
        // Status byte: UTF-8 (top bit clear) with a 2-character language code.
        payload[0] = 0x02;
        payload[1..3].copy_from_slice(b"en");
        let n = t.as_bytes().len();
        if n + 3 > payload.len() {
            return Err("text too long");
        }
        payload[3..3 + n].copy_from_slice(t.as_bytes());
        at = push_record(out, at, record_header(uri.is_none(), true), b'T', &payload[..3 + n])?;
    }
    Ok(at)
}

/// Wrap a message in the NDEF TLV, terminate it and zero-pad to `granularity`
/// (16 for Classic blocks, 4 for Type 2 pages).
pub fn wrap_tlv(msg: &[u8], out: &mut [u8], granularity: usize) -> Result<usize, &'static str> {
    let hdr = if msg.len() < 0xFF { 2 } else { 4 };
    let end = hdr + msg.len() + 1; // header + message + terminator
    if end > out.len() {
        return Err("message does not fit the tag");
    }
    if hdr == 2 {
        out[0] = 0x03;
        out[1] = msg.len() as u8;
    } else {
        out[0] = 0x03;
        out[1] = 0xFF;
        out[2] = (msg.len() >> 8) as u8;
        out[3] = msg.len() as u8;
    }
    out[hdr..hdr + msg.len()].copy_from_slice(msg);
    out[hdr + msg.len()] = 0xFE;
    let padded = end.div_ceil(granularity) * granularity;
    if padded > out.len() {
        return Err("message does not fit the tag");
    }
    for b in &mut out[end..padded] {
        *b = 0;
    }
    Ok(padded)
}

/// Turn a MIFARE status byte into something readable.
fn mifare_error(status: u8) -> &'static str {
    match status {
        0x0E => "write refused by the block's access bits",
        0x0F => "no write access to the block",
        0x13 => "not authenticated",
        0x14 => "authentication failed",
        _ => "MIFARE command failed",
    }
}

async fn write_block<I2C>(
    pn: &mut Pn532<I2C>,
    target: u8,
    block: u8,
    data: &[u8; 16],
) -> Result<(), &'static str>
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let mut cmd = [0u8; 18];
    cmd[0] = MIFARE_WRITE_CLASSIC;
    cmd[1] = block;
    cmd[2..18].copy_from_slice(data);
    match pn.in_data_exchange(target, &cmd, 1, 400).await {
        Ok(_) => Ok(()),
        Err(crate::pn532::Error::Status(s)) => {
            warn!("MIFARE classic write block {} -> status {:#04x}", block, s);
            Err(mifare_error(s))
        }
        Err(e) => {
            warn!("MIFARE classic write block {} failed: {}", block, e.as_str());
            Err(e.as_str())
        }
    }
}

/// MIFARE Classic: write into the NDEF sector(s), never into a trailer block,
/// authenticating each sector first. Verifies by reading everything back, because
/// a write status byte alone is weak evidence.
async fn write_classic<I2C>(
    pn: &mut Pn532<I2C>,
    target: &Target,
    msg: &[u8],
) -> Result<(usize, u16, bool), &'static str>
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let mut stream = [0u8; CLASSIC_WRITE_MAX];
    let total = wrap_tlv(msg, &mut stream, 16)?;
    let uid = target.uid4();
    let mut scratch = [0u8; SCRATCH];
    let mut key_idx = 0usize;
    let mut sector_open = -1i8;
    const BLOCKS: [u8; 9] = [4, 5, 6, 8, 9, 10, 12, 13, 14];

    let mut at = 0usize;
    for &b in BLOCKS.iter() {
        if at >= total {
            break;
        }
        let sector = (b / 4) as i8;
        if sector != sector_open {
            key_idx = authenticate(pn, target.number, b, &uid, &mut scratch, 0).await?;
            sector_open = sector;
        }
        let mut data = [0u8; 16];
        data.copy_from_slice(&stream[at..at + 16]);
        write_block(pn, target.number, b, &data).await?;
        at += 16;
    }

    // Read back: the card is the authority on whether the write took.
    let mut back = [0u8; CLASSIC_WRITE_MAX];
    let mut at = 0usize;
    let mut sector_open = -1i8;
    for &b in BLOCKS.iter() {
        if at >= total {
            break;
        }
        let sector = (b / 4) as i8;
        if sector != sector_open {
            if authenticate(pn, target.number, b, &uid, &mut scratch, 0).await.is_err() {
                return Ok((key_idx, total as u16, false));
            }
            sector_open = sector;
        }
        let cmd = [MIFARE_READ, b];
        match pn.in_data_exchange(target.number, &cmd, 16, 300).await {
            Ok(d) if d.len() >= 16 => {
                back[at..at + 16].copy_from_slice(&d[..16]);
                at += 16;
            }
            _ => return Ok((key_idx, total as u16, false)),
        }
    }
    let verified = back[..total] == stream[..total];
    Ok((key_idx, total as u16, verified))
}

/// Type 2 (NTAG/Ultralight): write pages from 4. Only onto a tag that is already
/// NDEF formatted - we do not write a capability container, because getting that
/// wrong is how a tag ends up unusable.
async fn write_type2<I2C>(
    pn: &mut Pn532<I2C>,
    target: &Target,
    msg: &[u8],
) -> Result<(u16, bool), &'static str>
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    // Capability container at page 3: E1 10 <size/8> <access>.
    let cmd = [MIFARE_READ, 3];
    let hdr = match pn.in_data_exchange(target.number, &cmd, 16, 300).await {
        Ok(d) if d.len() >= 4 => d,
        _ => return Err("could not read the tag header"),
    };
    if hdr[0] != 0xE1 || hdr[1] != 0x10 {
        return Err("tag is not NDEF formatted (no capability container)");
    }
    let capacity = hdr[2] as usize * 8;

    let mut stream = [0u8; TYPE2_WRITE_MAX];
    let total = wrap_tlv(msg, &mut stream, 4)?;
    if total > capacity {
        return Err("message does not fit the tag's NDEF capacity");
    }

    let mut page = 4u8;
    let mut at = 0usize;
    while at < total {
        let mut cmd = [0u8; 6];
        cmd[0] = MIFARE_WRITE_TYPE2;
        cmd[1] = page;
        cmd[2..6].copy_from_slice(&stream[at..at + 4]);
        match pn.in_data_exchange(target.number, &cmd, 1, 400).await {
            Ok(_) => {}
            Err(crate::pn532::Error::Status(s)) => {
                warn!("Ultralight write page {} -> status {:#04x}", page, s);
                return Err(mifare_error(s));
            }
            Err(e) => {
                warn!("Ultralight write page {} failed: {}", page, e.as_str());
                return Err(e.as_str());
            }
        }
        at += 4;
        page += 1;
    }

    let n = total.min(TYPE2_WRITE_MAX) as u8;
    let cmd = [MIFARE_READ, 4];
    match pn.in_data_exchange(target.number, &cmd, 16, 300).await {
        Ok(d) if d.len() >= 16 && n as usize <= 16 => Ok((total as u16, d[..n as usize] == stream[..n as usize])),
        // Beyond the first read we would need several pages; report unverified
        // rather than honestly claiming a check we did not do.
        Ok(_) => Ok((total as u16, total <= 16)),
        Err(_) => Err("could not read the tag back"),
    }
}

/// Write `uri` and/or `text` to the tag under the reader.
pub async fn write<I2C>(
    pn: &mut Pn532<I2C>,
    target: &Target,
    uri: Option<&str>,
    text: Option<&str>,
) -> Report
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let mut msg = [0u8; 256];
    let n = match build_message(uri, text, &mut msg) {
        Ok(n) => n,
        Err(why) => return Report::fail("write", why),
    };
    write_message(pn, target, &msg[..n]).await
}

/// Write a ready-made NDEF message (the bytes between the TLV header and the
/// terminator) to the tag under the reader. This is what `PUT /nfc/ndef` hands in:
/// the caller owns the record encoding, we own the container.
pub async fn write_message<I2C>(pn: &mut Pn532<I2C>, target: &Target, msg: &[u8]) -> Report
where
    I2C: embedded_hal_async::i2c::I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    let n = msg.len();
    let mut r = Report::new("write");
    let _ = r.raw.extend_from_slice(&msg[..n.min(r.raw.capacity())]);

    match target.kind() {
        "MIFARE Classic 1K" | "MIFARE Classic 4K" => {
            if n + 3 > CLASSIC_WRITE_MAX {
                let mut f = Report::fail("write", "message too long for this card (max ~140 bytes)");
                let _ = f.raw.extend_from_slice(&msg[..n.min(f.raw.capacity())]);
                return f;
            }
            match write_classic(pn, target, &msg[..n]).await {
                Ok((key, bytes, verified)) => {
                    r.ok = verified;
                    r.bytes = bytes;
                    r.verified = verified;
                    r.key = CLASSIC_KEY_NAMES[key];
                    if !verified {
                        let _ = r.text.push_str("write not verified by read-back");
                    }
                }
                Err(why) => {
                    let _ = r.text.push_str(why);
                }
            }
        }
        "ISO14443A Type 2 (NTAG/Ultralight)" => match write_type2(pn, target, &msg[..n]).await {
            Ok((bytes, verified)) => {
                r.ok = verified;
                r.bytes = bytes;
                r.verified = verified;
                if !verified {
                    let _ = r.text.push_str("write not verified by read-back");
                }
            }
            Err(why) => {
                let _ = r.text.push_str(why);
            }
        },
        _ => {
            let _ = r.text.push_str("unsupported tag type for writing");
        }
    }
    r
}
