//! Small Win32 helpers that do not belong to a feature module.

pub mod autostart;
pub mod shell;

use windows::Win32::Globalization::GetUserDefaultLocaleName;
use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

/// Null-terminated UTF-16 for Win32 calls.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The user's Windows display locale, like "fr-FR".
pub fn system_locale() -> String {
    let mut buffer = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return "en-US".into();
    }
    String::from_utf16_lossy(&buffer[..len as usize - 1])
}

/// Local time minus UTC, in seconds, including daylight saving when active.
pub fn utc_offset() -> i64 {
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;
    let mut tz = TIME_ZONE_INFORMATION::default();
    let kind = unsafe { GetTimeZoneInformation(&mut tz) };
    let extra = if kind == TIME_ZONE_ID_DAYLIGHT { tz.DaylightBias } else { tz.StandardBias };
    -((tz.Bias + extra) as i64) * 60
}
