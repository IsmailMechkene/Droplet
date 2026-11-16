//! "Start with Windows": one value under HKCU\Software\Microsoft\Windows\CurrentVersion\Run.
//! Per-user, no admin rights, and nothing else in the registry is touched.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
};

use super::wide;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Droplet";

/// Adds or removes the Run entry for the current executable.
/// Debug builds do nothing, so a development build never registers itself.
pub fn set(enabled: bool) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("\"{}\"", exe.display());
    let (key_path, name) = (wide(RUN_KEY), wide(VALUE_NAME));
    unsafe {
        let mut key = HKEY::default();
        let opened = RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key_path.as_ptr()), None, KEY_SET_VALUE, &mut key);
        if opened != ERROR_SUCCESS {
            return Err(format!("could not open the Run key ({})", opened.0));
        }
        let result = if enabled {
            let data = wide(&command);
            let bytes = std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2);
            RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, PCWSTR(name.as_ptr()))
        };
        let _ = RegCloseKey(key);
        // Deleting a value that is not there is fine.
        if result == ERROR_SUCCESS || !enabled {
            Ok(())
        } else {
            Err(format!("could not update the Run key ({})", result.0))
        }
    }
}
