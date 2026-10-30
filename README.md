# nfc-reader

PN532 card reader for the Evolis Primacy's encoder slot: reads the UID of the card
at the antenna and serves it over HTTP. Rust + embassy + esp-hal, no Espressif
framework. Replaces the ESPHome reader in `~/git/ESPuino-label-printer/reader/`
(kept out of tree until this one is proven on hardware).

## Hardware

| | AtomS3 Lite (target) | ESP32 DevKit (bench) |
|---|---|---|
| chip | ESP32-S3FN8 | WROOM-32, rev3+ (**rev1 will not boot esp-hal**) |
| I2C SDA | Grove yellow = G2 | G21 |
| I2C SCL | Grove white = G1 | G22 |
| PN532 VCC | Grove red = 5V | 5V pin |
| LED | WS2812C on G35 | none (state only in `/nfc/status` and the log) |
| logging | native USB-Serial-JTAG (`/dev/ttyACM*`) | UART bridge (`/dev/ttyUSB*`, 115200) |

PN532: DIP switches **SEL0 ON / SEL1 OFF** = I2C, address 0x24. Power it from 5 V
for full field range. Only four wires reach it (GND, 5V, SDA, SCL), so there is **no
IRQ and no reset line** - the driver polls the status byte, and that determines the
whole design of `src/pn532.rs`.

## Build and flash

```sh
nix develop            # rustup + espflash + gcc, sources ~/export-esp.sh
cp .env.example .env   # WIFI_SSID / WIFI_PASSWORD / MDNS_HOSTNAME
cargo run --release                                            # AtomS3 Lite (default)
CARGO_BUILD_TARGET=xtensa-esp32-none-elf \
  cargo run --release --no-default-features --features board-esp32-devkit
```

`--no-default-features` is required for the second one: the board features are chip
selections and enabling both chips fails in `esp-metadata-generated` with duplicate
definitions. Add `--features raw-api` for the raw command endpoint.

Without credentials the firmware still runs and logs (reader only, no network), so
the PN532 can be brought up before touching Wi-Fi.

## API

No auth. Control endpoints take query parameters, which keeps every client to a
single `curl`:

| request | returns |
|---|---|
| `GET /nfc/status` | everything: reader state + firmware, `i2c_ack`, rf on/off, led state, polls/errors, last poll duration, the card (only while it is there), the last NDEF read |
| `GET /nfc/uid` | `{"present":true,"uid":"04-A2-B3-C4","uid_decimal":"004162179196"}`, or `404 {"present":false}` |
| `GET /nfc/uid?wait_ms=3000` | same, but holds the request open until a card appears (<= 30 s) |
| `GET /nfc/card` | UID plus `kind`, `sak`, `atqa`, `last_seen_ms` |
| `GET /nfc/scan?wait_ms=` | force a poll and report what is under the reader |
| `GET /nfc/ndef` | reads the NDEF message off the card: `{"ok":true,"records":1,"tag":"…","uid":"…","key":"nfc-forum","text":"U:https://example.com","msg_len":16,"b64":"0QEMVQRleGFtcGxlLmNvbQ=="}` |
| `PUT /nfc/ndef` | writes a whole NDEF message: the body **is** the message (the records, which is what an NDEF library produces); the container is ours. Raw bytes, so binary and long payloads need no escaping |
| `GET /nfc/rf?on=false` | stop polling and `InRelease` the card; `on=true` resumes |
| `GET /nfc/rf?on=false&deep=true` | additionally `PowerDown`: field truly off, chip asleep (the next access wakes it, so init is suppressed until RF is switched back on) |
| `GET /nfc/reset` | pulse the PN532's RSTPDN and re-initialize; reports whether the chip came back |
| `GET /nfc/write?url=…` | writes a URI record to the card **under the reader**; `?text=…` writes a text record, both together write two records |
| `GET /nfc/raw?cmd=02&args=` | (`raw-api` feature) raw PN532 command, response as hex |

Both write forms return `{"ok":true,"bytes":32,"key":"nfc-forum","verified":true}` -
`verified` means the tag was read back and compared. They write whatever card is on
the coil (404 when there is none); there is no arming, so a card that arrives later
is not touched. Bodies over 256 bytes are rejected, not truncated.

In the read response, `b64` is the byte-exact NDEF message (capped at 120 bytes,
`msg_len` reports the true length) and `text` is a human summary: records one per
line, control characters dropped so a hostile payload cannot inject escapes into
your terminal, valid UTF-8 passed through. Use `b64` when the bytes matter.

`PUT` for a whole message, `GET /nfc/write` for a curl one-liner (percent-encode
the values, an unencoded `&`/`=` in a URL would end the query field):

```sh
curl 'http://nfc-reader.local/nfc/write?url=https%3A%2F%2Fexample.org%2Fhello'
```

Reachable at `http://<MDNS_HOSTNAME>.local/nfc/status` (A record + `_http._tcp`
DNS-SD, TXT `path=/nfc/status`); the IP is in the boot log and `/nfc/status`.

