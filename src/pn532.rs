#![allow(
    dead_code,
    reason = "the driver exposes the full command set (RF configuration, register
              pokes, PowerDown, InCommunicateThru) on purpose: they are reachable
              through the raw endpoint and are what RF-off tuning needs"
)]

//! Minimal async PN532 driver, ISO14443A over I2C, written for embassy.
//!
//! Why not the `pn532` crate: its `Interface` trait is synchronous and
//! `process()` spins on the I2C status byte internally, so every command blocks
//! the thread-mode executor (which is also running embassy-net). This driver
//! polls the same status byte but `await`s between attempts, so the executor
//! stays responsive; it also exposes the RF-field and PowerDown controls we
//! need for `/nfc/rf`.
//!
//! Frame format (PN532 User Manual §6.2.1; cross-checked against the crate):
//! ```text
//! host -> 00 00 FF LEN LCS D4 CMD DATA... DCS 00     LEN = 2 + data.len()
//! pn532-> 00 00 FF LEN LCS D5 CMD DATA... DCS 00     prefixed by a status byte (0x01 = ready)
//! ACK   = 00 00 FF 00 FF 00
//! LCS = (!LEN)+1 ; DCS = (!(D4|D5 + CMD + DATA sum))+1   (LEN+LCS == 0 mod 256)
//! ```
//! I2C reads return the status byte first, so every read is
//! `[read 1 byte status][read frame]` in one transaction.

use core::fmt;

use embassy_time::{Duration, Instant, Timer, with_timeout};

/// Byte dump for frame-level logging.
pub struct HexBytes<'a>(pub &'a [u8]);

impl fmt::Display for HexBytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}
use embedded_hal::i2c::{Error as _, ErrorKind};
use embedded_hal_async::i2c::{I2c, Operation};
use log::warn;

/// 7-bit I2C address of the PN532 with the DIP switches set to I2C.
/// The board's `SEL0 ON / SEL1 OFF` position selects this.
pub const I2C_ADDRESS: u8 = 0x24;

const PREAMBLE: [u8; 3] = [0x00, 0x00, 0xFF];
const POSTAMBLE: u8 = 0x00;
const TFI_HOST_TO_PN532: u8 = 0xD4;
const TFI_PN532_TO_HOST: u8 = 0xD5;
pub const ACK_FRAME: [u8; 6] = [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];
/// The PN532 signals "I have data for you" with this status byte.
const I2C_READY: u8 = 0x01;

/// Sent to make the chip re-present the response from the start (see the read
/// path in `try_command`) and, per ESPHome, the way to abort a still-running
/// InListPassiveTarget.
const NACK_FRAME: [u8; 6] = [0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00];

/// Largest response payload we hand to a caller (NDEF reads need 16, raw
/// commands a bit more).
/// Generous on purpose: a 10-byte transaction at 100 kHz is ~1 ms, so this only
/// fires when the bus is actually stuck. Too tight a value turns Wi-Fi init
/// jitter into phantom "bus errors".
const BUS_TIMEOUT_MS: u64 = 200;

const MAX_DATA: usize = 64;
const SCRATCH: usize = 9 + MAX_DATA;

// --- commands -----------------------------------------------------------------
pub const CMD_GET_FIRMWARE_VERSION: u8 = 0x02;
pub const CMD_GET_GENERAL_STATUS: u8 = 0x04;
pub const CMD_WRITE_REGISTER: u8 = 0x08;
pub const CMD_READ_REGISTER: u8 = 0x06;
pub const CMD_SAM_CONFIGURATION: u8 = 0x14;
pub const CMD_POWER_DOWN: u8 = 0x16;
pub const CMD_RF_CONFIGURATION: u8 = 0x32;
pub const CMD_IN_LIST_PASSIVE_TARGET: u8 = 0x4A;
pub const CMD_IN_DATA_EXCHANGE: u8 = 0x40;
pub const CMD_IN_RELEASE: u8 = 0x52;
pub const CMD_IN_COMMUNICATE_THRU: u8 = 0x42;

