//! PN532 card reader for the Evolis Primacy's encoder slot.
//!
//! Tasks: Wi-Fi/DHCP, embassy-net runner, two HTTP workers, and one reader task
//! that owns the I2C bus, the PN532 and the status LED. HTTP handlers never
//! touch the bus: they publish a request epoch into `SHARED` and wait for the
//! reader task to complete it, so all PN532 traffic stays in one task and the
//! bus needs no locking.
//!
//! Board is a cargo feature (AtomS3 Lite by default, ESP32 DevKit for the
//! bench); the API is hand-rolled JSON into heapless buffers, no serde in the
//! response path.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially buffers"
)]

mod board;
mod led;
mod ndef;
mod pn532;

use core::fmt::Write as _;
use core::net::{Ipv4Addr, Ipv6Addr};
use core::sync::atomic::{AtomicU32, Ordering};

use alloc::boxed::Box;

use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use edge_mdns::HostAnswersMdnsHandler;
use edge_mdns::buf::VecBufAccess;
use edge_mdns::domain::base::Ttl;
use edge_mdns::host::{Host, Service, ServiceAnswers};
use edge_mdns::io::{self, Mdns};
use edge_nal::UdpSplit;
use edge_nal_embassy::{Udp, UdpBuffers};
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{AnyPin, Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::rng::Rng;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::{
    AuthenticationMethodConfig, Config as WifiConfig, ControllerConfig, Interface, WifiController,
    sta::StationConfig,
};
use log::{error, info, warn};
use picoserve::extract::Query;
use picoserve::response::{ContentBody, ContentHeaders, Response, StatusCode};
use picoserve::routing::get;
use serde::Deserialize;

use esp_backtrace as _;

use crate::led::{AMBER, Inputs, Rgb};
use crate::pn532::{Pn532, Target};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write($val);
        x
    }};
}

// --- configuration -------------------------------------------------------------

const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const MDNS_HOSTNAME: &str = env!("MDNS_HOSTNAME");

/// GPIO wired to the PN532's RSTPDN (active low), from `PN532_RESET_PIN` in `.env`.
/// Empty means no reset line is wired and the reader relies on the retry loop
/// alone. Pin numbers: on the DevKit testbed GPIO25/26/27 are free; on the AtomS3
/// Lite G5/G6/G7/G8/G38/G39 are on the spare header. Never a pin already in use
/// (I2C, LED) and never a flash pin (6-11) or a strapping pin (0/2/12/15).
const PN532_RESET_PIN: Option<u8> = parse_pin(env!("PN532_RESET_PIN"));

/// Decimal GPIO number -> pin. `const` because `Shared::new` is a `const fn` (it
/// initialises a `static`), and `str::parse` is not const.
const fn parse_pin(s: &str) -> Option<u8> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut n: u16 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let d = bytes[i];
        if !d.is_ascii_digit() {
            return None;
        }
        n = n * 10 + (d - b'0') as u16;
        if n > 255 {
            return None;
        }
        i += 1;
    }
    Some(n as u8)
}

const HTTP_PORT: u16 = 80;
const HTTP_TASKS: u8 = 2;

/// Every JSON response uses the same body type - picoserve gives the whole router
/// one body type - and must hold the largest document without silent truncation:
/// `/nfc/status` carries the card view, the NDEF text, its base64 and the result
/// of the last write. `heapless` strings truncate instead of failing, so an
/// overflowing buffer would emit a broken document rather than an error.
///
/// The size is a literal on purpose: a named const in the `heapless::String<N>`
/// position of the response type stops the router's size computation from
/// normalising (`E0080: cannot be normalized`).


/// How long a single poll may wait for a response. With a card in the field the
/// chip answers in ~30 ms; with an empty field it never answers at all (see
/// `list_passive_target`), so this is really the poll period.
const POLL_RF_TIMEOUT_MS: u64 = 150;
/// How long a REST-triggered exchange keeps the LED amber after it finished.
const REST_GLOW_MS: u64 = 300;
/// NDEF reads talk to the tag several times; give them room.
const NDEF_TIMEOUT_MS: u64 = 3000;
/// Retry interval when the PN532 is not answering (also covers hot-plugging).
const INIT_RETRY_MS: u64 = 5000;
/// A write is auth + up to 9 block writes + a read-back check; give it room.
const WRITE_TIMEOUT_MS: u64 = 8000;

static OUR_IP: AtomicU32 = AtomicU32::new(0);

// mDNS: a UDP socket on :5353 joined to 224.0.0.251. The buffers live in
// `static`s so the task future stays small; statics need a thread-safe raw
// mutex, hence `CriticalSectionRawMutex`.
static MDNS_RECV_BUF: VecBufAccess<CriticalSectionRawMutex, 1500> = VecBufAccess::new();
static MDNS_SEND_BUF: VecBufAccess<CriticalSectionRawMutex, 1500> = VecBufAccess::new();
/// Triggers an unsolicited announcement when `Host`/`Service` change.
static MDNS_BROADCAST: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// `rand_core::TryRng` over the hardware RNG - edge-mdns uses it for response
/// jitter.
struct EspRng(Rng);