Two UID spellings, both in every card response: `uid` is the dashed hex form the
existing reader documents (`04-A2-B3-C4`), `uid_decimal` is the 12-digit decimal the
print shop stores as `serial_number` (each UID byte as `%03d`, `004162179196`).

## LED

One pixel, so precedence is fixed: fault blinks red, a REST-triggered exchange
blinks amber over everything, then green while a card is present, dim white while
RF is on and nothing is there, off when RF is off. `/nfc/status` reports the state
(`red-blink`, `amber-blink`, `green`, `dim-white`, `off`), which is the only way to
see it on a board without an addressable LED.

## The reset line (RSTPDN), and why you want it

Wire the PN532's **RSTPDN** to a spare GPIO and set it in `.env`:

```
PN532_RESET_PIN=25
```

DevKit: 25/26/27 are free. AtomS3 Lite spare header: 5/6/7/8/38/39. Never a pin the
firmware already uses (I2C, LED), a flash pin (6-11), or a strapping pin
(0/2/12/15). Wire **RSTPDN**, not RSTO - RSTO is an output on the module.

With it wired the firmware drives it itself:

- a 100 ms pulse **at boot**, so every start begins from a clean chip;
- a pulse after **3 failed init attempts in a row**, which is the real recovery: a
  PN532 caught mid-transaction by an ESP32 reset holds a bus line low and no amount
  of retrying clears it, only a power cycle did;
- on demand via `GET /nfc/reset`.

Without it the firmware still retries and reports honestly, but that stuck state
needs someone to power-cycle the module. Note RSTPDN low *is* power-down: the pulse
always releases it, so a stuck-low pin (or a wrong pin number) means a dead reader.

With the reset line wired, flashing needs no preparation. Without it, park the reader
before flashing - it polls every 150 ms, so at reset time the chip is almost always
mid-transaction:

```sh
curl -s 'http://<ip>/nfc/rf?on=false&deep=true'   # field off, chip asleep
espflash flash ...
```

## Waking a powered-down chip

`?deep=true` sends PowerDown with wake-on-I2C enabled, so the **next I2C access wakes
it** - no reset needed. Verified: `deep=true` reports `deep_off:true`, and switching RF
back on brings `firmware 1.6.7` back without touching the reset line. Waking is a
firmware restart inside the chip, not an instant answer, so the reader suppresses
initialisation while powered down and re-runs it (firmware version + SAM
configuration) when RF returns.

RSTPDN also exits power-down, but as a full reset: SAM state and the activated card are
lost and the card has to be re-selected. That is the recovery path, not the everyday
wake path.

## License

MIT OR Apache-2.0, at your option - see `LICENSE-MIT`, `LICENSE-APACHE` and the
attribution notes in `NOTICE`. Third-party licences (dependency inventory,
licence texts, and the non-Cargo pieces in the flashed image such as the ESP-IDF
bootloader and Espressif's prebuilt radio libraries) are documented in
`THIRD_PARTY_LICENSES.md`.

## Bring-up notes

The reader must answer its first I2C access at boot, otherwise the init retries
every 5 s (`no PN532 on the I2C bus yet (...)`) and `/nfc/status` reports
`state:"failed"` with the reason. `I2C bus error (no status byte...)` means the bus
itself did not complete - a line held low, typically a module caught mid-transaction
by a flash reset or a sagging 5 V rail; power-cycle the module (esp-hal 1.2.2 has no
bus recovery). `malformed response frame` prints the raw bytes it rejected, which is
the fastest way in.

## Limits and next steps

- **NDEF writing** covers URI and text records, one or two per message, on MIFARE
  Classic (auth + block writes, trailers never touched, verified by reading back)
  and on Type 2/Ultralight (4-byte page writes, only onto a tag that already has a
  capability container). Longer messages need the long-form TLV; the encoder
  handles it but a 1K Classic card only offers ~140 usable bytes.
- **Not implemented: formatting a blank tag.** A Classic card with no MAD/NDEF
  container needs its sector trailers rewritten, and a wrong trailer makes those
  sectors unreadable for good. Format with a phone app instead. Type 2 is refused
  unless already formatted, for the same reason.
- **MIFARE Classic** reads try the well-known key A candidates (factory, MAD,
  NFC-Forum) because authentication is mandatory even for reads; nothing is written
  and no other keys are hunted. Type 2 tags need no auth.
- An empty field costs a full poll window: `MxRtyPassiveActivation` defaults to
  0xFF (retry forever), so `InListPassiveTarget` sends no response at all and the
  wait can only time out. That timeout means "no target", not a failure - the poll
  is then aborted by writing the ACK frame, so nothing is left running mid-flight.
  Measured: ~157 ms per empty poll, 0 errors, ~29 ms to see a card that is already
  on the coil. `/nfc/status` reports `last_poll_ms`.
- `power_down`/RF-field register control is wired but unverified on hardware.
