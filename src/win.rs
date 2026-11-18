use std::{
    ffi::OsStr,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};

use windows_sys::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::{LibraryLoader::GetModuleHandleW, Registry::*},
    UI::{
        Controls::Dialogs::{
            GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST,
            OPENFILENAMEW,
        },
        HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
        WindowsAndMessaging::*,
    },
};

use crate::model::Result;

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn wide_os(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

pub fn instance() -> windows_sys::Win32::Foundation::HINSTANCE {
    unsafe { GetModuleHandleW(null()) }
}

pub fn alert(owner: HWND, title: &str, text: &str) {
    unsafe {
        MessageBoxW(
            owner,
            wide(text).as_ptr(),
            wide(title).as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

pub fn confirm(owner: HWND, title: &str, text: &str) -> bool {
    unsafe {
        MessageBoxW(
            owner,
            wide(text).as_ptr(),
            wide(title).as_ptr(),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        ) == IDYES
    }
}

pub fn pick_file(owner: HWND) -> Option<PathBuf> {
    let mut file = vec![0_u16; 32_768];
    let filter = wide(
        "课表、日历和备份 (*.csv;*.ics;*.html;*.json)\0*.csv;*.ics;*.html;*.htm;*.json\0所有支持的文件 (*.csv;*.ics;*.html;*.json)\0*.csv;*.ics;*.html;*.htm;*.json\0",
    );
    let options = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file.as_mut_ptr(),
        nMaxFile: file.len() as u32,
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };
    let mut options = options;
    if unsafe { GetOpenFileNameW(&mut options) } == 0 {
        return None;
    }
    let length = file.iter().position(|value| *value == 0)?;
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &file[..length],
    )))
}

pub fn pick_backup(owner: HWND) -> Result<PathBuf> {
    let path = pick_file(owner).ok_or("未选择备份文件")?;
    if path.extension().is_none_or(|ext| ext != "json") {
        return Err("请选择 JSON 备份文件".into());
    }
    Ok(path)
}

pub fn work_area() -> (RECT, i32) {
    let mut cursor = POINT::default();
    unsafe {
        GetCursorPos(&mut cursor);
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let mut dpi = 96;
    let mut y = 96;
    unsafe {
        GetMonitorInfoW(monitor, &mut info);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut y);
    }
    (info.rcWork, dpi.max(96) as i32)
}

pub fn dark_mode() -> bool {
    let mut value = 1_u32;
    let mut size = 4_u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        );
    }
    value == 0
}

pub fn round_region(window: HWND, radius: i32) {
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(window, &mut rect);
        let region = CreateRoundRectRgn(
            0,
            0,
            rect.right - rect.left + 1,
            rect.bottom - rect.top + 1,
            radius * 2,
            radius * 2,
        );
        if !region.is_null() {
            SetWindowRgn(window, region, 1);
        }
    }
}

pub fn frame_theme(window: HWND, dark: bool, rounded: bool) {
    let flag = i32::from(dark);
    let (caption, border, ink): (u32, u32, u32) = if dark {
        (0x23_20_1a_u32, 0x44_40_35, 0xf3_f3ed)
    } else {
        (0xe9_f1_ec, 0xe0_e5df, 0x1f_2118)
    };
    unsafe {
        DwmSetWindowAttribute(window, 20, (&flag as *const i32).cast(), 4);
        if rounded {
            let corner = 2_u32;
            DwmSetWindowAttribute(window, 33, (&corner as *const u32).cast(), 4);
        }
        // CAPTION_COLOR / BORDER_COLOR / TEXT_COLOR (Windows 11 22000+); ignored elsewhere.
        DwmSetWindowAttribute(window, 35, (&caption as *const u32).cast(), 4);
        DwmSetWindowAttribute(window, 34, (&border as *const u32).cast(), 4);
        DwmSetWindowAttribute(window, 36, (&ink as *const u32).cast(), 4);
    }
}

pub fn startup(exe: &Path, set: Option<bool>) -> Result<bool> {
    let key = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = wide("CourseClock");
    let command: Vec<u16> = [
        vec![b'"' as u16],
        {
            let mut path = wide_os(exe.as_os_str());
            path.pop();
            path
        },
        wide("\" --tray"),
    ]
    .concat();
    let mut open = null_mut();
    let rights = if set.is_some() {
        KEY_QUERY_VALUE | KEY_SET_VALUE
    } else {
        KEY_QUERY_VALUE
    };
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, rights, &mut open) };
    if status != 0 {
        return Err(format!("无法访问登录启动设置（{status}）"));
    }
    let result = match set {
        Some(true) => unsafe {
            RegSetValueExW(
                open,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        },
        Some(false) => unsafe { RegDeleteValueW(open, name.as_ptr()) },
        None => {
            let mut current = vec![0_u16; 32_768];
            let mut size = (current.len() * 2) as u32;
            let mut kind = 0;
            let status = unsafe {
                RegQueryValueExW(
                    open,
                    name.as_ptr(),
                    null(),
                    &mut kind,
                    current.as_mut_ptr().cast(),
                    &mut size,
                )
            };
            unsafe {
                RegCloseKey(open);
            }
            return Ok(status == 0
                && kind == REG_SZ
                && current.get(..size as usize / 2) == Some(&command));
        }
    };
    unsafe {
        RegCloseKey(open);
    }
    if result == 0 || (set == Some(false) && result == 2) {
        Ok(set == Some(true))
    } else {
        Err(format!("无法更新登录启动设置（{result}）"))
    }
}

pub fn icon() -> HICON {
    let size = 32;
    let mut bits = null_mut();
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let bitmap = CreateDIBSection(
            null_mut(),
            &header,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        );
        if bitmap.is_null() || bits.is_null() {
            return LoadIconW(null_mut(), IDI_APPLICATION);
        }
        let pixels = std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize);
        for y in 0..size {
            for x in 0..size {
                let dx = (x - 15).abs();
                let dy = (y - 15).abs();
                if (dx - 7).max(0).pow(2) + (dy - 7).max(0).pow(2) <= 8_i32.pow(2) {
                    let radius = (x - 15).pow(2) + (y - 15).pow(2);
                    let clock = (64..=104).contains(&radius)
                        || ((14..=16).contains(&x) && (9..=15).contains(&y))
                        || ((15..=20).contains(&x) && (14..=16).contains(&y));
                    pixels[(y * size + x) as usize] = if clock { 0xffe0fff0 } else { 0xff146451 };
                }
            }
        }
        let mask_bits = [0_u8; 128];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr().cast());
        let source = ICONINFO {
            fIcon: 1,
            hbmColor: bitmap,
            hbmMask: mask,
            ..Default::default()
        };
        let icon = CreateIconIndirect(&source);
        DeleteObject(bitmap);
        DeleteObject(mask);
        icon
    }
}