impl rand_core::TryRng for EspRng {
    type Error = core::convert::Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        Ok(self.0.random())
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        Ok((u64::from(self.0.random()) << 32) | u64::from(self.0.random()))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Self::Error> {
        for chunk in dst.chunks_mut(4) {
            let bytes = self.0.random().to_le_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

// --- shared state --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Init {
    Booting,
    Ready([u8; 4]),
    Failed(&'static str),
}

impl Init {
    fn json(&self) -> &'static str {
        match self {
            Init::Booting => "booting",
            Init::Ready(_) => "ok",
            Init::Failed(_) => "failed",
        }
    }
    fn firmware(&self, out: &mut heapless::String<32>) {
        if let Init::Ready(fw) = self {
            let _ = write!(out, "{}.{}.{}", fw[1], fw[2], fw[3]);
        }
    }
}

/// Everything the two halves (HTTP workers, reader task) share. Guarded by one
/// blocking mutex; never held across an `await`.
struct Shared {
    // requests: bump an epoch, the reader task notices and answers
    rf_on: bool,
    rf_epoch: u32,
    /// `?on=false&deep=true`: send PowerDown (field truly off, chip asleep).
    deep_off: bool,
    deep_off_epoch: u32,
    deep_off_done: u32,
    /// True while the front end is powered down; suppresses the init retries
    /// (each I2C access would otherwise wake the chip again).
    deep_off_active: bool,
    scan_epoch: u32,
    ndef_epoch: u32,
    raw_epoch: u32,
    raw_cmd: u8,
    raw_args: heapless::Vec<u8, 32>,

    // published by the reader task
    init: Init,
    i2c_ack: bool,
    polls: u32,
    errors: u32,
    last_poll_ms: u32,
    last_error: &'static str,
    present: bool,
    target: Target,
    last_seen_ms: u64,
    ndef_epoch_done: u32,
    ndef_uid: [u8; 10],
    ndef_uid_len: u8,
    ndef_records: u8,
    ndef_ok: bool,
    ndef_tag: &'static str,
    /// Which Classic key opened the NDEF sector ("" when the tag has none).
    ndef_key: &'static str,
    /// Raw NDEF message bytes, for the base64 field.
    ndef_raw: heapless::Vec<u8, 192>,

    // requests: `GET /nfc/write` publishes what to write, the reader task writes
    // it to whatever card is under the reader
    write_epoch: u32,
    write_epoch_done: u32,
    write_uri: heapless::String<256>,
    write_text: heapless::String<256>,
    write_ok: bool,
    /// Reason a write failed, for the API and the log.
    write_error: heapless::String<96>,
    write_key: &'static str,
    write_bytes: u16,
    write_verified: bool,
    /// `PUT /nfc/ndef`: a ready-made NDEF message to write verbatim.
    write_blob: heapless::Vec<u8, 256>,
    /// Distinguishes "the blob is the message" from "encode uri/text".
    write_blob_mode: bool,

    // PN532 reset line (RSTPDN): clears a chip that is holding the bus low, which
    // otherwise needs a physical power cycle
    reset_pin: Option<u8>,
    reset_epoch: u32,
    reset_epoch_done: u32,
    /// How many times the chip has been reset since boot.
    resets: u16,
    ndef_text: heapless::String<384>,
    raw_epoch_done: u32,
    raw_resp: heapless::Vec<u8, 64>,
    raw_error: &'static str,
    led: &'static str,
    rest_until_ms: u64,
}

impl Shared {
    const fn new() -> Self {
        Self {
            rf_on: true,
            rf_epoch: 0,
            deep_off: false,
            deep_off_epoch: 0,
            deep_off_done: 0,
            deep_off_active: false,
            scan_epoch: 0,
            ndef_epoch: 0,
            raw_epoch: 0,
            raw_cmd: 0,
            raw_args: heapless::Vec::new(),
            init: Init::Booting,
            i2c_ack: false,
            polls: 0,
            errors: 0,
            last_poll_ms: 0,
            last_error: "",
            present: false,
            target: Target::EMPTY,
            last_seen_ms: 0,
            ndef_epoch_done: 0,
            ndef_uid: [0; 10],
            ndef_uid_len: 0,
            ndef_records: 0,
            ndef_ok: false,
            ndef_tag: "",
            ndef_key: "",
            ndef_raw: heapless::Vec::new(),
            write_epoch: 0,
            write_epoch_done: 0,
            write_uri: heapless::String::new(),
            write_text: heapless::String::new(),
            write_ok: false,
            write_error: heapless::String::new(),
            write_key: "",
            write_bytes: 0,
            write_verified: false,
            write_blob: heapless::Vec::new(),
            write_blob_mode: false,
            reset_pin: PN532_RESET_PIN,
            reset_epoch: 0,
            reset_epoch_done: 0,
            resets: 0,
            ndef_text: heapless::String::new(),
            raw_epoch_done: 0,
            raw_resp: heapless::Vec::new(),
            raw_error: "",
            led: "off",
            rest_until_ms: 0,
        }
    }
}

static SHARED: Mutex<CriticalSectionRawMutex, Shared> = Mutex::new(Shared::new());
/// Woken whenever presence changes (a `Signal`, so a second concurrent waiter
/// only learns late - it re-checks the state after its own timeout).
static PRESENCE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static NDEF_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RAW_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static WRITE_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static RESET_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static DEEP_DONE: Signal<CriticalSectionRawMutex, ()> = Signal::new();

async fn shared<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut guard = SHARED.lock().await;
    f(&mut guard)
}

// --- LED ------------------------------------------------------------------------

#[cfg(feature = "board-atoms3lite")]
type WsLed = esp_hal_smartled::RmtSmartLeds<
    'static,
    { esp_hal_smartled::buffer_size::<smart_leds::RGB8>(1) },
    esp_hal::Async,
    smart_leds::RGB8,
    esp_hal_smartled::color_order::Grb,
>;

/// The DevKit testbed has no addressable LED, so the state machine is only
/// observable in the log and `/nfc/status` there.
#[cfg(not(feature = "board-atoms3lite"))]
type WsLed = core::convert::Infallible;

async fn show(led: &mut Option<WsLed>, color: Rgb) {
    #[cfg(feature = "board-atoms3lite")]
    if let Some(led) = led.as_mut() {
        use smart_leds::SmartLedsWriteAsync;
        let px = [smart_leds::RGB8 {
            r: color.r,
            g: color.g,
            b: color.b,
        }];
        if led.write(px).await.is_err() {
            warn!("LED write failed");
        }
    }
    #[cfg(not(feature = "board-atoms3lite"))]
    {
        let _ = (led, color);
    }
}

// --- main ----------------------------------------------------------------------

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    info!("nfc-reader v{} on {}", env!("CARGO_PKG_VERSION"), board::BOARD);
    info!(
        "status LED: {}",
        if board::HAS_WS2812 {
            "WS2812 on GPIO35"
        } else {
            "none on this board - LED states only in /nfc/status and the log"
        }
    );

