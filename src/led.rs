//! LED state machine, board independent.
//!
//! One pixel has to say four things, so precedence matters:
//! red (init failed) > amber (a REST-triggered exchange is in flight) >
//! green (card present) > dim white (RF on, nothing there) > off (RF off).
//! Amber blinks over green so a REST read is visible while a card sits there.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const OFF: Rgb = Rgb { r: 0, g: 0, b: 0 };
/// Not "off": the reader is live and sees nothing, so it is clearly on.
pub const DIM_WHITE: Rgb = Rgb {
    r: 8,
    g: 8,
    b: 10,
};
pub const GREEN: Rgb = Rgb {
    r: 0,
    g: 48,
    b: 0,
};
pub const AMBER: Rgb = Rgb {
    r: 64,
    g: 32,
    b: 0,
};
pub const RED: Rgb = Rgb {
    r: 64,
    g: 0,
    b: 0,
};

pub struct Inputs {
    /// RF polling enabled (`/nfc/rf`).
    pub rf_on: bool,
    /// A card is in the field.
    pub present: bool,
    /// A REST-initiated exchange is in flight, or finished moments ago.
    pub rest_active: bool,
    /// The PN532 never came up.
    pub init_failed: bool,
}

pub fn desired(i: &Inputs, now_ms: u64) -> Rgb {
    // 1 Hz blink for faults, ~3 Hz for traffic.
    if i.init_failed {
        return if (now_ms / 500) % 2 == 0 { RED } else { OFF };
    }
    if i.rest_active {
        return if (now_ms / 150) % 2 == 0 { AMBER } else { OFF };
    }
    if !i.rf_on {
        return OFF;
    }
    if i.present { GREEN } else { DIM_WHITE }
}

/// The state behind the blink, for `/nfc/status` and the log. On a board with
/// no addressable LED this is the only place the state machine is visible, and
/// unlike `name()` it does not flicker between the on and off half of a blink.
pub fn state_name(i: &Inputs) -> &'static str {
    if i.init_failed {
        "red-blink"
    } else if i.rest_active {
        "amber-blink"
    } else if !i.rf_on {
        "off"
    } else if i.present {
        "green"
    } else {
        "dim-white"
    }
}

