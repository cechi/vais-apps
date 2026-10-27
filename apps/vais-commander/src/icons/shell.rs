//! The shell's file icons on Windows: `SHGetFileInfo` answers from the
//! extension alone (`SHGFI_USEFILEATTRIBUTES`), so no file is touched. The
//! icon's bitmaps are read back with GDI and encoded as a PNG for GPUI.

use std::ffi::c_void;
use std::mem::size_of;

use image::ImageEncoder as _;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits,
    GetObjectW, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::UI::Shell::{
    SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES, SHGetFileInfoW,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};
use windows::core::PCWSTR;

/// The large (32px) shell icon for a file name, as PNG bytes.
pub fn icon_png(name: &str) -> Option<Vec<u8>> {
    // Only the extension matters to the shell.
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let attributes = FILE_ATTRIBUTE_NORMAL;
    let flags = SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES;
    let mut info = SHFILEINFOW::default();
    // SAFETY: `wide` is NUL terminated and outlives the call; `info` is a
    // valid out-pointer of the size we pass. The icon handle we receive is
    // destroyed below.
    unsafe {
        let ok = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            attributes,
            Some(&mut info as *mut SHFILEINFOW),
            size_of::<SHFILEINFOW>() as u32,
            flags,
        );
        if ok == 0 || info.hIcon.is_invalid() {
            return None;
        }
        let png = icon_to_png(info.hIcon);
        let _ = DestroyIcon(info.hIcon);
        png
    }
}

unsafe fn icon_to_png(icon: HICON) -> Option<Vec<u8>> {
    let mut parts = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut parts) }.ok()?;
    let pixels = unsafe { icon_rgba(parts.hbmColor, parts.hbmMask) };
    for bitmap in [parts.hbmColor, parts.hbmMask] {
        if !bitmap.is_invalid() {
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        }
    }
    let (width, height, rgba) = pixels?;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .ok()?;
    Some(png)
}

/// RGBA pixels of an icon. Icons without an alpha channel use the mask
/// bitmap for transparency; icons without a colour bitmap are skipped.
unsafe fn icon_rgba(color: HBITMAP, mask: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if color.is_invalid() {
        return None;
    }
    let mut bitmap = BITMAP::default();
    let got = unsafe {
        GetObjectW(
            HGDIOBJ(color.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bitmap as *mut BITMAP as *mut c_void),
        )
    };
    if got == 0 || bitmap.bmWidth <= 0 || bitmap.bmHeight <= 0 {
        return None;
    }
    let (width, height) = (bitmap.bmWidth as u32, bitmap.bmHeight as u32);

    let screen = unsafe { GetDC(HWND(std::ptr::null_mut())) };
    let pixels = unsafe { read_bgra(screen, color, width, height) };
    let mask_pixels = if mask.is_invalid() {
        None
    } else {
        unsafe { read_bgra(screen, mask, width, height) }
    };
    unsafe { ReleaseDC(HWND(std::ptr::null_mut()), screen) };
    let pixels = pixels?;

    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] != 0);
    let mut rgba = pixels;
    for (i, p) in rgba.chunks_exact_mut(4).enumerate() {
        p.swap(0, 2);
        if !has_alpha {
            // Mask: white (any set channel) means transparent.
            p[3] = match &mask_pixels {
                Some(m) if m[i * 4] != 0 => 0,
                _ => 255,
            };
        }
    }
    Some((width, height, rgba))
}

/// Top-down 32-bit BGRA pixels of a bitmap.
unsafe fn read_bgra(dc: HDC, bitmap: HBITMAP, width: u32, height: u32) -> Option<Vec<u8>> {
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    (lines != 0).then_some(pixels)
}