    // --- PN532 on I2C: Grove port on the AtomS3 Lite, classic ESP32 defaults
    // on the DevKit testbed. 4 wires only (no IRQ, no reset), so the driver
    // polls the status byte.
    #[cfg(feature = "board-atoms3lite")]
    let (sda, scl) = (peripherals.GPIO2, peripherals.GPIO1); // Grove: yellow SDA, white SCL
    #[cfg(feature = "board-esp32-devkit")]
    let (sda, scl) = (peripherals.GPIO21, peripherals.GPIO22);

    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(board::I2C_FREQ_KHZ)),
    )
    .expect("failed to build the I2C peripheral")
    .with_sda(sda)
    .with_scl(scl)
    .into_async();
    let pn532 = Pn532::new(i2c);

    // --- PN532 reset line (optional)
    // RSTPDN is an input on the module, so driving it from a GPIO lets the firmware
    // reset the chip itself instead of asking for a power cycle: it clears a chip
    // that is holding SDA low after being caught mid-transaction by an ESP32 reset,
    // and it makes the first init attempt after every flash start from a clean chip.
    let pn532_reset = PN532_RESET_PIN.map(|n| {
        info!("PN532 reset line on GPIO{n}");
        // Safety: the pin number comes from the build-time config and must not be
        // one the firmware uses elsewhere; `steal` is how esp-hal takes a pin whose
        // number is only known at runtime.
        let pin = unsafe { AnyPin::steal(n) };
        Output::new(pin, Level::High, OutputConfig::default())
    });
    if pn532_reset.is_none() {
        info!("no PN532 reset line configured (PN532_RESET_PIN unset)");
    }

    // --- LED
    #[cfg(feature = "board-atoms3lite")]
    let led = {
        let rmt = esp_hal::rmt::Rmt::new(peripherals.RMT, Rate::from_mhz(80))
            .expect("failed to initialize RMT")
            .into_async();
        esp_hal_smartled::RmtSmartLeds::new(
            esp_hal_smartled::WS2812_TIMING,
            rmt.channel0,
            peripherals.GPIO35,
            Rate::from_mhz(80),
        )
        .ok()
    };
    #[cfg(not(feature = "board-atoms3lite"))]
    let led: Option<WsLed> = None;

    spawner.spawn(reader_task(pn532, led, pn532_reset).unwrap());

    // --- Wi-Fi + HTTP. Optional: without credentials the reader still runs, so
    // the PN532 can be brought up without touching the network.
    if WIFI_SSID == "CHANGE_ME" || WIFI_SSID.is_empty() {
        warn!("WIFI_SSID is not configured - running without network (serial only)");
        loop {
            Timer::after(Duration::from_secs(30)).await;
            let (init, present) = shared(|s| (s.init, s.present)).await;
            info!(
                "heartbeat: reader={} card={} (no network: set WIFI_SSID in .env)",
                init.json(),
                present
            );
        }
    }

    let station_config = WifiConfig::Station(
        StationConfig::default()
            .with_ssid(WIFI_SSID.try_into().unwrap())
            .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                WIFI_PASSWORD.try_into().unwrap(),
            )),
    );

    let station = esp_radio::wifi::Interface::station();
    let controller = esp_radio::wifi::WifiController::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("failed to initialize the Wi-Fi controller");

    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let seed = {
        let rng = Rng::new();
        (u64::from(rng.random()) << 32) | u64::from(rng.random())
    };

    let (stack, runner) = embassy_net::new(
        station,
        net_config,
        // DHCP + 2 HTTP sockets + headroom for the TCP sockets picoserve opens.
        mk_static!(StackResources<6>, StackResources::<6>::new()),
        seed,
    );

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    stack.wait_config_up().await;
    let ip = match stack.config_v4() {
        Some(cfg) => {
            OUR_IP.store(u32::from(cfg.address.address()), Ordering::Relaxed);
            info!("got IPv4 address {}", cfg.address);
            cfg.address.address()
        }
        None => Ipv4Addr::UNSPECIFIED,
    };

    for id in 0..HTTP_TASKS {
        spawner.spawn(http_task(stack, id).unwrap());
    }
    spawner.spawn(mdns_task(stack).unwrap());
    info!("REST API on http://{ip}/nfc/status (also http://{MDNS_HOSTNAME}.local/)");

    loop {
        Timer::after(Duration::from_secs(30)).await;
        let (init, present, polls, errors) = shared(|s| (s.init, s.present, s.polls, s.errors)).await;
        info!(
            "heartbeat: reader={} card={present} polls={polls} errors={errors} heap={} B",
            init.json(),
            esp_alloc::HEAP.free()
        );
    }
}

// --- network tasks --------------------------------------------------------------