/// MIFARE / Type-2 tag commands, sent inside `InDataExchange`.
pub const MIFARE_AUTH_KEY_A: u8 = 0x60;
pub const MIFARE_AUTH_KEY_B: u8 = 0x61;
pub const MIFARE_READ: u8 = 0x30;
/// MIFARE Classic WRITE is 0xA0; 0xA2 is the Ultralight/Type 2 WRITE. Sending
/// the wrong one gets no answer from the tag at all, which reads like a bus or
/// auth problem rather than a wrong opcode.
pub const MIFARE_WRITE_CLASSIC: u8 = 0xA0;
pub const MIFARE_WRITE_TYPE2: u8 = 0xA2;

/// Which wait a timeout came from: the ACK of our command, or its response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ack,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Nothing acknowledged the I2C address: no reader on the bus.
    NoDevice,
    /// The PN532 never acknowledged the command inside the timeout.
    TimeoutAck,
    /// The command was acknowledged but the response never arrived. With the
    /// default MxRtyPassiveActivation this is what an empty RF field looks like:
    /// InListPassiveTarget keeps polling and never answers.
    TimeoutResponse,
    /// The ACK frame was missing or malformed.
    BadAck,
    /// The response frame failed its header/checksum checks.
    BadFrame,
    /// The PN532 reported a command-level error (see `ErrorCode` in its manual).
    Status(u8),
    /// The response did not fit `MAX_DATA`.
    TooLong,
    /// Bus error (arbitration lost, bus stuck, ...).
    Bus,
}

impl Error {
    pub fn as_str(&self) -> &'static str {
        match self {
            Error::NoDevice => "no PN532 on the I2C bus (nothing answered at 0x24)",
            Error::TimeoutAck => "PN532 never acknowledged the command",
            Error::TimeoutResponse => {
                "PN532 acknowledged but sent no response (empty RF field / no target)"
            }
            Error::BadAck => "missing/malformed ACK frame",
            Error::BadFrame => "malformed response frame (see the byte dump above)",
            Error::Status(_) => "PN532 reported a command error",
            Error::TooLong => "response too long",
            Error::Bus => "I2C bus error (no status byte; check SDA/SCL/pull-ups)",
        }
    }
}

/// Answer to `GetGeneralStatus`.
#[derive(Debug, Clone, Copy, Default)]
pub struct GeneralStatus {
    /// Last error the PN532 recorded.
    pub error: u8,
    /// The PN532 is driving the antenna.
    pub field: bool,
    /// Number of targets currently active.
    pub targets: u8,
}

/// A target found in the RF field.
#[derive(Debug, Clone, Copy, Default)]
pub struct Target {
    /// Logical target number, needed for `InDataExchange`/`InRelease`.
    pub number: u8,
    /// ATQA as sent by the card.
    pub atqa: [u8; 2],
    /// SAK: 0x00 = Type 2 (NTAG/Ultralight), 0x08/0x18 = MIFARE Classic.
    pub sak: u8,
    pub uid_len: u8,
    pub uid: [u8; 10],
}

impl Target {
    pub const EMPTY: Self = Self {
        number: 0,
        atqa: [0; 2],
        sak: 0,
        uid_len: 0,
        uid: [0; 10],
    };

    /// The UID as the reader's callers want it: `04a2b3c4`.
    pub fn uid_hex(&self, out: &mut heapless::String<32>) {
        let n = (self.uid_len as usize).min(10);
        for (i, b) in self.uid[..n].iter().enumerate() {
            if i > 0 {
                let _ = out.push('-');
            }
            let _ = core::fmt::write(out, format_args!("{:02X}", b));
        }
    }

    /// ESPuino's decimal form: each UID byte as three decimal digits, which is
    /// the 12-digit `serial_number` the print shop stores (`004162179196`).
    pub fn uid_decimal(&self, out: &mut heapless::String<32>) {
        let n = (self.uid_len as usize).min(10);
        for b in &self.uid[..n] {
            let _ = core::fmt::write(out, format_args!("{:03}", b));
        }
    }

    /// What the tag is, from its SAK.
    pub fn kind(&self) -> &'static str {
        match self.sak {
            0x00 => "ISO14443A Type 2 (NTAG/Ultralight)",
            0x08 | 0x18 => "MIFARE Classic 1K",
            0x09 => "MIFARE Mini",
            0x10 | 0x11 => "MIFARE Plus",
            0x20 => "ISO14443A Type 4 / DESFire",
            _ => "ISO14443A (unknown SAK)",
        }
    }

    /// Up to 4 bytes for the MIFARE auth command (only the first four count).
    pub fn uid4(&self) -> [u8; 4] {
        let mut out = [0u8; 4];
        out.copy_from_slice(&self.uid[..4]);
        out
    }
}

