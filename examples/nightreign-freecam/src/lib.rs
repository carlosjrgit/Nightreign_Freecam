use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use fromsoftware_shared::{F32Matrix4x4, F32Vector4, FromStatic};
use nalgebra_glm as glm;
use nightreign::cs::{CSCam, CSCamera};
use windows::core::PCSTR;
use windows::Win32::Foundation::{BOOL, HINSTANCE, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

extern "system" {
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn ClipCursor(lpRect: *const std::ffi::c_void) -> BOOL;
}

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct XINPUT_GAMEPAD {
    pub w_buttons: u16,
    pub b_left_trigger: u8,
    pub b_right_trigger: u8,
    pub s_thumb_lx: i16,
    pub s_thumb_ly: i16,
    pub s_thumb_rx: i16,
    pub s_thumb_ry: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct XINPUT_STATE {
    pub dw_packet_number: u32,
    pub gamepad: XINPUT_GAMEPAD,
}

pub type FnXInputGetState = unsafe extern "system" fn(u32, *mut XINPUT_STATE) -> u32;

static FREECAM_ACTIVE: AtomicBool = AtomicBool::new(false);
static ORIGINAL_XINPUT_GET_STATE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_SET_CURSOR_POS: AtomicUsize = AtomicUsize::new(0);

static OVERLAY_HWND: AtomicUsize = AtomicUsize::new(0);
static OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(true);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PAINTSTRUCT {
    hdc: isize,
    f_erase: BOOL,
    rc_paint: RECT,
    f_restore: BOOL,
    f_inc_update: BOOL,
    rgb_reserved: [u8; 32],
}

#[repr(C)]
struct MSG {
    hwnd: isize,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
    l_private: u32,
}

#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: Option<unsafe extern "system" fn(isize, u32, usize, isize) -> isize>,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: isize,
    h_icon: isize,
    h_cursor: isize,
    h_br_background: isize,
    lpsz_menu_name: *const u16,
    lpsz_class_name: *const u16,
    h_icon_sm: isize,
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(lpwcx: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        dwExStyle: u32,
        lpClassName: *const u16,
        lpWindowName: *const u16,
        dwStyle: u32,
        X: i32,
        Y: i32,
        nWidth: i32,
        nHeight: i32,
        hWndParent: isize,
        hMenu: isize,
        hInstance: isize,
        lpParam: *mut std::ffi::c_void,
    ) -> isize;
    fn DefWindowProcW(hWnd: isize, Msg: u32, wParam: usize, lParam: isize) -> isize;
    fn ShowWindow(hWnd: isize, nCmdShow: i32) -> BOOL;
    fn UpdateWindow(hWnd: isize) -> BOOL;
    fn SetLayeredWindowAttributes(hWnd: isize, crKey: u32, bAlpha: u8, dwFlags: u32) -> BOOL;
    fn BeginPaint(hWnd: isize, lpPaint: *mut PAINTSTRUCT) -> isize;
    fn EndPaint(hWnd: isize, lpPaint: *const PAINTSTRUCT) -> BOOL;
    fn GetMessageW(lpMsg: *mut MSG, hWnd: isize, wMsgFilterMin: u32, wMsgFilterMax: u32) -> BOOL;
    fn TranslateMessage(lpMsg: *const MSG) -> BOOL;
    fn DispatchMessageW(lpMsg: *const MSG) -> isize;
    fn FillRect(hDC: isize, lprc: *const RECT, hbr: isize) -> i32;
    fn FrameRect(hDC: isize, lprc: *const RECT, hbr: isize) -> i32;
    fn DrawTextW(hDC: isize, lpchText: *const u16, cchText: i32, lprc: *mut RECT, format: u32) -> i32;
    fn SetWindowPos(hWnd: isize, hWndInsertAfter: isize, X: i32, Y: i32, cx: i32, cy: i32, uFlags: u32) -> BOOL;
    fn EnumWindows(lpEnumFunc: Option<unsafe extern "system" fn(isize, isize) -> BOOL>, lParam: isize) -> BOOL;
    fn GetWindowThreadProcessId(hWnd: isize, lpdwProcessId: *mut u32) -> u32;
    fn IsWindowVisible(hWnd: isize) -> BOOL;
    fn DestroyWindow(hWnd: isize) -> BOOL;
    fn PostQuitMessage(nExitCode: i32);
    fn SetTimer(hWnd: isize, nIDEvent: usize, uElapse: u32, lpTimerFunc: Option<unsafe extern "system" fn(isize, u32, usize, u32)>) -> usize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateSolidBrush(color: u32) -> isize;
    fn SelectObject(hdc: isize, hgdiobj: isize) -> isize;
    fn DeleteObject(ho: isize) -> BOOL;
    fn SetBkMode(hdc: isize, mode: i32) -> i32;
    fn SetTextColor(hdc: isize, color: u32) -> u32;
    fn CreateFontW(
        cHeight: i32,
        cWidth: i32,
        cEscapement: i32,
        cOrientation: i32,
        cWeight: i32,
        bItalic: u32,
        bUnderline: u32,
        bStrikeOut: u32,
        iCharSet: u32,
        iOutPrecision: u32,
        iClipPrecision: u32,
        iQuality: u32,
        iPitchAndFamily: u32,
        pszFaceName: *const u16,
    ) -> isize;
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

fn to_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: isize,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    match msg {
        0x0014 => 1,
        0x000F => {
            let mut ps = std::mem::zeroed::<PAINTSTRUCT>();
            let hdc = BeginPaint(hwnd, &mut ps);
            if hdc != 0 {
                render_overlay_ui(hdc);
                EndPaint(hwnd, &ps);
            }
            0
        }
        0x0113 => {
            let my_pid = GetCurrentProcessId();

            unsafe extern "system" fn check_game_window(w: isize, lparam: isize) -> BOOL {
                let mut pid = 0u32;
                GetWindowThreadProcessId(w, &mut pid);
                let my_pid = *(lparam as *const u32);
                if pid == my_pid && w != (OVERLAY_HWND.load(Ordering::Relaxed) as isize) {
                    if IsWindowVisible(w).0 != 0 {
                        let found_ptr = (lparam as *mut u32).add(1) as *mut bool;
                        *found_ptr = true;
                        return BOOL(0);
                    }
                }
                BOOL(1)
            }

            let mut ctx: [u32; 2] = [my_pid, 0];
            EnumWindows(Some(check_game_window), ctx.as_mut_ptr() as isize);
            if ctx[1] == 0 {
                DestroyWindow(hwnd);
                PostQuitMessage(0);
            }
            0
        }
        0x0002 => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn render_overlay_ui(hdc: isize) {
    let full_rc = RECT { left: 0, top: 0, right: 330, bottom: 370 };
    let bg_brush = CreateSolidBrush(rgb(16, 18, 24));
    FillRect(hdc, &full_rc, bg_brush);
    DeleteObject(bg_brush);

    let border_brush = CreateSolidBrush(rgb(55, 65, 85));
    FrameRect(hdc, &full_rc, border_brush);
    DeleteObject(border_brush);

    SetBkMode(hdc, 1);

    let segoe = to_wide_null("Segoe UI");
    let font_title = CreateFontW(19, 0, 0, 0, 700, 0, 0, 0, 1, 0, 0, 5, 0, segoe.as_ptr());
    let font_sub = CreateFontW(13, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, segoe.as_ptr());
    let font_bold = CreateFontW(14, 0, 0, 0, 600, 0, 0, 0, 1, 0, 0, 5, 0, segoe.as_ptr());
    let font_norm = CreateFontW(14, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, segoe.as_ptr());

    SelectObject(hdc, font_title);
    SetTextColor(hdc, rgb(240, 195, 75));
    let title_w = to_wide_null("FREECAM GUIDE");
    let mut rc_title = RECT { left: 16, top: 12, right: 314, bottom: 32 };
    DrawTextW(hdc, title_w.as_ptr(), (title_w.len() - 1) as i32, &mut rc_title, 0x00000000 | 0x00000020);

    SelectObject(hdc, font_sub);
    SetTextColor(hdc, rgb(150, 160, 175));
    let sub_w = to_wide_null("[H] or [F2] to Toggle Guide");
    let mut rc_sub = RECT { left: 16, top: 32, right: 314, bottom: 48 };
    DrawTextW(hdc, sub_w.as_ptr(), (sub_w.len() - 1) as i32, &mut rc_sub, 0x00000000 | 0x00000020);

    let div_brush = CreateSolidBrush(rgb(45, 55, 72));
    let div1_rc = RECT { left: 16, top: 52, right: 314, bottom: 53 };
    FillRect(hdc, &div1_rc, div_brush);

    let items = [
        ("P / F1", "Toggle Freecam"),
        ("W, A, S, D", "Move Camera"),
        ("Space / Ctrl", "Fly Up / Down"),
        ("Shift / Alt", "Boost / Slow Speed"),
        ("Mouse", "Look Around (360)"),
        ("Q / E  (R)", "Roll Camera (Reset)"),
        ("T", "Teleport Character"),
        ("Home", "Rescue to Origin"),
    ];

    let mut y = 60;
    for (key, desc) in items {
        SelectObject(hdc, font_bold);
        SetTextColor(hdc, rgb(225, 230, 240));
        let key_w = to_wide_null(key);
        let mut rc_k = RECT { left: 16, top: y, right: 135, bottom: y + 24 };
        DrawTextW(hdc, key_w.as_ptr(), (key_w.len() - 1) as i32, &mut rc_k, 0x00000000 | 0x00000020 | 0x00000004);

        SelectObject(hdc, font_norm);
        SetTextColor(hdc, rgb(165, 175, 195));
        let desc_w = to_wide_null(desc);
        let mut rc_d = RECT { left: 135, top: y, right: 314, bottom: y + 24 };
        DrawTextW(hdc, desc_w.as_ptr(), (desc_w.len() - 1) as i32, &mut rc_d, 0x00000000 | 0x00000020 | 0x00000004);

        y += 32;
    }

    let div2_rc = RECT { left: 16, top: y + 4, right: 314, bottom: y + 5 };
    FillRect(hdc, &div2_rc, div_brush);
    DeleteObject(div_brush);

    SelectObject(hdc, font_sub);
    SetTextColor(hdc, rgb(80, 200, 150));
    let footer_w = to_wide_null("Keyboard & Mouse Mode Active");
    let mut rc_footer = RECT { left: 16, top: y + 10, right: 314, bottom: y + 28 };
    DrawTextW(hdc, footer_w.as_ptr(), (footer_w.len() - 1) as i32, &mut rc_footer, 0x00000000 | 0x00000020 | 0x00000004);

    DeleteObject(font_title);
    DeleteObject(font_sub);
    DeleteObject(font_bold);
    DeleteObject(font_norm);
}

fn start_overlay_thread() {
    std::thread::spawn(|| {
        unsafe {
            let class_name = to_wide_null("NightreignFreecamOSD");
            let title = to_wide_null("Freecam Guide");

            let mut wc = std::mem::zeroed::<WNDCLASSEXW>();
            wc.cb_size = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wc.style = 0x0003;
            wc.lpfn_wnd_proc = Some(overlay_wnd_proc);
            wc.lpsz_class_name = class_name.as_ptr();

            RegisterClassExW(&wc);

            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let overlay_w = 330;
            let overlay_h = 370;
            let x = if screen_w > (overlay_w + 30) {
                screen_w - overlay_w - 24
            } else {
                24
            };
            let y = 24;

            let ex_style = 0x00000008 | 0x00080000 | 0x00000020 | 0x08000000 | 0x00000080;
            let style = 0x80000000;

            let hwnd = CreateWindowExW(
                ex_style,
                class_name.as_ptr(),
                title.as_ptr(),
                style,
                x,
                y,
                overlay_w,
                overlay_h,
                0,
                0,
                0,
                std::ptr::null_mut(),
            );

            if hwnd == 0 {
                return;
            }

            SetLayeredWindowAttributes(hwnd, 0, 215, 0x00000002);
            OVERLAY_HWND.store(hwnd as usize, Ordering::SeqCst);

            SetTimer(hwnd, 1, 250, None);

            ShowWindow(hwnd, 4);
            UpdateWindow(hwnd);

            let mut msg = std::mem::zeroed::<MSG>();
            while GetMessageW(&mut msg, 0, 0, 0).0 > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

fn log_msg(msg: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("freecam.log")
    {
        let now = std::time::SystemTime::now();
        let _ = writeln!(f, "[{:?}] {}", now, msg);
    }
}

fn is_valid_ptr<T>(ptr: *const T) -> bool {
    if ptr.is_null() {
        return false;
    }
    let addr = ptr as usize;
    addr >= 0x10000 && addr <= 0x0000_7fff_ffff_ffff
}

fn is_key_down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16 & 0x8000) != 0 }
}

fn is_key_pressed(vk: i32, was_down: &mut bool) -> bool {
    let down = is_key_down(vk);
    let pressed = down && !*was_down;
    *was_down = down;
    pressed
}

unsafe extern "system" fn hooked_xinput_get_state(
    dw_user_index: u32,
    p_state: *mut XINPUT_STATE,
) -> u32 {
    let orig = ORIGINAL_XINPUT_GET_STATE.load(Ordering::Relaxed);
    if orig == 0 {
        return 1167;
    }
    let orig_fn: FnXInputGetState = std::mem::transmute(orig);
    let res = orig_fn(dw_user_index, p_state);
    if res != 0 || p_state.is_null() {
        return res;
    }

    if FREECAM_ACTIVE.load(Ordering::Relaxed) {
        (*p_state).gamepad.w_buttons = 0;
        (*p_state).gamepad.b_left_trigger = 0;
        (*p_state).gamepad.b_right_trigger = 0;
        (*p_state).gamepad.s_thumb_lx = 0;
        (*p_state).gamepad.s_thumb_ly = 0;
        (*p_state).gamepad.s_thumb_rx = 0;
        (*p_state).gamepad.s_thumb_ry = 0;
    }

    res
}

unsafe extern "system" fn hooked_set_cursor_pos(x: i32, y: i32) -> BOOL {
    if FREECAM_ACTIVE.load(Ordering::Relaxed) {
        return BOOL(1);
    }

    let orig = ORIGINAL_SET_CURSOR_POS.load(Ordering::Relaxed);
    if orig != 0 {
        let orig_fn: unsafe extern "system" fn(i32, i32) -> BOOL = std::mem::transmute(orig);
        orig_fn(x, y)
    } else {
        BOOL(1)
    }
}

fn real_set_cursor_pos(x: i32, y: i32) {
    let orig = ORIGINAL_SET_CURSOR_POS.load(Ordering::Relaxed);
    if orig != 0 {
        let orig_fn: unsafe extern "system" fn(i32, i32) -> BOOL = unsafe { std::mem::transmute(orig) };
        unsafe {
            let _ = orig_fn(x, y);
        }
    } else {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y);
        }
    }
}

unsafe fn install_iat_hooks() {
    let base = match GetModuleHandleA(PCSTR(std::ptr::null())) {
        Ok(h) => h.0 as usize,
        Err(_) => return,
    };

    let xinput_slot = (base + 0xD8FC9C) as *mut usize;
    if is_valid_ptr(xinput_slot) {
        let cur = *xinput_slot;
        if cur != 0 && cur != (hooked_xinput_get_state as usize) {
            ORIGINAL_XINPUT_GET_STATE.store(cur, Ordering::SeqCst);
            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
            if VirtualProtect(xinput_slot as *const _, 8, PAGE_READWRITE, &mut old_protect).is_ok() {
                *xinput_slot = hooked_xinput_get_state as usize;
                let mut dummy = PAGE_PROTECTION_FLAGS(0);
                let _ = VirtualProtect(xinput_slot as *const _, 8, old_protect, &mut dummy);
            }
        }
    }

    let cursor_slot = (base + 0xD8F93C) as *mut usize;
    if is_valid_ptr(cursor_slot) {
        let cur = *cursor_slot;
        if cur != 0 && cur != (hooked_set_cursor_pos as usize) {
            ORIGINAL_SET_CURSOR_POS.store(cur, Ordering::SeqCst);
            let mut old_protect = PAGE_PROTECTION_FLAGS(0);
            if VirtualProtect(cursor_slot as *const _, 8, PAGE_READWRITE, &mut old_protect).is_ok() {
                *cursor_slot = hooked_set_cursor_pos as usize;
                let mut dummy = PAGE_PROTECTION_FLAGS(0);
                let _ = VirtualProtect(cursor_slot as *const _, 8, old_protect, &mut dummy);
            }
        }
    }
}

// =========================================================================
// CONGELAMENTO DO JOGADOR (PLAYER FREEZE)
// =========================================================================

fn get_player_physics_ptr() -> Option<*mut u8> {
    if let Ok(wcm) = unsafe { <nightreign::cs::WorldChrMan as FromStatic>::instance() } {
        let wcm_ptr = wcm as *mut nightreign::cs::WorldChrMan as *mut u8;
        if is_valid_ptr(wcm_ptr) {
            let player_ptr = unsafe { *(wcm_ptr.add(0x174e8) as *const *mut u8) };
            if is_valid_ptr(player_ptr) {
                let modules_ptr = unsafe { *(player_ptr.add(0x1b8) as *const *mut u8) };
                if is_valid_ptr(modules_ptr) {
                    let physics_ptr = unsafe { *(modules_ptr.add(0x68) as *const *mut u8) };
                    if is_valid_ptr(physics_ptr) {
                        return Some(physics_ptr);
                    }
                }
            }
        }
    }

    unsafe {
        if let Ok(hmod) = GetModuleHandleA(PCSTR(std::ptr::null())) {
            let base = hmod.0 as usize;
            let wcm_global = (base + 0x3B04378) as *const *mut u8;
            if is_valid_ptr(wcm_global) {
                let wcm = *wcm_global;
                if is_valid_ptr(wcm) {
                    let player = *(wcm.add(0x174e8) as *const *mut u8);
                    if is_valid_ptr(player) {
                        let modules = *(player.add(0x1b8) as *const *mut u8);
                        if is_valid_ptr(modules) {
                            let physics = *(modules.add(0x68) as *const *mut u8);
                            if is_valid_ptr(physics) {
                                return Some(physics);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn get_player_coords_ptr() -> Option<*mut [f32; 3]> {
    get_player_physics_ptr().map(|p| unsafe { p.add(0x70) as *mut [f32; 3] })
}

fn teleport_player_clean(target: [f32; 3]) {
    if let Some(phys) = get_player_physics_ptr() {
        unsafe {
            let pos = phys.add(0x70) as *mut [f32; 4];
            let last_pos = phys.add(0x80) as *mut [f32; 4];
            *pos = [target[0], target[1], target[2], 1.0];
            *last_pos = [target[0], target[1], target[2], 1.0];
            *phys.add(0x91) = 1;
            *(phys.add(0xd0) as *mut [f32; 4]) = [0.0, 0.0, 0.0, 0.0];
            *(phys.add(0xe0) as *mut [f32; 4]) = [0.0, 0.0, 0.0, 0.0];
        }
    } else if let Some(coords_ptr) = get_player_coords_ptr() {
        unsafe {
            *coords_ptr = target;
        }
    }
}

fn get_player_no_dead_ptr() -> Option<*mut u8> {
    if let Ok(wcm) = unsafe { <nightreign::cs::WorldChrMan as FromStatic>::instance() } {
        let wcm_ptr = wcm as *mut nightreign::cs::WorldChrMan as *mut u8;
        if is_valid_ptr(wcm_ptr) {
            let player_ptr = unsafe { *(wcm_ptr.add(0x174e8) as *const *mut u8) };
            if is_valid_ptr(player_ptr) {
                let modules_ptr = unsafe { *(player_ptr.add(0x1b8) as *const *mut u8) };
                if is_valid_ptr(modules_ptr) {
                    let data_ptr = unsafe { *(modules_ptr.add(0x00) as *const *mut u8) };
                    if is_valid_ptr(data_ptr) {
                        return Some(unsafe { data_ptr.add(0x189) });
                    }
                }
            }
        }
    }

    unsafe {
        if let Ok(hmod) = GetModuleHandleA(PCSTR(std::ptr::null())) {
            let base = hmod.0 as usize;
            let wcm_global = (base + 0x3B04378) as *const *mut u8;
            if is_valid_ptr(wcm_global) {
                let wcm = *wcm_global;
                if is_valid_ptr(wcm) {
                    let player = *(wcm.add(0x174e8) as *const *mut u8);
                    if is_valid_ptr(player) {
                        let modules = *(player.add(0x1b8) as *const *mut u8);
                        if is_valid_ptr(modules) {
                            let data = *(modules.add(0x00) as *const *mut u8);
                            if is_valid_ptr(data) {
                                return Some(data.add(0x189));
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

fn set_player_no_dead(enable: bool) {
    if let Some(ptr) = get_player_no_dead_ptr() {
        unsafe {
            if enable {
                *ptr |= 0x02;
            } else {
                *ptr &= !0x02;
            }
        }
    }
}

// RPC exports for gui.exe compatibility

#[no_mangle]
pub unsafe extern "C" fn initialize(ctx: *mut u8) -> u32 {
    if !ctx.is_null() {
        *ctx.add(0x10) = 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn get_agent_state(ctx: *mut u8) -> u32 {
    if !ctx.is_null() {
        *ctx.add(0x10) = 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn poll_events(ctx: *mut u8) -> u32 {
    if !ctx.is_null() {
        *ctx.add(0x10) = 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn post_event(ctx: *mut u8) -> u32 {
    if !ctx.is_null() {
        *ctx.add(0x10) = 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn snapshot_camera_state(ctx: *mut u8) -> u32 {
    if !ctx.is_null() {
        *ctx.add(0x10) = 1;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn DllMain(_hmodule: HINSTANCE, reason: u32) -> bool {
    if reason == 0 {
        let hwnd = OVERLAY_HWND.load(Ordering::SeqCst);
        if hwnd != 0 {
            DestroyWindow(hwnd as isize);
            PostQuitMessage(0);
        }
        return true;
    }
    if reason != 1 {
        return true;
    }

    log_msg("Nightreign freecam module loaded.");

    install_iat_hooks();
    start_overlay_thread();

    std::thread::spawn(|| {
        let res = std::panic::catch_unwind(|| {
            run_freecam_loop();
        });
        if let Err(e) = res {
            log_msg(&format!("Error in freecam loop: {:?}", e));
        }
    });

    true
}

fn update_all_cameras(camera: &mut CSCamera, matrix: F32Matrix4x4, fov: f32) {
    let base = camera as *mut CSCamera as *mut *mut CSCam;
    for i in 0..4 {
        let cam_ptr = unsafe { *base.add(i) };
        if is_valid_ptr(cam_ptr) {
            unsafe {
                (*cam_ptr).matrix = matrix;
                (*cam_ptr).fov = fov;
            }
        }
    }
}

fn get_primary_camera(camera: &CSCamera) -> Option<*mut CSCam> {
    let base = camera as *const CSCamera as *const *mut CSCam;
    let p0 = unsafe { *base };
    if is_valid_ptr(p0) {
        return Some(p0);
    }
    None
}

fn run_freecam_loop() {
    let mut p_key_down = false;
    let mut f1_key_down = false;
    let mut h_key_down = false;
    let mut f2_key_down = false;
    let mut r_key_down = false;
    let mut t_key_down = false;
    let mut backspace_down = false;
    let mut home_down = false;
    let mut freecam_enabled = false;

    let mut cam_pos = glm::vec3(0.0f32, 0.0, 0.0);
    let mut cam_yaw = 0.0f32;
    let mut cam_pitch = 0.0f32;
    let mut cam_roll = 0.0f32;
    let mut cam_fov = 45.0f32;
    let base_speed = 12.0f32;

    let mut spawn_origin_pos: Option<[f32; 3]> = None;
    let mut frozen_player_pos: Option<[f32; 3]> = None;

    let mut last_mouse_pos = POINT { x: 0, y: 0 };
    let mut mouse_initialized = false;

    let mut last_frame = Instant::now();
    let mut logged_singleton = false;

    log_msg("Freecam loop started.");

    loop {
        std::thread::sleep(Duration::from_millis(2));
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().clamp(0.0005, 0.05);
        last_frame = now;

        set_player_no_dead(true);

        if spawn_origin_pos.is_none() {
            if let Some(phys) = get_player_physics_ptr() {
                unsafe {
                    let pos = *(phys.add(0x70) as *const [f32; 4]);
                    if (pos[0].abs() > 0.1 || pos[1].abs() > 0.1 || pos[2].abs() > 0.1) && pos[0].is_finite() {
                        spawn_origin_pos = Some([pos[0], pos[1], pos[2]]);
                        log_msg(&format!(
                            "Spawn origin registered: ({:.2}, {:.2}, {:.2})",
                            pos[0], pos[1], pos[2]
                        ));
                    }
                }
            }
        }

        let toggle_p = is_key_pressed(0x50, &mut p_key_down);
        let toggle_f1 = is_key_pressed(0x70, &mut f1_key_down);
        let toggle_osd = is_key_pressed(0x48, &mut h_key_down) || is_key_pressed(0x71, &mut f2_key_down);
        let reset_r = is_key_pressed(0x52, &mut r_key_down);

        if toggle_osd {
            let was_vis = OVERLAY_VISIBLE.fetch_xor(true, Ordering::SeqCst);
            let now_vis = !was_vis;
            let hwnd = OVERLAY_HWND.load(Ordering::Relaxed);
            if hwnd != 0 {
                unsafe {
                    if now_vis {
                        ShowWindow(hwnd as isize, 4);
                        SetWindowPos(hwnd as isize, -1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
                    } else {
                        ShowWindow(hwnd as isize, 0);
                    }
                }
            }
        }

        let camera_res = unsafe { <CSCamera as FromStatic>::instance() };
        let camera = match camera_res {
            Ok(cam) => cam,
            Err(_) => {
                continue;
            }
        };

        if !logged_singleton {
            logged_singleton = true;
            log_msg(&format!("CSCamera resolved at {:p}", camera));
        }

        let rescue_home = is_key_pressed(0x24, &mut home_down)
            || is_key_pressed(0x08, &mut backspace_down);

        if rescue_home {
            if let Some(origin) = spawn_origin_pos {
                log_msg(&format!(
                    "Returning player to spawn origin: ({:.2}, {:.2}, {:.2})",
                    origin[0], origin[1], origin[2]
                ));
                teleport_player_clean(origin);
                set_player_no_dead(true);

                if freecam_enabled {
                    freecam_enabled = false;
                    FREECAM_ACTIVE.store(false, Ordering::SeqCst);
                    camera.camera_mask = 0;
                    frozen_player_pos = None;
                    mouse_initialized = false;
                }
            }
        }

        let primary_ptr = match get_primary_camera(camera) {
            Some(p) => p,
            None => {
                continue;
            }
        };

        let primary_cam = unsafe { &*primary_ptr };

        if toggle_p || toggle_f1 {
            freecam_enabled = !freecam_enabled;
            FREECAM_ACTIVE.store(freecam_enabled, Ordering::SeqCst);

            if freecam_enabled {
                log_msg("Freecam enabled");
                let m = &primary_cam.matrix;
                cam_pos = glm::vec3(m.3 .0, m.3 .1, m.3 .2);
                cam_fov = primary_cam.fov;

                let fwd = glm::vec3(m.2 .0, m.2 .1, m.2 .2);
                cam_pitch = (-fwd.y).clamp(-0.999, 0.999).asin();
                cam_yaw = fwd.x.atan2(fwd.z);
                cam_roll = 0.0;

                unsafe {
                    let _ = ClipCursor(std::ptr::null());
                }

                let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
                let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
                let cx = if screen_w > 0 { screen_w / 2 } else { 960 };
                let cy = if screen_h > 0 { screen_h / 2 } else { 540 };
                real_set_cursor_pos(cx, cy);
                last_mouse_pos = POINT { x: cx, y: cy };
                mouse_initialized = true;

                if let Some(coords_ptr) = get_player_coords_ptr() {
                    let p_pos = unsafe { *coords_ptr };
                    frozen_player_pos = Some(p_pos);
                    log_msg(&format!(
                        "Player position locked: ({:.2}, {:.2}, {:.2})",
                        p_pos[0], p_pos[1], p_pos[2]
                    ));
                }

                set_player_no_dead(true);
                camera.camera_mask = 0b00100010;
                let init_matrix = primary_cam.matrix;
                update_all_cameras(camera, init_matrix, cam_fov);

                let hwnd = OVERLAY_HWND.load(Ordering::Relaxed);
                if hwnd != 0 && OVERLAY_VISIBLE.load(Ordering::Relaxed) {
                    unsafe {
                        SetWindowPos(hwnd as isize, -1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010);
                    }
                }
            } else {
                log_msg("Freecam disabled");
                camera.camera_mask = 0;

                frozen_player_pos = None;
                mouse_initialized = false;
            }
        }

        if !freecam_enabled {
            continue;
        }

        camera.camera_mask = 0b00100010;

        if let Some(freeze_pos) = frozen_player_pos {
            teleport_player_clean(freeze_pos);
        }

        let teleport_to_cam = is_key_pressed(0x54, &mut t_key_down);

        if teleport_to_cam {
            let target_pos = [cam_pos.x, cam_pos.y, cam_pos.z];
            log_msg(&format!(
                "Teleported player to camera: ({:.2}, {:.2}, {:.2})",
                target_pos[0], target_pos[1], target_pos[2]
            ));

            teleport_player_clean(target_pos);
            set_player_no_dead(true);

            freecam_enabled = false;
            FREECAM_ACTIVE.store(false, Ordering::SeqCst);
            camera.camera_mask = 0;
            frozen_player_pos = None;
            mouse_initialized = false;
            continue;
        }

        let mut speed_multiplier = 1.0f32;

        if is_key_down(0x10) {
            speed_multiplier *= 3.5;
        }
        if is_key_down(0x12) {
            speed_multiplier *= 0.25;
        }


        let move_speed = base_speed * speed_multiplier;
        let rot_speed = 2.0f32;

        let mut cur_mouse = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut cur_mouse).is_ok() } {
            if !mouse_initialized {
                last_mouse_pos = cur_mouse;
                mouse_initialized = true;
            } else {
                let dx = (cur_mouse.x - last_mouse_pos.x) as f32;
                let dy = (cur_mouse.y - last_mouse_pos.y) as f32;
                last_mouse_pos = cur_mouse;

                if dx.abs() > 0.0 || dy.abs() > 0.0 {
                    let mut sens = 0.0032f32;
                    if is_key_down(0x02) || is_key_down(0x12) {
                        sens *= 0.25;
                    }
                    if is_key_down(0x10) {
                        sens *= 1.5;
                    }

                    cam_yaw += dx * sens;
                    cam_pitch += dy * sens;

                    if cam_yaw > std::f32::consts::PI {
                        cam_yaw -= std::f32::consts::PI * 2.0;
                    } else if cam_yaw < -std::f32::consts::PI {
                        cam_yaw += std::f32::consts::PI * 2.0;
                    }
                }

                let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
                let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
                let cx = if screen_w > 0 { screen_w / 2 } else { 960 };
                let cy = if screen_h > 0 { screen_h / 2 } else { 540 };

                let dist_x = (cur_mouse.x - cx).abs();
                let dist_y = (cur_mouse.y - cy).abs();
                if dist_x > 150 || dist_y > 120 {
                    unsafe {
                        let _ = ClipCursor(std::ptr::null());
                    }
                    real_set_cursor_pos(cx, cy);
                    last_mouse_pos = POINT { x: cx, y: cy };
                }
            }
        }

        if is_key_down(0x25) || is_key_down(0x64) {
            cam_yaw -= rot_speed * dt;
        }
        if is_key_down(0x27) || is_key_down(0x66) {
            cam_yaw += rot_speed * dt;
        }
        if is_key_down(0x26) || is_key_down(0x68) {
            cam_pitch -= rot_speed * dt;
        }
        if is_key_down(0x28) || is_key_down(0x62) {
            cam_pitch += rot_speed * dt;
        }

        if is_key_down(0x51) {
            cam_roll -= rot_speed * 0.8 * dt;
        }
        if is_key_down(0x45) {
            cam_roll += rot_speed * 0.8 * dt;
        }
        if reset_r {
            cam_roll = 0.0;
        }


        cam_pitch = cam_pitch.clamp(-1.55, 1.55);

        let forward = glm::vec3(
            cam_yaw.sin() * cam_pitch.cos(),
            -cam_pitch.sin(),
            cam_yaw.cos() * cam_pitch.cos(),
        );
        let world_up = glm::vec3(0.0, 1.0, 0.0);
        let right_base = glm::normalize(&glm::cross(&world_up, &forward));
        let up_base = glm::normalize(&glm::cross(&forward, &right_base));

        let right = if cam_roll.abs() > 0.001 {
            glm::rotate_vec3(&right_base, cam_roll, &forward)
        } else {
            right_base
        };
        let up = if cam_roll.abs() > 0.001 {
            glm::rotate_vec3(&up_base, cam_roll, &forward)
        } else {
            up_base
        };

        let mut move_dir = glm::vec3(0.0f32, 0.0, 0.0);

        if is_key_down(0x57) {
            move_dir += forward;
        }
        if is_key_down(0x53) {
            move_dir -= forward;
        }
        if is_key_down(0x41) {
            move_dir -= right;
        }
        if is_key_down(0x44) {
            move_dir += right;
        }
        if is_key_down(0x20) {
            move_dir += world_up;
        }
        if is_key_down(0x11) || is_key_down(0x43) {
            move_dir -= world_up;
        }

        if glm::length(&move_dir) > 0.001 {
            let len = glm::length(&move_dir);
            let dir = glm::normalize(&move_dir);
            let step = dir * (len.min(1.0) * move_speed * dt);
            cam_pos += step;
        }

        let new_matrix = F32Matrix4x4(
            F32Vector4(right.x, right.y, right.z, 0.0),
            F32Vector4(up.x, up.y, up.z, 0.0),
            F32Vector4(forward.x, forward.y, forward.z, 0.0),
            F32Vector4(cam_pos.x, cam_pos.y, cam_pos.z, 1.0),
        );

        update_all_cameras(camera, new_matrix, cam_fov);
    }
}