/// Advertises `<MDNS_HOSTNAME>.local` (A record) and `_http._tcp.local`
/// (SRV/TXT/PTR), and answers queries for both.
#[embassy_executor::task]
async fn mdns_task(stack: Stack<'static>) {
    let ip = Ipv4Addr::from(OUR_IP.load(Ordering::Relaxed));

    let host = Host {
        hostname: MDNS_HOSTNAME,
        ipv4: ip,
        ipv6: Ipv6Addr::UNSPECIFIED,
        ttl: Ttl::from_secs(120),
    };
    let service = Service {
        name: MDNS_HOSTNAME,
        priority: 0,
        weight: 0,
        service: "_http",
        protocol: "_tcp",
        port: HTTP_PORT,
        service_subtypes: &[],
        txt_kvs: &[("path", "/nfc/status")],
    };
    let answers = ServiceAnswers::new(&host, &service);

    // edge-nal's UDP factory over embassy-net, with a buffer pool. Local, not a
    // `static`: `Pool` is deliberately `!Sync`.
    let buffers = UdpBuffers::<1, 1500, 1500>::new();
    let udp = Udp::new(stack, &buffers);

    let mut socket = match io::bind(
        &udp,
        io::IPV4_DEFAULT_SOCKET,
        Some(Ipv4Addr::UNSPECIFIED),
        None,
    )
    .await
    {
        Ok(socket) => socket,
        Err(err) => {
            error!("mDNS: could not bind UDP :5353 ({:?})", err.erase());
            return;
        }
    };

    let (recv, send) = socket.split();
    let mdns = Mdns::new(
        Some(Ipv4Addr::UNSPECIFIED),
        None,
        recv,
        send,
        &MDNS_RECV_BUF,
        &MDNS_SEND_BUF,
        EspRng(Rng::new()),
        &MDNS_BROADCAST,
    );

    info!("mDNS: answering for {MDNS_HOSTNAME}.local -> {ip}");
    if let Err(err) = mdns
        .run(HostAnswersMdnsHandler::new(&answers))
        .await
        .map_err(|err| err.erase())
    {
        error!("mDNS responder stopped: {:?}", err);
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        info!("connecting to Wi-Fi SSID {WIFI_SSID:?}");
        match controller.connect_async().await {
            Ok(info) => {
                info!("Wi-Fi connected: {:?}", info);
                let info = controller.wait_for_disconnect_async().await.ok();
                warn!("Wi-Fi disconnected: {:?}", info);
            }
            Err(err) => error!("Wi-Fi connect failed: {:?}", err),
        }
        Timer::after(Duration::from_secs(5)).await;
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) {
    runner.run().await
}

// --- reader task ----------------------------------------------------------------

/// Hold RSTPDN low for 100 ms, the documented reset pulse, then let the chip
/// settle before the next command.
async fn pulse_reset(reset: &mut Option<Output<'static>>) {
    if let Some(pin) = reset.as_mut() {
        pin.set_low();
        Timer::after(Duration::from_millis(100)).await;
        pin.set_high();
        Timer::after(Duration::from_millis(150)).await;
    }
}

#[embassy_executor::task]
async fn reader_task(
    mut pn: Pn532<I2c<'static, esp_hal::Async>>,
    mut led: Option<WsLed>,
    mut reset: Option<Output<'static>>,
) {
    // Start from a chip that is known clean. Without a reset line, a PN532 caught
    // mid-transaction by the reset that flashing performs can hold a bus line low
    // and every retry then fails until someone power-cycles the module.
    if reset.is_some() {
        info!("resetting the PN532 (RSTPDN) at boot");
        pulse_reset(&mut reset).await;
    }

    let mut misses = 0u8;
    let mut init_failures = 0u32;
    let mut rf_seen = u32::MAX;
    let mut next_init = Instant::now();
    // Errors are the interesting case but the reader can sit in one for hours;
    // rate-limit by time, not by count, so a stalled reader is visible without
    // filling the log.
    let mut last_warn = Instant::now();

    loop {
        let now = Instant::now().as_millis();

        // --- `GET /nfc/reset`: pulse the chip and start over
        if shared(|s| s.reset_epoch != s.reset_epoch_done).await {
            let epoch = shared(|s| s.reset_epoch).await;
            if reset.is_some() {
                info!("resetting the PN532 on request");
                pulse_reset(&mut reset).await;
            } else {
                warn!("reset requested but no PN532 reset line is configured");
            }
            shared(|s| {
                s.reset_epoch_done = epoch;
                s.resets = s.resets.wrapping_add(1);
                s.init = Init::Booting;
                s.present = false;
                s.target = Target::EMPTY;
                s.ndef_key = "";
                s.ndef_raw.clear();
                s.ndef_text.clear();
                s.last_error = "";
            })
            .await;
            init_failures = 0;
            next_init = Instant::now();
            RESET_DONE.signal(());
        }

        let (init_ok, rf_on, rf_epoch, deep_off_active) = shared(|s| {
            (
                matches!(s.init, Init::Ready(_)),
                s.rf_on,
                s.rf_epoch,
                s.deep_off_active,
            )
        })
        .await;

        // --- (re)initialize the reader, and keep retrying so hot-plugging the
        // PN532 works and a card-less bench setup still comes up.
        if !init_ok && !deep_off_active {
            let mut came_up = false;
            if Instant::now() >= next_init {
                next_init = Instant::now() + Duration::from_millis(INIT_RETRY_MS);
                let ack = pn.probe().await;
                match pn.firmware_version().await {
                    Ok(fw) => {
                        info!("PN532 answered: IC={:#04x} firmware={}.{}.{}", fw[0], fw[1], fw[2], fw[3]);
                        match pn.sam_configuration().await {
                            Ok(()) => {
                                // No MxRtyPassiveActivation setup: with the factory
                                // 0xFF the poll never answers on an empty field,
                                // which `list_passive_target` handles by aborting
                                // the poll (see there). RFConfiguration item 0x05
                                // is rejected by this chip anyway.
                                info!("PN532 initialized (SAM normal mode)");
                                came_up = true;
                                shared(|s| {
                                    s.init = Init::Ready(fw);
                                    s.i2c_ack = true;
                                    s.last_error = "";
                                })
                                .await;
                            }
                            Err(e) => {
                                warn!("PN532 SAMConfiguration failed: {}", e.as_str());
                                shared(|s| s.last_error = e.as_str()).await;
                            }
                        }
                    }
                    Err(e) => {
                        shared(|s| {
                            s.init = Init::Failed(e.as_str());
                            s.i2c_ack = ack;
                            s.last_error = e.as_str();
                        })
                        .await;
                        if ack {
                            warn!("PN532 is on the bus but the exchange failed: {}", e.as_str());
                        } else {
                            info!("no PN532 on the I2C bus yet ({})", e.as_str());
                        }
                    }
                }
            } else {
                let inputs = Inputs {
                    rf_on,
                    present: false,
                    rest_active: false,
                    init_failed: true,
                };
                let color = led::desired(&inputs, now);
                let state = led::state_name(&inputs);
                shared(|s| s.led = state).await;
                show(&mut led, color).await;
                Timer::after(Duration::from_millis(250)).await;
                continue;
            }
            // Three failed attempts in a row with a reset line available means
            // retrying is not helping (typically a chip holding the bus), so reset
            // it instead of nagging until someone walks over with a power cycle.
            if came_up {
                init_failures = 0;
            } else {
                init_failures = init_failures.saturating_add(1);
                if init_failures % 3 == 0 && reset.is_some() {
                    warn!("PN532 still not answering after {init_failures} attempts - resetting it");
                    pulse_reset(&mut reset).await;
                    shared(|s| {
                        s.resets = s.resets.wrapping_add(1);
                        s.init = Init::Booting;
                    })
                    .await;
                    next_init = Instant::now();
                }
            }
            // Whether that attempt worked or not, re-evaluate from the top:
            // falling through here would poll a reader that just failed.
            continue;
        }

        // --- RF switched off: release the card once, then idle.
        if !rf_on {
            if rf_seen != rf_epoch {
                rf_seen = rf_epoch;
                let _ = pn.in_release(1).await;
                shared(|s| {
                    s.present = false;
                    // The reader is not looking any more: drop the card view so
                    // the API only ever reports what is under it *now*.
                    s.target = Target::EMPTY;
                    s.last_error = "RF off";
                })
                .await;
                info!("RF polling disabled");
            }
            let (want_deep, deep_epoch) = shared(|s| (s.deep_off, s.deep_off_epoch)).await;
            if want_deep {
                match pn.power_down(true).await {
                    Ok(()) => {
                        info!(
                            "PN532 powered down: RF field off, chip asleep until the next I2C access"
                        );
                        shared(|s| {
                            s.deep_off = false;
                            s.deep_off_done = deep_epoch;
                            s.deep_off_active = true;
                            s.init = Init::Booting;
                        })
                        .await;
                    }
                    Err(e) => {
                        warn!("PowerDown failed: {}", e.as_str());
                        shared(|s| {
                            s.deep_off = false;
                            s.deep_off_done = deep_epoch;
                            s.last_error = e.as_str();
                        })
                        .await;
                    }
                }
                DEEP_DONE.signal(());
            }
            let inputs = Inputs {
                rf_on: false,
                present: false,
                rest_active: false,
                init_failed: false,
            };
            let color = led::desired(&inputs, now);
            let state = led::state_name(&inputs);
            shared(|s| s.led = state).await;
            show(&mut led, color).await;
            Timer::after(Duration::from_millis(200)).await;
            continue;
        }
        rf_seen = rf_epoch;

        // --- one RF poll
        let poll_start = Instant::now();
        let found = pn.list_passive_target(POLL_RF_TIMEOUT_MS).await;
        let poll_ms = (Instant::now() - poll_start).as_millis() as u32;

        let mut present_now = false;
        match found {
            Ok(Some(t)) => {
                misses = 0;
                present_now = true;
                let changed = shared(|s| {
                    s.polls += 1;
                    s.last_poll_ms = poll_ms;
                    s.errors = 0;
                    s.last_error = "";
                    if !s.present || s.target.uid != t.uid {
                        s.present = true;
                        s.target = t;
                        s.last_seen_ms = now;
                        s.ndef_ok = false;
                        s.ndef_records = 0;
                        s.ndef_key = "";
                        s.ndef_raw.clear();
                        s.ndef_text.clear();
                        s.ndef_uid = [0; 10];
                        s.ndef_uid_len = 0;
                        true
                    } else {
                        s.last_seen_ms = now;
                        false
                    }
                })
                .await;
                if changed {
                    let mut hex = heapless::String::<32>::new();
                    t.uid_hex(&mut hex);
                    let mut dec = heapless::String::<32>::new();
                    t.uid_decimal(&mut dec);
                    info!("card: {hex} ({dec}) {} ATQA={:02x}{:02x} SAK={:02x}", t.kind(), t.atqa[0], t.atqa[1], t.sak);
                    PRESENCE.signal(());
                }
            }
            Ok(None) => {
                shared(|s| {
                    s.polls += 1;
                    s.last_poll_ms = poll_ms;
                })
                .await;
                misses = misses.saturating_add(1);
                if misses >= board::PRESENCE_MISSES {
                    let was = shared(|s| {
                        let was = s.present;
                        s.present = false;
                        s.target = Target::EMPTY;
                        was
                    })
                    .await;
                    if was {
                        info!("card removed");
                        PRESENCE.signal(());
                    }
                }
            }
            Err(e) => {
                shared(|s| {
                    s.polls += 1;
                    s.errors = s.errors.saturating_add(1);
                    s.last_error = e.as_str();
                    s.last_poll_ms = poll_ms;
                })
                .await;
                // A failed poll is ambiguous on its own, so ask the chip
                // something cheap: that separates "the reader is gone" from "the
                // RF round failed once".
                let gone = e == crate::pn532::Error::NoDevice
                    || pn.firmware_version().await.is_err();
                if gone {
                    // Reader vanished (unplugged / powered down / bus stuck): go
                    // back to the init path instead of hammering the bus.
                    shared(|s| {
                        s.init = Init::Failed(e.as_str());
                        s.present = false;
                        s.target = Target::EMPTY;
                    })
                    .await;
                    next_init = Instant::now() + Duration::from_millis(INIT_RETRY_MS);
                } else if Instant::now() >= last_warn + Duration::from_secs(30) {
                    last_warn = Instant::now();
                    warn!("RF poll error: {}", e.as_str());
                }
            }
        }

        // --- serve REST-triggered work (needs a card under the reader)
        let want_ndef = shared(|s| s.ndef_epoch != s.ndef_epoch_done).await;
        let want_raw = shared(|s| s.raw_epoch != s.raw_epoch_done).await;
        let want_write = shared(|s| s.write_epoch != s.write_epoch_done).await;

        if want_ndef || want_raw || want_write {
            let target = shared(|s| s.target).await;
            let present = present_now || shared(|s| s.present).await;

            // Amber while the reader is actually talking to the tag.
            shared(|s| s.rest_until_ms = Instant::now().as_millis() + REST_GLOW_MS).await;
            show(&mut led, AMBER).await;

            if want_ndef {
                let epoch = shared(|s| s.ndef_epoch).await;
                if present {
                    let report = ndef::read(&mut pn, &target).await;
                    shared(|s| {
                        s.ndef_epoch_done = epoch;
                        s.ndef_ok = report.ok;
                        s.ndef_records = report.records;
                        s.ndef_tag = report.tag;
                        s.ndef_key = report.key;
                        s.ndef_raw = report.raw;
                        s.ndef_text = report.text;
                        s.ndef_uid = target.uid;
                        s.ndef_uid_len = target.uid_len;
                    })
                    .await;
                    info!("NDEF read done: ok={} records={}", report.ok, report.records);
                } else {
                    shared(|s| {
                        s.ndef_epoch_done = epoch;
                        s.ndef_ok = false;
                        s.ndef_records = 0;
                        s.ndef_tag = "none";
                        s.ndef_key = "";
                        s.ndef_raw.clear();
                        s.ndef_text.clear();
                        let _ = s.ndef_text.push_str("no card under the reader");
                    })
                    .await;
                }
                NDEF_DONE.signal(());
            }

            if want_raw {
                let epoch = shared(|s| s.raw_epoch).await;
                let (cmd, args) = shared(|s| {
                    let mut args = heapless::Vec::<u8, 32>::new();
                    let _ = args.extend_from_slice(&s.raw_args);
                    (s.raw_cmd, args)
                })
                .await;
                if present {
                    match pn.raw(cmd, &args, 64, 1000).await {
                        Ok(resp) => {
                            shared(|s| {
                                s.raw_epoch_done = epoch;
                                s.raw_resp.clear();
                                let _ = s.raw_resp.extend_from_slice(resp);
                                s.raw_error = "";
                            })
                            .await;
                        }
                        Err(e) => {
                            shared(|s| {
                                s.raw_epoch_done = epoch;
                                s.raw_resp.clear();
                                s.raw_error = e.as_str();
                            })
                            .await;
                        }
                    }
                } else {
                    shared(|s| {
                        s.raw_epoch_done = epoch;
                        s.raw_error = "no card under the reader";
                    })
                    .await;
                }
                RAW_DONE.signal(());
            }

            if want_write {
                let epoch = shared(|s| s.write_epoch).await;
                let (uri, text) = shared(|s| (s.write_uri.clone(), s.write_text.clone())).await;
                let (blob, blob_mode) =
                    shared(|s| (s.write_blob.clone(), s.write_blob_mode)).await;
                if present {
                    // Amber while the reader is talking to the tag.
                    shared(|s| s.rest_until_ms = Instant::now().as_millis() + REST_GLOW_MS).await;
                    let report = if blob_mode {
                        // `PUT /nfc/ndef`: the caller supplied the message itself.
                        ndef::write_message(&mut pn, &target, &blob).await
                    } else {
                        ndef::write(
                            &mut pn,
                            &target,
                            if uri.is_empty() { None } else { Some(uri.as_str()) },
                            if text.is_empty() { None } else { Some(text.as_str()) },
                        )
                        .await
                    };
                    if report.ok {
                        info!(
                            "wrote {} bytes to {} (key {}, verified={})",
                            report.bytes,
                            target.kind(),
                            report.key,
                            report.verified
                        );
                    } else {
                        warn!("write failed: {}", report.text.as_str());
                    }
                    shared(|s| {
                        s.write_epoch_done = epoch;
                        s.write_ok = report.ok;
                        s.write_key = report.key;
                        s.write_bytes = report.bytes;
                        s.write_verified = report.verified;
                        s.write_error.clear();
                        if !report.ok {
                            let _ = s.write_error.push_str(report.text.as_str());
                        }
                    })
                    .await;
                } else {
                    shared(|s| {
                        s.write_epoch_done = epoch;
                        s.write_ok = false;
                        s.write_error.clear();
                        let _ = s.write_error.push_str("no card under the reader");
                    })
                    .await;
                }
                WRITE_DONE.signal(());
            }
        }

        // --- LED
        let last = shared(|s| s.last_seen_ms).await;
        let rest_active = Instant::now().as_millis() < shared(|s| s.rest_until_ms).await;
        let inputs = Inputs {
            rf_on: true,
            present: present_now && Instant::now().as_millis() <= last + 1500,
            rest_active,
            init_failed: false,
        };
        let color = led::desired(&inputs, now);
        let state = led::state_name(&inputs);
        shared(|s| s.led = state).await;
        show(&mut led, color).await;

        Timer::after(Duration::from_millis(board::POLL_INTERVAL_MS)).await;
    }
}

// --- HTTP -----------------------------------------------------------------------

type Json = Response<ContentHeaders, ContentBody<heapless::String<1200>>>;

fn json(body: heapless::String<1200>, code: StatusCode) -> Json {
    Response::new(code, body).with_content_type("application/json")
}

#[derive(Deserialize)]
struct WaitQuery {
    #[serde(default)]
    wait_ms: Option<u32>,
}

#[derive(Deserialize)]
struct RfQuery {
    #[serde(default)]
    on: Option<bool>,
    /// `?on=false&deep=true` additionally sends PowerDown: the field is truly
    /// off (microwatts) and the PN532 needs re-initializing to come back.
    #[serde(default)]
    deep: Option<bool>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct WriteQuery {
    /// A URI record. Percent-encode it: `&`/`=` would end the query field.
    #[serde(default)]
    url: Option<heapless::String<256>>,
    /// A text record (UTF-8, language "en").
    #[serde(default)]
    text: Option<heapless::String<256>>,
}

#[derive(Deserialize)]
struct RawQuery {
    #[serde(default)]
    cmd: Option<heapless::String<8>>,
    #[serde(default)]
    args: Option<heapless::String<64>>,
}

#[embassy_executor::task(pool_size = 2)]
async fn http_task(stack: Stack<'static>, id: u8) {
    let app = picoserve::Router::new()
        .route("/nfc/status", get(get_status))
        .route("/nfc/uid", get(get_uid))
        .route("/nfc/card", get(get_card))
        .route("/nfc/scan", get(get_scan))
        .route("/nfc/ndef", get(get_ndef))
        // Control routes are GET-with-params on purpose: this is a device API
        // driven by curl and the spooler, and `curl host/nfc/rf?on=false` is
        // the whole client. There is no cache between them on the LAN.
        .route("/nfc/rf", get(rf_control))
        .route("/nfc/reset", get(get_reset))
        .route("/nfc/write", get(get_write))
        .route("/nfc/ndef", get(get_ndef).put(put_ndef))
        .route("/nfc/raw", get(get_raw));

    let http_buffer: &'static mut [u8] = Box::leak(Box::new([0u8; 2048]));
    let rx_buffer: &'static mut [u8] = Box::leak(Box::new([0u8; 1536]));
    let tx_buffer: &'static mut [u8] = Box::leak(Box::new([0u8; 1536]));

    static CONFIG: picoserve::Config = picoserve::Config::const_default();

    let _ = picoserve::Server::new(&app, &CONFIG, http_buffer)
        .listen_and_serve(id, stack, HTTP_PORT, rx_buffer, tx_buffer)
        .await;
}

/// `/nfc/status` - everything about the reader right now.
async fn get_status() -> Json {
    let mut b = heapless::String::<1200>::new();
    let (
        board,
        ip,
        rf_on,
        init,
        i2c_ack,
        present,
        target,
        polls,
        errors,
        last_error,
        poll_ms,
        last_seen,
        led,
        ndef_ok,
        ndef_records,
        ndef_tag,
        ndef_key,
        ndef_raw,
        ndef_text,
        write_ok,
        write_bytes,
        write_key,
        write_verified,
        write_error,
        resets,
        reset_pin,
        uptime,
    ) =
        shared(|s| {
            (
                board::BOARD,
                Ipv4Addr::from(OUR_IP.load(Ordering::Relaxed)),
                s.rf_on,
                s.init,
                s.i2c_ack,
                s.present,
                s.target,
                s.polls,
                s.errors,
                s.last_error,
                s.last_poll_ms,
                s.last_seen_ms,
                s.led,
                s.ndef_ok,
                s.ndef_records,
                s.ndef_tag,
                s.ndef_key,
                s.ndef_raw.clone(),
                s.ndef_text.clone(),
                s.write_ok,
                s.write_bytes,
                s.write_key,
                s.write_verified,
                s.write_error.clone(),
                s.resets,
                s.reset_pin,
                Instant::now().as_secs(),
            )
        })
        .await;

    let mut fw = heapless::String::<32>::new();
    init.firmware(&mut fw);
    // -1 rather than null: one less shape for a client to handle
    let (reset_count, reset_pin_i) = (resets, reset_pin.map(|n| n as i16).unwrap_or(-1));
    let mut uid = heapless::String::<32>::new();
    let mut dec = heapless::String::<32>::new();
    target.uid_hex(&mut uid);
    target.uid_decimal(&mut dec);

    let _ = write!(
        b,
        concat!(
            r#"{{"board":"{}","target":"{}","fw":"{} {}","uptime_s":{},"ip":"{}","#,
            r#""reader":{{"state":"{}","firmware":"{}","i2c_ack":{},"error":"{}","polls":{},"errors":{},"last_poll_ms":{}}},"#,
            r#""rf":{},"led":"{}","card":{{"present":{},"uid":"{}","uid_decimal":"{}","kind":"{}","sak":{},"atqa":"{:02x}{:02x}","last_seen_ms":{}}},"#,
            r#""resets":{},"reset_pin":{},"ndef":{{"ok":{},"records":{},"tag":"{}","key":"{}","text":{:?},"msg_len":{},"b64":"{}"}},"#,
            r#""write":{{"ok":{},"bytes":{},"key":"{}","verified":{},"error":{:?}}}}}"#
        ),
        board,
        env!("CHIP_TARGET"),
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        uptime,
        ip,
        init.json(),
        fw,
        i2c_ack,
        last_error,
        polls,
        errors,
        poll_ms,
        rf_on,
        led,
        present,
        uid,
        dec,
        target.kind(),
        target.sak,
        target.atqa[0],
        target.atqa[1],
        last_seen,
        reset_count,
        reset_pin_i,
        ndef_ok,
        ndef_records,
        ndef_tag,
        ndef_key,
        JsonStr(&ndef_text),
        ndef_raw.len(),
        B64(&ndef_raw[..ndef_raw.len().min(B64_MAX_BYTES)]),
        write_ok,
        write_bytes,
        write_key,
        write_verified,
        JsonStr(&write_error),
    );

    json(b, StatusCode::OK)
}

/// A JSON string with real escaping. `{:?}` is not JSON - Rust writes control
/// characters as `\u{7}` where JSON needs `\u0007` - and the previous version
/// escaped only newlines, so a quote or backslash in an NDEF payload produced a
/// broken document. Payloads are attacker-ish data: they come off a tag.
struct JsonStr<'a>(&'a str);

impl core::fmt::Debug for JsonStr<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_char('"')?;
        for c in self.0.chars() {
            match c {
                '"' => f.write_str("\\\"")?,
                '\\' => f.write_str("\\\\")?,
                '\n' => f.write_str("\\n")?,
                '\r' => f.write_str("\\r")?,
                '\t' => f.write_str("\\t")?,
                '\u{8}' => f.write_str("\\b")?,
                '\u{c}' => f.write_str("\\f")?,
                c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
                c => f.write_char(c)?,
            }
        }
        f.write_char('"')
    }
}

