//! Board-specific wiring: which pins the PN532 sits on, whether there is an
//! addressable LED, and how the firmware logs.
//!
//! Selected by the `board-*` cargo features (see Cargo.toml), so the same
//! firmware source runs on the printer's AtomS3 Lite and on a plain ESP32
//! DevKit used as a PN532 testbed.

/// Human readable board name, shown in the API and the boot log.
#[cfg(feature = "board-atoms3lite")]
pub const BOARD: &str = "M5Stack AtomS3 Lite (ESP32-S3)";
#[cfg(feature = "board-esp32-devkit")]
pub const BOARD: &str = "ESP32 DevKit (WROOM-32, testbed)";

/// I2C clock. The PN532 is specified up to 400 kHz, but its I2C timing is
/// sloppy; 100 kHz is the setting that works on every breakout we have seen.
/// Raise it only if the poll rate matters more than reliability.
pub const I2C_FREQ_KHZ: u32 = 100;

/// How often the reader polls for a card when idle, in milliseconds. This is
/// the presence-detection latency (plus the RF round trip).
pub const POLL_INTERVAL_MS: u64 = 60;

/// Consecutive empty polls before a card counts as removed. One miss is not a
/// removal: a card on the edge of the field flickers.
pub const PRESENCE_MISSES: u8 = 2;

/// The board's own LED. The AtomS3 Lite has a single WS2812C-2020; a WROOM
/// devkit has nothing we can drive from here (its blue LED is on a different
/// pin and not addressable), so there the LED state is only logged.
#[cfg(feature = "board-atoms3lite")]
pub const HAS_WS2812: bool = true;
#[cfg(feature = "board-esp32-devkit")]
pub const HAS_WS2812: bool = false;