/// Bounded bus access. A stuck bus (module unpowered, SDA held low) must surface
/// as a failed operation the reader task can retry, never as an await that never
/// returns - a wedged reader task would answer the API with stale state forever.
async fn bus_write<I2C: I2c>(i2c: &mut I2C, bytes: &[u8]) -> Result<(), Error> {
    match with_timeout(Duration::from_millis(BUS_TIMEOUT_MS), i2c.write(I2C_ADDRESS, bytes)).await {
        Ok(Ok(())) => Ok(()),
        _ => Err(Error::Bus),
    }
}

async fn bus_transaction<I2C: I2c>(
    i2c: &mut I2C,
    ops: &mut [Operation<'_>],
) -> Result<(), Error> {
    match with_timeout(
        Duration::from_millis(BUS_TIMEOUT_MS),
        i2c.transaction(I2C_ADDRESS, ops),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        _ => Err(Error::Bus),
    }
}

pub struct Pn532<I2C> {
    i2c: I2C,
    buf: [u8; SCRATCH],
}

impl<I2C> Pn532<I2C>
where
    I2C: I2c,
    I2C::Error: embedded_hal::i2c::Error,
{
    pub fn new(i2c: I2C) -> Self {
        Self {
            i2c,
            buf: [0; SCRATCH],
        }
    }

    pub fn release(self) -> I2C {
        self.i2c
    }

    // --- low level -------------------------------------------------------------

    /// One attempt at the PN532's status byte. `Ok(false)` means "not ready
    /// yet" (the chip NAKs its address until it has something to say), which is
    /// the documented behaviour, not an error.
    async fn ready_once(&mut self) -> Result<bool, Error> {
        let mut status = [0u8; 1];
        let read = with_timeout(
            Duration::from_millis(BUS_TIMEOUT_MS),
            self.i2c.read(I2C_ADDRESS, &mut status),
        )
        .await;
        match read {
            Ok(Ok(())) => Ok(status[0] == I2C_READY),
            Ok(Err(e)) => match e.kind() {
                ErrorKind::NoAcknowledge(_) => Ok(false),
                _ => Err(Error::Bus),
            },
            Err(_timeout) => Err(Error::Bus),
        }
    }

    async fn wait_ready(&mut self, timeout_ms: u64, phase: Phase) -> Result<(), Error> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut nak = false;
        loop {
            match self.ready_once().await {
                Ok(true) => return Ok(()),
                // NAK: the chip is there and busy, keep waiting.
                Ok(false) => nak = true,
                // no status byte at all: keep trying, but remember it
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                return Err(if nak {
                    match phase {
                        Phase::Ack => Error::TimeoutAck,
                        Phase::Response => Error::TimeoutResponse,
                    }
                } else {
                    Error::NoDevice
                });
            }
            Timer::after(Duration::from_millis(2)).await;
        }
    }

    /// Is anything reacting at 0x24? Bring-up diagnostic: separates "no reader
    /// wired" from "reader present but not answering".
    ///
    /// `ready_once` returns `Ok(false)` when the chip NAKs its address, which
    /// the PN532 does while it has nothing to say - so a NAK still means there
    /// is a slave on the bus. Only a bus-level failure (no status byte at all,
    /// which is how a missing device reads on this controller) counts as
    /// absent. The distinction matters here: a bare ESP32 with nothing on the
    /// I2C pins times out rather than NAKing.
    pub async fn probe(&mut self) -> bool {
        !matches!(self.ready_once().await, Err(Error::Bus))
    }

    async fn write_frame(&mut self, command: u8, data: &[u8]) -> Result<(), Error> {
        let len = 2 + data.len();
        if len > 0xFF || 9 + data.len() > SCRATCH {
            return Err(Error::TooLong);
        }

        let mut sum = TFI_HOST_TO_PN532.wrapping_add(command);
        for &b in data {
            sum = sum.wrapping_add(b);
        }

        let buf = &mut self.buf;
        buf[0] = PREAMBLE[0];
        buf[1] = PREAMBLE[1];
        buf[2] = PREAMBLE[2];
        buf[3] = len as u8;
        buf[4] = (!(len as u8)).wrapping_add(1);
        buf[5] = TFI_HOST_TO_PN532;
        buf[6] = command;
        buf[7..7 + data.len()].copy_from_slice(data);
        buf[7 + data.len()] = (!sum).wrapping_add(1);
        buf[8 + data.len()] = POSTAMBLE;

        let n = 9 + data.len();
        bus_write(&mut self.i2c, &self.buf[..n]).await
    }

    async fn read_ack(&mut self) -> Result<(), Error> {
        // An I2C read returns the status byte FIRST and the frame after it, so
        // every frame read is a two-part transaction with a repeated start.
        let mut status = [0u8; 1];
        let mut ack = [0u8; 6];
        let mut ops = [Operation::Read(&mut status), Operation::Read(&mut ack)];
        bus_transaction(&mut self.i2c, &mut ops)
            .await
            .map_err(|_| Error::BadAck)?;
        if ack == ACK_FRAME {
            Ok(())
        } else {
            warn!(
                "PN532 bad ACK: status={:02x} expected={} got={}",
                status[0],
                HexBytes(&ACK_FRAME),
                HexBytes(&ack)
            );
            Err(Error::BadAck)
        }
    }

    /// Send a command and return the response payload (the bytes after `D5 CMD`).
    pub async fn command<'a>(
        &'a mut self,
        command: u8,
        data: &[u8],
        max_data: usize,
        timeout_ms: u64,
    ) -> Result<&'a [u8], Error> {
        if max_data > MAX_DATA {
            return Err(Error::TooLong);
        }

        // The PN532 occasionally rejects a frame (bus noise, mid-command
        // state); one retry of the whole exchange is cheap and fixes it.
        let mut attempt = 0;
        let data_len = loop {
            attempt += 1;
            match self.try_command(command, data, max_data, timeout_ms).await {
                Ok(len) => break len,
                // A BadAck or a garbled frame is usually the chip caught
                // mid-update on the I2C read; re-sending the command makes it
                // produce a fresh response. Anything else (no device, real
                // timeout, ...) is reported straight away.
                Err(Error::BadAck | Error::BadFrame) if attempt < 3 => {
                    Timer::after(Duration::from_millis(5)).await;
                    continue;
                }
                Err(other) => return Err(other),
            }
        };
        Ok(&self.buf[7..7 + data_len])
    }

    /// Send a command and leave the response data in `self.buf[7..]`,
    /// returning its length.
    async fn try_command(
        &mut self,
        command: u8,
        data: &[u8],
        max_data: usize,
        timeout_ms: u64,
    ) -> Result<usize, Error> {
        // `max_data` is the caller's budget, but the frame is always read out in
        // full at the length the chip announces: stopping short desynchronises the
        // chip's I2C buffer. The payload is what it is; callers slice what they
        // need.
        let _ = max_data;

        self.write_frame(command, data).await?;

        self.wait_ready(100, Phase::Ack).await?;
        self.read_ack().await?;

        // ACK seen; now wait for the response frame (an RF round can take tens
        // of milliseconds).
        self.wait_ready(timeout_ms, Phase::Response).await?;

        // Read exactly the frame the chip announced. Reading more bytes than the
        // frame contains desynchronises the PN532's I2C buffer, and then the
        // *next* command reads garbage (seen as "malformed response frame" on
        // whichever command followed an over-read).
        //
        // Reference sequence (ESPHome pn532_i2c): peek the header, NACK so the
        // chip re-presents the response from the start, then read the whole
        // frame at its exact length.
        let mut head = [0u8; 7];
        {
            let mut status = [0u8; 1];
            let mut ops = [Operation::Read(&mut status), Operation::Read(&mut head)];
            bus_transaction(&mut self.i2c, &mut ops).await?;
            if status[0] != I2C_READY {
                return Err(Error::BadFrame);
            }
        }
        if head[0..3] != PREAMBLE
            || head[3].wrapping_add(head[4]) != 0
            || head[5] != TFI_PN532_TO_HOST
        {
            warn!("PN532 header rejected: raw={}", HexBytes(&head));
            return Err(Error::BadFrame);
        }
        let len = head[3] as usize; // TFI + command + data
        if len < 2 {
            return Err(Error::BadFrame);
        }

        bus_write(&mut self.i2c, &NACK_FRAME).await?;
        if self.wait_ready(200, Phase::Response).await.is_err() {
            return Err(Error::BadFrame);
        }

        let frame_len = len + 7; // preamble .. postamble
        let n = frame_len.min(SCRATCH);
        {
            let mut status = [0u8; 1];
            let buf = &mut self.buf[..n];
            buf.fill(0);
            let mut ops = [Operation::Read(&mut status), Operation::Read(buf)];
            bus_transaction(&mut self.i2c, &mut ops).await?;
            if status[0] != I2C_READY {
                return Err(Error::BadFrame);
            }
        }
        // Did not fit the scratch buffer: drain the rest so the chip's stream
        // stays in sync for the next command.
        let mut rest = frame_len - n;
        while rest > 0 {
            let chunk = rest.min(SCRATCH);
            let mut status = [0u8; 1];
            let mut skip = [0u8; SCRATCH];
            let mut ops = [
                Operation::Read(&mut status),
                Operation::Read(&mut skip[..chunk]),
            ];
            if bus_transaction(&mut self.i2c, &mut ops).await.is_err() {
                break;
            }
            rest -= chunk;
        }
        if frame_len > n {
            return Err(Error::TooLong);
        }

        // buf is the frame itself: 00 00 FF LEN LCS D5 CMD DATA DCS 00
        let f = &self.buf[..n];
        if f[0..3] != PREAMBLE || f[5] != TFI_PN532_TO_HOST {
            warn!("PN532 frame rejected (preamble/TFI): raw={}", HexBytes(f));
            return Err(Error::BadFrame);
        }
        if f[4] != (!(f[3])).wrapping_add(1) || f[len + 6] != POSTAMBLE {
            warn!("PN532 frame rejected (LCS/postamble): raw={}", HexBytes(f));
            return Err(Error::BadFrame);
        }
        // The PN532 answers with the command byte INCREMENTED: a GetFirmwareVersion
        // request (0x02) comes back as 0x03.
        if f[6] != command.wrapping_add(1) {
            warn!(
                "PN532 command echo {:#04x}, expected {:#04x}: raw={}",
                f[6],
                command.wrapping_add(1),
                HexBytes(f)
            );
            return Err(Error::BadFrame);
        }
        // DCS covers TFI + command + data (LEN + LCS cancel out mod 256).
        let mut sum = 0u8;
        for &b in &f[5..5 + len] {
            sum = sum.wrapping_add(b);
        }
        if f[len + 5] != (!sum).wrapping_add(1) {
            warn!("PN532 frame rejected (DCS): raw={}", HexBytes(f));
            return Err(Error::BadFrame);
        }

        Ok(len - 2)
    }

    // --- PN532 commands --------------------------------------------------------

    /// Returns (IC, Version, Revision, Support).
    pub async fn firmware_version(&mut self) -> Result<[u8; 4], Error> {
        let r = self.command(CMD_GET_FIRMWARE_VERSION, &[], 5, 100).await?;
        if r.len() < 4 {
            return Err(Error::BadFrame);
        }
        Ok([r[0], r[1], r[2], r[3]])
    }

    /// Normal SAM mode: no SAM attached, the PN532 is a plain reader.
    /// Must be done once after power-up, otherwise all RF commands fail.
    pub async fn sam_configuration(&mut self) -> Result<(), Error> {
        // mode 0x01 = normal, timeout 0x14 = 20 * 50ms = 1s, no IRQ pin
        self.command(CMD_SAM_CONFIGURATION, &[0x01, 0x14, 0x00], 1, 100)
            .await?;
        Ok(())
    }

    /// Abort a command the chip is still executing by writing the ACK frame.
    /// This is ESPHome's trick for a long-running `InListPassiveTarget`; the chip
    /// does not answer it.
    pub async fn abort(&mut self) {
        let _ = bus_write(&mut self.i2c, &ACK_FRAME).await;
        Timer::after(Duration::from_millis(2)).await;
    }

    /// Poll for one ISO14443A target. `Ok(None)` = field empty.
    ///
    /// `MxRtyPassiveActivation` defaults to 0xFF (retry forever), so with an empty
    /// field the chip sends no response at all and the response wait times out -
    /// that timeout *is* "no target", not a failure. The poll is then aborted so
    /// nothing is left running mid-flight, which is what desynchronises the chip.
    pub async fn list_passive_target(&mut self, timeout_ms: u64) -> Result<Option<Target>, Error> {
        // (MaxTg, BrTy). BrTy 0x00 = 106 kbps type A - NOT 0x01, which is
        // FeliCa 212 kbps, so passing 0x01 makes the chip search for a card that
        // will never answer and (with the default MxRtyPassiveActivation = 0xFF)
        // never return a response at all.
        let r = match self
            .command(
                CMD_IN_LIST_PASSIVE_TARGET,
                &[0x01, 0x00],
                16,
                timeout_ms,
            )
            .await
        {
            Ok(r) => r,
            Err(Error::TimeoutResponse) => {
                self.abort().await;
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        if r.is_empty() {
            return Err(Error::BadFrame);
        }
        let ntg = r[0];
        if ntg == 0 {
            return Ok(None);
        }
        // Tg, SENS_RES(2), SEL_RES, NfcIdLength, NfcId
        if r.len() < 6 {
            return Err(Error::BadFrame);
        }
        let uid_len = r[5].min(10) as usize;
        if r.len() < 6 + uid_len {
            return Err(Error::BadFrame);
        }
        let mut t = Target {
            number: r[1],
            atqa: [r[2], r[3]],
            sak: r[4],
            uid_len: uid_len as u8,
            uid: [0; 10],
        };
        t.uid[..uid_len].copy_from_slice(&r[6..6 + uid_len]);
        Ok(Some(t))
    }

    /// Send a tag-level command (MIFARE / Type 2) and return the tag's answer
    /// including its leading status byte.
    pub async fn in_data_exchange<'a>(
        &'a mut self,
        target: u8,
        data: &[u8],
        max_data: usize,
        timeout_ms: u64,
    ) -> Result<&'a [u8], Error> {
        let mut args = heapless::Vec::<u8, 32>::new();
        args.push(target).map_err(|_| Error::TooLong)?;
        args.extend_from_slice(data).map_err(|_| Error::TooLong)?;
        let r = self
            .command(CMD_IN_DATA_EXCHANGE, &args, max_data + 1, timeout_ms)
            .await?;
        if r.is_empty() {
            return Err(Error::BadFrame);
        }
        if r[0] != 0x00 {
            // 0x01 = timeout waiting for the tag, 0x14 = MIFARE auth failure, ...
            return Err(Error::Status(r[0]));
        }
        Ok(&r[1..])
    }

    pub async fn in_release(&mut self, target: u8) -> Result<(), Error> {
        self.command(CMD_IN_RELEASE, &[target], 1, 100).await?;
        Ok(())
    }

    pub async fn write_register(&mut self, register: u16, value: u8) -> Result<(), Error> {
        let args = [
            (register >> 8) as u8,
            (register & 0xFF) as u8,
            value,
        ];
        self.command(CMD_WRITE_REGISTER, &args, 1, 100).await?;
        Ok(())
    }

    pub async fn rf_configuration(&mut self, item: u8, params: &[u8]) -> Result<(), Error> {
        let mut args = heapless::Vec::<u8, 16>::new();
        args.push(item).map_err(|_| Error::TooLong)?;
        args.extend_from_slice(params).map_err(|_| Error::TooLong)?;
        self.command(CMD_RF_CONFIGURATION, &args, 1, 100).await?;
        Ok(())
    }

    /// Power down the whole front end (field off). With `wakeup_enable` the
    /// chip wakes on the next I2C access and comes back in a "reset" state, so
    /// `start()` has to run again (this is the deepest "RF off").
    pub async fn power_down(&mut self, wakeup_enable: bool) -> Result<(), Error> {
        let arg = if wakeup_enable { 0x80 } else { 0x00 };
        self.command(CMD_POWER_DOWN, &[arg], 1, 100).await?;
        Ok(())
    }

    /// Raw escape hatch for the `raw-api` feature and for bring-up.
    pub async fn raw<'a>(
        &'a mut self,
        command: u8,
        data: &[u8],
        max_data: usize,
        timeout_ms: u64,
    ) -> Result<&'a [u8], Error> {
        self.command(command, data, max_data, timeout_ms).await
    }
}