/// How many raw message bytes go out base64-encoded. The HTTP body buffer is
/// fixed and shares it with the decoded text; a URL or text record
/// is far below this, and `msg_len` still reports the true size.
const B64_MAX_BYTES: usize = 120;

/// Standard base64 with padding, for the raw NDEF message bytes. An NDEF payload
/// is a byte string, not text: this is what a caller decodes when the record is
/// not something we can render.
struct B64<'a>(&'a [u8]);

impl core::fmt::Display for B64<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for chunk in self.0.chunks(3) {
            let n = ((chunk[0] as u32) << 16)
                | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
                | (*chunk.get(2).unwrap_or(&0) as u32);
            f.write_char(T[(n >> 18) as usize & 63] as char)?;
            f.write_char(T[(n >> 12) as usize & 63] as char)?;
            f.write_char(if chunk.len() > 1 {
                T[(n >> 6) as usize & 63] as char
            } else {
                '='
            })?;
            f.write_char(if chunk.len() > 2 {
                T[n as usize & 63] as char
            } else {
                '='
            })?;
        }
        Ok(())
    }
}

async fn get_uid(Query(q): Query<WaitQuery>) -> Json {
    card_json(q.wait_ms.unwrap_or(0) as u64, false).await
}

async fn get_card(Query(q): Query<WaitQuery>) -> Json {
    card_json(q.wait_ms.unwrap_or(0) as u64, true).await
}

