//! Monitor enumeration: work areas, DPI scale, refresh rate and a stable id.

use serde::Serialize;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromPoint, DEVMODEW, DISPLAY_DEVICEW,
    ENUM_CURRENT_SETTINGS, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, EDD_GET_DEVICE_INTERFACE_NAME, MONITORINFOF_PRIMARY};

use super::dock::Rect;

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// The monitor's device interface path. Stable across reboots and re-plugging,
    /// unlike "\\.\DISPLAY2", which Windows renumbers.
    pub id: String,
    /// Full monitor rectangle and the work area (without the taskbar), physical pixels.
    pub bounds: Rect,
    pub work: Rect,
    /// 1.0 at 100 % scaling.
    pub scale: f64,
    pub refresh_hz: u32,
    pub primary: bool,
}

/// What the settings window lists under Notch › Screen.
#[derive(Debug, Clone, Serialize)]
pub struct MonitorChoice {
    pub id: String,
    pub index: usize,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

fn rect(r: RECT) -> Rect {
    Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

fn wide_to_string(wide: &[u16]) -> String {
    let len = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..len])
}

fn describe(handle: HMONITOR) -> Option<Monitor> {
    unsafe {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return None;
        }
        let adapter = PCWSTR(info.szDevice.as_ptr());

        // The monitor attached to this adapter output; its DeviceID is the interface path.
        let mut device = DISPLAY_DEVICEW { cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32, ..Default::default() };
        let id = if EnumDisplayDevicesW(adapter, 0, &mut device, EDD_GET_DEVICE_INTERFACE_NAME).as_bool() {
            wide_to_string(&device.DeviceID)
        } else {
            String::new()
        };
        let id = if id.is_empty() { wide_to_string(&info.szDevice) } else { id };

        let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
        let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);

        let mut mode = DEVMODEW { dmSize: std::mem::size_of::<DEVMODEW>() as u16, ..Default::default() };
        let refresh_hz = if EnumDisplaySettingsW(adapter, ENUM_CURRENT_SETTINGS, &mut mode).as_bool() && mode.dmDisplayFrequency > 1 {
            mode.dmDisplayFrequency
        } else {
            60
        };

        Some(Monitor {
            id,
            bounds: rect(info.monitorInfo.rcMonitor),
            work: rect(info.monitorInfo.rcWork),
            scale: dpi_x as f64 / 96.0,
            refresh_hz: refresh_hz.clamp(30, 360),
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        })
    }
}

unsafe extern "system" fn collect(handle: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> windows::core::BOOL {
    let list = &mut *(data.0 as *mut Vec<Monitor>);
    list.extend(describe(handle));
    true.into()
}

/// All monitors, primary first, then left to right.
pub fn all() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut list as *mut Vec<Monitor> as isize));
    }
    list.sort_by_key(|m| (!m.primary, m.bounds.left, m.bounds.top));
    list
}

pub fn primary() -> Option<Monitor> {
    all().into_iter().find(|m| m.primary)
}

pub fn by_id(id: &str) -> Option<Monitor> {
    all().into_iter().find(|m| m.id == id)
}

/// Cursor position in physical screen pixels.
pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn at_point(x: i32, y: i32) -> Option<Monitor> {
    unsafe { describe(MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST)) }
}

pub fn choices() -> Vec<MonitorChoice> {
    all()
        .into_iter()
        .enumerate()
        .map(|(i, m)| MonitorChoice {
            id: m.id,
            index: i + 1,
            width: m.bounds.right - m.bounds.left,
            height: m.bounds.bottom - m.bounds.top,
            primary: m.primary,
        })
        .collect()
}
