//! Small Win32 helpers that do not belong to a feature module.

pub mod autostart;
pub mod conpty;
pub mod dpapi;
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

/// The user's Downloads folder.
pub fn downloads_dir() -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None).ok()?;
        let text = path.to_string().ok();
        CoTaskMemFree(Some(path.0 as *const _));
        text.map(std::path::PathBuf::from)
    }
}

/// The Windows build number (19045 on Windows 10 22H2, 22000+ on Windows 11).
pub fn os_build() -> u32 {
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let mut buffer = [0u16; 16];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            w!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion"),
            w!("CurrentBuildNumber"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };
    if read.is_err() {
        return 0;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len]).trim().parse().unwrap_or(0)
}
