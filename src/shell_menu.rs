//! The Explorer context menu on Windows: the shell builds the menu for the
//! chosen files (`IContextMenu`, the one behind "Show more options"), it is
//! shown at the mouse pointer and the shell runs the command picked.

use std::cell::RefCell;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CMF_EXTENDEDVERBS, CMF_NORMAL, CMIC_MASK_CONTROL_DOWN, CMIC_MASK_PTINVOKE,
    CMIC_MASK_SHIFT_DOWN, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, DefSubclassProc,
    IContextMenu, IContextMenu2, IContextMenu3, ILFindLastID, ILFree, IShellFolder,
    RemoveWindowSubclass, SHBindToParent, SHGetDesktopFolder, SHParseDisplayName,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, SW_SHOWNORMAL, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{Interface, PCSTR, PCWSTR};

/// `CMIC_MASK_UNICODE`; the bindings only carry it as `SEE_MASK_UNICODE`.
const CMIC_MASK_UNICODE: u32 = 0x4000;
/// The command ids the shell may give its items; 0 is "nothing picked".
const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7fff;
const SUBCLASS_ID: usize = 0x7661_6973;

thread_local! {
    /// The menu on screen. Owner-drawn items and the submenus the shell fills
    /// in when they open ("Send to", "Open with") need the menu messages the
    /// owner window receives.
    static ACTIVE: RefCell<Option<IContextMenu>> = const { RefCell::new(None) };
}

/// The native handle of a GPUI window, the owner of the menu.
pub fn hwnd(window: &gpui_kit::Window) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Show the shell's menu for `items` (entries of one directory), or for the
/// background of `dir` when there are none, and run what the user picks.
/// The menu runs a modal message loop, so this must not be called while the
/// app is borrowed (inside an update); GPUI's window procedure needs it.
pub fn show(hwnd: isize, dir: &Path, items: &[PathBuf]) -> Result<()> {
    if items.iter().any(|p| p.parent() != items[0].parent()) {
        bail!("the entries are in different folders");
    }
    let hwnd = HWND(hwnd as *mut _);
    // SAFETY: COM is initialised on this (the UI) thread by GPUI; every
    // pointer handed to the shell outlives the call that uses it.
    unsafe {
        let menu = context_menu(hwnd, dir, items)?;
        let popup = CreatePopupMenu()?;
        let mut flags = CMF_NORMAL;
        if key_down(VK_SHIFT.0) {
            flags |= CMF_EXTENDEDVERBS;
        }
        let result = menu
            .QueryContextMenu(popup, 0, FIRST_COMMAND, LAST_COMMAND, flags)
            .map_err(anyhow::Error::from)
            .and_then(|()| track(hwnd, &menu, popup, dir));
        let _ = DestroyMenu(popup);
        result
    }
}

/// An absolute item id list; freed on drop.
struct Pidl(*mut ITEMIDLIST);

impl Drop for Pidl {
    fn drop(&mut self) {
        // SAFETY: allocated by SHParseDisplayName and freed only here.
        unsafe { ILFree(Some(self.0)) };
    }
}

unsafe fn parse(path: &Path) -> Result<Pidl> {
    let wide = wide(path);
    let mut pidl = std::ptr::null_mut();
    unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>, &mut pidl, 0, None)? };
    Ok(Pidl(pidl))
}

unsafe fn context_menu(hwnd: HWND, dir: &Path, items: &[PathBuf]) -> Result<IContextMenu> {
    if items.is_empty() {
        let folder = unsafe { parse(dir)? };
        let desktop = unsafe { SHGetDesktopFolder()? };
        let folder: IShellFolder = unsafe { desktop.BindToObject(folder.0, None::<&IBindCtx>)? };
        return Ok(unsafe { folder.CreateViewObject(hwnd)? });
    }
    let pidls = items
        .iter()
        .map(|p| unsafe { parse(p) })
        .collect::<Result<Vec<_>>>()?;
    let parent: IShellFolder = unsafe { SHBindToParent(pidls[0].0, None)? };
    let children: Vec<*const ITEMIDLIST> = pidls
        .iter()
        .map(|p| unsafe { ILFindLastID(p.0) } as *const ITEMIDLIST)
        .collect();
    Ok(unsafe { parent.GetUIObjectOf(hwnd, &children, None)? })
}

unsafe fn track(hwnd: HWND, menu: &IContextMenu, popup: HMENU, dir: &Path) -> Result<()> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point)? };
    ACTIVE.with(|active| *active.borrow_mut() = Some(menu.clone()));
    let subclassed =
        unsafe { SetWindowSubclass(hwnd, Some(forward_menu_messages), SUBCLASS_ID, 0) }.as_bool();
    let flags = TPM_RETURNCMD.0 | TPM_RIGHTBUTTON.0;
    let command = unsafe { TrackPopupMenuEx(popup, flags, point.x, point.y, hwnd, None) }.0 as u32;
    if subclassed {
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(forward_menu_messages), SUBCLASS_ID) };
    }
    ACTIVE.with(|active| active.borrow_mut().take());
    if command < FIRST_COMMAND {
        return Ok(());
    }
    // MAKEINTRESOURCE: the command's offset stands in for a verb string.
    let verb = (command - FIRST_COMMAND) as usize;
    let directory = wide(dir);
    let mut mask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
    if key_down(VK_SHIFT.0) {
        mask |= CMIC_MASK_SHIFT_DOWN;
    }
    if key_down(VK_CONTROL.0) {
        mask |= CMIC_MASK_CONTROL_DOWN;
    }
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: mask,
        hwnd,
        lpVerb: PCSTR(verb as *const u8),
        lpVerbW: PCWSTR(verb as *const u16),
        lpDirectoryW: PCWSTR(directory.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: point,
        ..Default::default()
    };
    let info = &info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO;
    unsafe { menu.InvokeCommand(info)? };
    Ok(())
}

/// Passes the menu messages of the owner window to the shell's menu while
/// it is on screen.
unsafe extern "system" fn forward_menu_messages(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if matches!(
        message,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        // A clone, not a borrow: the shell may pump messages meanwhile.
        let menu = ACTIVE.with(|active| active.borrow().clone());
        if let Some(menu) = menu {
            if let Ok(menu) = menu.cast::<IContextMenu3>() {
                let mut result = LRESULT(0);
                if unsafe { menu.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }
                    .is_ok()
                {
                    return result;
                }
            } else if let Ok(menu) = menu.cast::<IContextMenu2>() {
                if unsafe { menu.HandleMenuMsg(message, wparam, lparam) }.is_ok() {
                    return LRESULT(0);
                }
            }
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn key_down(key: u16) -> bool {
    // SAFETY: no pointers involved.
    (unsafe { GetKeyState(key as i32) }) < 0
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