async fn card_json(wait_ms: u64, verbose: bool) -> Json {
    if wait_ms > 0 && !shared(|s| s.present).await {
        wait_for_card(wait_ms).await;
    }
    let (present, target, last_seen) = shared(|s| (s.present, s.target, s.last_seen_ms)).await;
    let mut uid = heapless::String::<32>::new();
    let mut dec = heapless::String::<32>::new();
    target.uid_hex(&mut uid);
    target.uid_decimal(&mut dec);

    let mut b = heapless::String::<1200>::new();
    if !present {
        let _ = write!(b, r#"{{"present":false}}"#);
        return json(b, StatusCode::NOT_FOUND);
    }
    if verbose {
        let _ = write!(
            b,
            r#"{{"present":true,"uid":"{}","uid_decimal":"{}","kind":"{}","sak":{},"atqa":"{:02x}{:02x}","last_seen_ms":{}}}"#,
            uid,
            dec,
            target.kind(),
            target.sak,
            target.atqa[0],
            target.atqa[1],
            last_seen
        );
    } else {
        let _ = write!(
            b,
            r#"{{"present":true,"uid":"{}","uid_decimal":"{}"}}"#,
            uid, dec
        );
    }
    json(b, StatusCode::OK)
}

/// `/nfc/scan?wait_ms=` - the same cached read as `/nfc/card`, with a longer
/// default wait (1000 ms) for a card to appear. It does **not** drive the antenna:
/// the reader polls continuously, so this can only ever be as fresh as the last
/// poll. `scan_epoch` below is reserved for a real forced poll if one is ever
/// needed - see the note in the README - and deliberately has no consumer today.
async fn get_scan(Query(q): Query<WaitQuery>) -> Json {
    shared(|s| s.scan_epoch = s.scan_epoch.wrapping_add(1)).await;
    let wait = q.wait_ms.unwrap_or(1000) as u64;
    if !shared(|s| s.present).await {
        wait_for_card(wait).await;
    }
    card_json(0, true).await
}

async fn rf_control(Query(q): Query<RfQuery>) -> Json {
    let mut b = heapless::String::<1200>::new();
    match q.on {
        Some(on) => {
            shared(|s| {
                s.rf_on = on;
                s.rf_epoch = s.rf_epoch.wrapping_add(1);
                if on {
                    s.deep_off_active = false;
                }
            })
            .await;
            let mut deep_done = false;
            if !on && q.deep == Some(true) {
                let epoch = shared(|s| {
                    s.deep_off = true;
                    s.deep_off_epoch = s.deep_off_epoch.wrapping_add(1);
                    s.deep_off_epoch
                })
                .await;
                let _ = with_timeout(Duration::from_millis(1500), DEEP_DONE.wait()).await;
                deep_done =
                    shared(|s| s.deep_off_done == epoch && s.deep_off_active).await;
            }
            let _ = write!(b, r#"{{"rf":{},"deep_off":{}}}"#, on, deep_done);
            json(b, StatusCode::OK)
        }
        None => {
            let rf = shared(|s| s.rf_on).await;
            let _ = write!(b, r#"{{"rf":{},"error":"pass ?on=true|false"}}"#, rf);
            json(b, StatusCode::BAD_REQUEST)
        }
    }
}

/// `/nfc/ndef` - read the NDEF message off the card that is under the reader now.
async fn get_ndef() -> Json {
    let mut b = heapless::String::<1200>::new();
    let present = shared(|s| s.present).await;
    if !present {
        let _ = write!(b, r#"{{"ok":false,"error":"no card under the reader"}}"#);
        return json(b, StatusCode::NOT_FOUND);
    }

    let epoch = shared(|s| {
        s.ndef_epoch = s.ndef_epoch.wrapping_add(1);
        s.ndef_epoch
    })
    .await;
    let _ = with_timeout(Duration::from_millis(NDEF_TIMEOUT_MS + 500), NDEF_DONE.wait()).await;

    let (done, ok, records, tag, key, raw, text, uid, uid_len) = shared(|s| {
        (
            s.ndef_epoch_done,
            s.ndef_ok,
            s.ndef_records,
            s.ndef_tag,
            s.ndef_key,
            s.ndef_raw.clone(),
            s.ndef_text.clone(),
            s.ndef_uid,
            s.ndef_uid_len,
        )
    })
    .await;
    if done != epoch {
        let _ = write!(b, r#"{{"ok":false,"error":"reader busy"}}"#);
        return json(b, StatusCode::SERVICE_UNAVAILABLE);
    }
    let _ = write!(
        b,
        r#"{{"ok":{},"records":{},"tag":"{}","uid":"{}","key":"{}","text":{:?},"msg_len":{},"b64":"{}"}}"#,
        ok,
        records,
        tag,
        HexUid(&uid[..(uid_len as usize).min(10)]),
        key,
        JsonStr(&text),
        raw.len(),
        B64(&raw[..raw.len().min(B64_MAX_BYTES)])
    );
    json(b, if ok { StatusCode::OK } else { StatusCode::NOT_FOUND })
}

/// `GET /nfc/reset` - pulse the PN532's RSTPDN and re-initialize. This is the
/// software equivalent of the power cycle that a chip holding the bus low
/// otherwise needs.
async fn get_reset() -> Json {
    let mut b = heapless::String::<1200>::new();
    if shared(|s| s.reset_pin).await.is_none() {
        let _ = write!(
            b,
            r#"{{"ok":false,"error":"no reset line configured: set PN532_RESET_PIN in .env"}}"#
        );
        return json(b, StatusCode::NOT_IMPLEMENTED);
    }
    let epoch = shared(|s| {
        s.reset_epoch = s.reset_epoch.wrapping_add(1);
        s.reset_epoch
    })
    .await;
    let _ = with_timeout(Duration::from_secs(5), RESET_DONE.wait()).await;
    let (done, resets) = shared(|s| (s.reset_epoch_done, s.resets)).await;
    if done != epoch {
        let _ = write!(b, r#"{{"ok":false,"error":"reader busy"}}"#);
        return json(b, StatusCode::SERVICE_UNAVAILABLE);
    }
    // The pulse and the re-initialisation are not the same event: wait for the
    // reader to settle so the answer says whether the chip actually came back.
    let mut init = shared(|s| s.init).await;
    for _ in 0..40 {
        if !matches!(init, Init::Booting) {
            break;
        }
        Timer::after(Duration::from_millis(100)).await;
        init = shared(|s| s.init).await;
    }
    let _ = write!(
        b,
        r#"{{"ok":{},"resets":{},"reader":{}}}"#,
        matches!(init, Init::Ready(_)),
        resets,
        init.json()
    );
    json(b, StatusCode::OK)
}

/// `/nfc/write?url=…&text=…` - write an NDEF message to the card under the reader.
/// Whatever is on the coil gets written; there is no arming, so a card that
/// arrives later is not touched.
async fn get_write(Query(q): Query<WriteQuery>) -> Json {
    let mut b = heapless::String::<1200>::new();
    if q.url.is_none() && q.text.is_none() {
        let _ = write!(b, r#"{{"ok":false,"error":"pass url= and/or text="}}"#);
        return json(b, StatusCode::BAD_REQUEST);
    }
    if !shared(|s| s.present).await {
        let _ = write!(b, r#"{{"ok":false,"error":"no card under the reader"}}"#);
        return json(b, StatusCode::NOT_FOUND);
    }

    let epoch = shared(|s| {
        s.write_uri.clear();
        if let Some(u) = &q.url {
            let _ = s.write_uri.push_str(u);
        }
        s.write_text.clear();
        if let Some(t) = &q.text {
            let _ = s.write_text.push_str(t);
        }
        s.write_error.clear();
        s.write_blob.clear();
        s.write_blob_mode = false;
        s.write_epoch = s.write_epoch.wrapping_add(1);
        s.write_epoch
    })
    .await;
    write_outcome(epoch).await
}

/// Wait for the reader task to finish the write armed as `epoch`, and render the
/// outcome. Shared by both write entry points.
async fn write_outcome(epoch: u32) -> Json {
    let mut b = heapless::String::<1200>::new();
    let _ = with_timeout(Duration::from_millis(WRITE_TIMEOUT_MS), WRITE_DONE.wait()).await;

    let (done, ok, key, bytes, verified, err) = shared(|s| {
        (
            s.write_epoch_done,
            s.write_ok,
            s.write_key,
            s.write_bytes,
            s.write_verified,
            s.write_error.clone(),
        )
    })
    .await;
    if done != epoch {
        let _ = write!(b, r#"{{"ok":false,"error":"reader busy"}}"#);
        return json(b, StatusCode::SERVICE_UNAVAILABLE);
    }
    if ok {
        let _ = write!(
            b,
            r#"{{"ok":true,"bytes":{},"key":"{}","verified":{}}}"#,
            bytes, key, verified
        );
        json(b, StatusCode::OK)
    } else {
        let _ = write!(b, r#"{{"ok":false,"error":{:?}}}"#, JsonStr(&err));
        json(b, StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// `PUT /nfc/ndef` - write a ready-made NDEF message to the card under the reader.
/// The body is the message, i.e. the records between the TLV header and the
/// terminator, which is what every NDEF library hands you; the container (TLV,
/// block/page layout, trailer skipping) stays ours. Raw bytes on purpose: a
/// payload should not have to survive a query string, and binary payloads cannot.
/// The body arrives as a `heapless::Vec` extractor: it is not generic over the
/// reader type (which is what a handler has to be), and it is sized against
/// Content-Length, so an oversized message is rejected rather than truncated -
/// a truncated NDEF message would be written to the card as garbage.
async fn put_ndef(data: heapless::Vec<u8, 256>) -> Json {
    let mut b = heapless::String::<1200>::new();
    if data.is_empty() {
        let _ = write!(b, r#"{{"ok":false,"error":"empty body: send an NDEF message"}}"#);
        return json(b, StatusCode::BAD_REQUEST);
    }
    if !shared(|s| s.present).await {
        let _ = write!(b, r#"{{"ok":false,"error":"no card under the reader"}}"#);
        return json(b, StatusCode::NOT_FOUND);
    }

    let epoch = shared(|s| {
        s.write_blob.clear();
        let n = data.len().min(s.write_blob.capacity());
        let _ = s.write_blob.extend_from_slice(&data[..n]);
        s.write_blob_mode = true;
        s.write_uri.clear();
        s.write_text.clear();
        s.write_error.clear();
        s.write_epoch = s.write_epoch.wrapping_add(1);
        s.write_epoch
    })
    .await;
    write_outcome(epoch).await
}

struct HexUid<'a>(&'a [u8]);
impl core::fmt::Display for HexUid<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, "-")?;
            }
            write!(f, "{:02X}", b)?;
        }
        Ok(())
    }
}

#[cfg(feature = "raw-api")]
async fn get_raw(Query(q): Query<RawQuery>) -> Json {
    let mut b = heapless::String::<1200>::new();
    let Some(cmd) = q.cmd.as_deref().and_then(parse_hex_u8) else {
        let _ = write!(b, r#"{{"error":"pass ?cmd=02&args=0102 (hex)"}}"#);
        return json(b, StatusCode::BAD_REQUEST);
    };

    let mut args = heapless::Vec::<u8, 32>::new();
    if let Some(a) = q.args.as_deref() {
        if !parse_hex_bytes(a, &mut args) {
            let _ = write!(b, r#"{{"error":"args must be hex bytes"}}"#);
            return json(b, StatusCode::BAD_REQUEST);
        }
    }

    let epoch = shared(|s| {
        s.raw_cmd = cmd;
        s.raw_args.clear();
        let _ = s.raw_args.extend_from_slice(&args);
        s.raw_epoch = s.raw_epoch.wrapping_add(1);
        s.raw_epoch
    })
    .await;
    let _ = with_timeout(Duration::from_millis(2000), RAW_DONE.wait()).await;

    let (done, resp, err) = shared(|s| (s.raw_epoch_done, s.raw_resp.clone(), s.raw_error)).await;
    if done != epoch {
        let _ = write!(b, r#"{{"error":"reader busy"}}"#);
        return json(b, StatusCode::SERVICE_UNAVAILABLE);
    }
    if !err.is_empty() {
        let _ = write!(b, r#"{{"ok":false,"error":"{}"}}"#, err);
        return json(b, StatusCode::NOT_FOUND);
    }
    let _ = write!(b, r#"{{"ok":true,"cmd":"{:02x}","resp":""#, cmd);
    for byte in resp.iter() {
        let _ = write!(b, "{:02x}", byte);
    }
    let _ = write!(b, r#""}}"#);
    json(b, StatusCode::OK)
}

#[cfg(not(feature = "raw-api"))]
async fn get_raw() -> Json {
    let mut b = heapless::String::<1200>::new();
    let _ = write!(
        b,
        r#"{{"error":"build with --features raw-api to use the raw command endpoint"}}"#
    );
    json(b, StatusCode::NOT_IMPLEMENTED)
}

#[cfg(feature = "raw-api")]
fn parse_hex_u8(s: &str) -> Option<u8> {
    let s = s.trim_start_matches("0x");
    u8::from_str_radix(s, 16).ok()
}

#[cfg(feature = "raw-api")]
fn parse_hex_bytes(s: &str, out: &mut heapless::Vec<u8, 32>) -> bool {
    let cleaned: heapless::String<64> = s.chars().filter(|c| *c != ',' && *c != ' ').collect();
    if !cleaned.len().is_multiple_of(2) {
        return false;
    }
    let bytes = cleaned.as_bytes();
    for pair in bytes.chunks(2) {
        let hi = (pair[0] as char).to_digit(16);
        let lo = (pair[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) if out.push((h * 16 + l) as u8).is_ok() => {}
            _ => return false,
        }
    }
    true
}

/// Wait until a card shows up, up to `wait_ms`. Re-checks the state after every
/// wake-up, so a missed `Signal` only costs latency.
async fn wait_for_card(wait_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(wait_ms.min(30_000));
    loop {
        if shared(|s| s.present).await {
            return;
        }
        if Instant::now() >= deadline {
            return;
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::from_millis(50));
        let _ = with_timeout(left, PRESENCE.wait()).await;
    }
}
