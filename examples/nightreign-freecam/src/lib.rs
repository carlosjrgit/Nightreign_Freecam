use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fromsoftware_shared::{F32Matrix4x4, F32Vector4, FromStatic};
use nalgebra_glm as glm;
use nightreign::cs::{CSCam, CSCamera};
use windows::core::PCSTR;
use windows::Win32::Foundation::{BOOL, HINSTANCE, POINT};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress, LoadLibraryA};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

extern "system" {
    fn GetSystemMetrics(nIndex: i32) -> i32;
    fn ClipCursor(lpRect: *const std::ffi::c_void) -> BOOL;
}

const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;

// =========================================================================
// DEFINIÇÕES E TIPOS DO XINPUT (CONTROLE / JOYSTICK)
// =========================================================================

pub const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;  // L3
pub const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080; // R3
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100; // LB
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200; // RB
pub const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub const XINPUT_GAMEPAD_Y: u16 = 0x8000;

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
static DIRECT_XINPUT_GET_STATE: AtomicUsize = AtomicUsize::new(0);

static LATEST_REAL_GAMEPAD: Mutex<Option<XINPUT_STATE>> = Mutex::new(None);

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

// =========================================================================
// IAT HOOKS: XINPUT & SETCURSORPOS (IMPEDE O JOGO DE BRIGAR COM O MOUSE E CONTROLE)
// =========================================================================

unsafe extern "system" fn hooked_xinput_get_state(
    dw_user_index: u32,
    p_state: *mut XINPUT_STATE,
) -> u32 {
    let orig = ORIGINAL_XINPUT_GET_STATE.load(Ordering::Relaxed);
    if orig == 0 {
        return 1167; // ERROR_DEVICE_NOT_CONNECTED
    }
    let orig_fn: FnXInputGetState = std::mem::transmute(orig);
    let res = orig_fn(dw_user_index, p_state);
    if res != 0 || p_state.is_null() {
        return res;
    }

    // Salva o estado real ANTES de qualquer modificação para a nossa câmera usar!
    if dw_user_index == 0 {
        if let Ok(mut lock) = LATEST_REAL_GAMEPAD.lock() {
            *lock = Some(*p_state);
        }
    }

    // Se a freecam estiver ativa, ocultamos os inputs do jogo para que o
    // personagem fique estático, sem andar, esquivar ou bater!
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

/// Impede o motor do jogo de puxar o cursor do mouse de volta para o centro enquanto Freecam estiver ativa!
unsafe extern "system" fn hooked_set_cursor_pos(x: i32, y: i32) -> BOOL {
    if FREECAM_ACTIVE.load(Ordering::Relaxed) {
        // Bloqueia a tentativa do jogo de travar o cursor!
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

/// Chamada direta e real do SetCursorPos do Windows para recentralizar o cursor da Freecam sem ser bloqueado
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

    // 1. Hook XInputGetState (RVA: 0xD8FC9C)
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
                log_msg(&format!("IAT Hook XInput instalado em 0x{:X}", base + 0xD8FC9C));
            }
        }
    }

    // 2. Hook SetCursorPos (RVA: 0xD8F93C)
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
                log_msg(&format!("IAT Hook SetCursorPos instalado em 0x{:X}", base + 0xD8F93C));
            }
        }
    }
}

// Carrega XInputGetState nativo diretamente como fallback
fn init_direct_xinput() {
    unsafe {
        let mut h = LoadLibraryA(PCSTR(b"xinput1_4.dll\0".as_ptr()));
        if h.is_err() {
            h = LoadLibraryA(PCSTR(b"xinput1_3.dll\0".as_ptr()));
        }
        if h.is_err() {
            h = LoadLibraryA(PCSTR(b"xinput9_1_0.dll\0".as_ptr()));
        }
        if let Ok(mod_handle) = h {
            if let Some(proc) = GetProcAddress(mod_handle, PCSTR(b"XInputGetState\0".as_ptr())) {
                DIRECT_XINPUT_GET_STATE.store(proc as usize, Ordering::SeqCst);
                log_msg("XInput nativo pronto para camera!");
            }
        }
    }
}

fn poll_gamepad_direct(dw_user_index: u32) -> Option<XINPUT_STATE> {
    let proc_addr = DIRECT_XINPUT_GET_STATE.load(Ordering::Relaxed);
    let target = if proc_addr != 0 {
        proc_addr
    } else {
        ORIGINAL_XINPUT_GET_STATE.load(Ordering::Relaxed)
    };

    if target == 0 {
        return None;
    }

    unsafe {
        let get_state: FnXInputGetState = std::mem::transmute(target);
        let mut state = XINPUT_STATE::default();
        if get_state(dw_user_index, &mut state) == 0 {
            Some(state)
        } else {
            None
        }
    }
}

// =========================================================================
// CONGELAMENTO DO JOGADOR (PLAYER FREEZE)
// =========================================================================

/// Retorna o ponteiro para o módulo de física (CSChrPhysicsModule) do jogador principal.
fn get_player_physics_ptr() -> Option<*mut u8> {
    // 1. Tenta via Singleton oficial WorldChrMan
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

    // 2. Fallback via RVA estático do ponteiro de WorldChrMan no .data (+0x3B04378)
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

/// Retorna o ponteiro para as coordenadas X, Y, Z da física do jogador principal.
fn get_player_coords_ptr() -> Option<*mut [f32; 3]> {
    get_player_physics_ptr().map(|p| unsafe { p.add(0x70) as *mut [f32; 3] })
}

/// Teleporta o jogador de forma 100% limpa, sem acumular inércia e sem ser arremessado.
/// Atualiza tanto 'position' (0x70) quanto 'last_update_position' (0x80),
/// ativa 'chr_proxy_pos_update_requested' (0x91 = 1) e zera 'root_motion' (0xd0 e 0xe0).
fn teleport_player_clean(target: [f32; 3]) {
    if let Some(phys) = get_player_physics_ptr() {
        unsafe {
            let pos = phys.add(0x70) as *mut [f32; 4];
            let last_pos = phys.add(0x80) as *mut [f32; 4];
            *pos = [target[0], target[1], target[2], 1.0];
            *last_pos = [target[0], target[1], target[2], 1.0];
            *phys.add(0x91) = 1; // Notifica o Havok que a posição foi forçada (Delta P = 0)
            *(phys.add(0xd0) as *mut [f32; 4]) = [0.0, 0.0, 0.0, 0.0]; // root_motion
            *(phys.add(0xe0) as *mut [f32; 4]) = [0.0, 0.0, 0.0, 0.0]; // root_motion_unk
        }
    } else if let Some(coords_ptr) = get_player_coords_ptr() {
        unsafe {
            *coords_ptr = target;
        }
    }
}

/// Retorna o ponteiro para o byte de flags de debug (incluindo No Dead) em ChrDataModule.
fn get_player_no_dead_ptr() -> Option<*mut u8> {
    // 1. Tenta via Singleton oficial WorldChrMan
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

    // 2. Fallback via RVA estático do ponteiro de WorldChrMan no .data (+0x3B04378)
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

/// Ativa ou desativa a flag nativa No Dead do jogo (imunidade a morte por queda e dano).
fn set_player_no_dead(enable: bool) {
    if let Some(ptr) = get_player_no_dead_ptr() {
        unsafe {
            if enable {
                *ptr |= 0x02; // Bit 1 = No Dead
            } else {
                *ptr &= !0x02;
            }
        }
    }
}

// =========================================================================
// EXPORTS EXIGIDOS PELO GUI.EXE (Freecam+ v0.5.1)
// =========================================================================

#[no_mangle]
pub unsafe extern "C" fn initialize(ctx: *mut u8) -> u32 {
    log_msg("RPC: initialize chamado com sucesso pelo gui.exe!");
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

// =========================================================================
// CICLO DE VIDA DA DLL (DllMain)
// =========================================================================

#[no_mangle]
pub unsafe extern "C" fn DllMain(_hmodule: HINSTANCE, reason: u32) -> bool {
    if reason != 1 {
        return true;
    }

    log_msg("=== Nightreign Freecam v5 (Freecam Estavel + Teleporte Streaming 'T') Carregada! ===");

    init_direct_xinput();
    install_iat_hooks();

    std::thread::spawn(|| {
        let res = std::panic::catch_unwind(|| {
            run_freecam_loop();
        });
        if let Err(e) = res {
            log_msg(&format!("ERRO no loop de freecam: {:?}", e));
        }
    });

    true
}

/// Atualiza TODAS as 4 cameras de perspectiva simultaneamente.
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

/// Aplica deadzone circular e resposta exponencial
fn apply_deadzone_and_curve(raw_x: i16, raw_y: i16, deadzone: f32) -> (f32, f32) {
    let norm_x = raw_x as f32 / 32767.0;
    let norm_y = raw_y as f32 / 32767.0;
    let mag = (norm_x * norm_x + norm_y * norm_y).sqrt();

    if mag < deadzone {
        return (0.0, 0.0);
    }

    let normalized_mag = ((mag - deadzone) / (1.0 - deadzone)).min(1.0);
    let curved_mag = normalized_mag.powf(1.6);

    let factor = curved_mag / mag;
    (norm_x * factor, norm_y * factor)
}

fn run_freecam_loop() {
    let mut p_key_down = false;
    let mut f1_key_down = false;
    let mut r_key_down = false;
    let mut t_key_down = false;
    let mut backspace_down = false;
    let mut home_down = false;
    let mut controller_toggle_down = false;
    let mut freecam_enabled = false;

    let mut cam_pos = glm::vec3(0.0f32, 0.0, 0.0);
    let mut cam_yaw = 0.0f32;
    let mut cam_pitch = 0.0f32;
    let mut cam_roll = 0.0f32;
    let mut cam_fov = 45.0f32;
    let mut base_speed = 12.0f32;

    let mut spawn_origin_pos: Option<[f32; 3]> = None;
    let mut frozen_player_pos: Option<[f32; 3]> = None;

    let mut last_mouse_pos = POINT { x: 0, y: 0 };
    let mut mouse_initialized = false;

    let mut last_frame = Instant::now();
    let mut logged_singleton = false;

    log_msg("Loop de freecam ativo. Aguardando mapa/jogador carregar...");

    loop {
        std::thread::sleep(Duration::from_millis(2));
        let now = Instant::now();
        let dt = (now - last_frame).as_secs_f32().clamp(0.0005, 0.05);
        last_frame = now;

        // 1. Mantém No Dead permanentemente ativo:
        // Elimina 100% dano e morte por queda. O personagem pode cair eternamente sem morrer.
        // Ao tocar em qualquer superfície sólida ou objeto com colisão, pousa em pé normalmente.
        set_player_no_dead(true);

        // 2. Salva a posição de surgimento inicial (spawn) assim que as coordenadas forem válidas
        if spawn_origin_pos.is_none() {
            if let Some(phys) = get_player_physics_ptr() {
                unsafe {
                    let pos = *(phys.add(0x70) as *const [f32; 4]);
                    if (pos[0].abs() > 0.1 || pos[1].abs() > 0.1 || pos[2].abs() > 0.1) && pos[0].is_finite() {
                        spawn_origin_pos = Some([pos[0], pos[1], pos[2]]);
                        log_msg(&format!(
                            "[SPAWN REGISTRADO] Origem inicial de spawn salva: ({:.2}, {:.2}, {:.2})",
                            pos[0], pos[1], pos[2]
                        ));
                    }
                }
            }
        }

        // Leitura de teclas de alternância
        let toggle_p = is_key_pressed(0x50, &mut p_key_down);
        let toggle_f1 = is_key_pressed(0x70, &mut f1_key_down);
        let reset_r = is_key_pressed(0x52, &mut r_key_down);

        // Obter estado real do Gamepad
        let gamepad_state = {
            let lock_state = LATEST_REAL_GAMEPAD.lock().ok().and_then(|g| *g);
            if lock_state.is_some() {
                lock_state
            } else {
                poll_gamepad_direct(0)
            }
        };

        let camera_res = unsafe { <CSCamera as FromStatic>::instance() };
        let camera = match camera_res {
            Ok(cam) => cam,
            Err(_) => {
                continue;
            }
        };

        if !logged_singleton {
            logged_singleton = true;
            log_msg(&format!("CSCamera encontrado em {:p}!", camera));
        }

        // 3. Botão de Resgate 24/7 (Home / Backspace / LB + Back no controle):
        // Leva o boneco instantaneamente de volta para onde surgiu pela primeira vez!
        // Se estiver caindo infinitamente no void ou preso, basta apertar HOME que ele retorna.
        let controller_rescue = if let Some(ref gp) = gamepad_state {
            let btns = gp.gamepad.w_buttons;
            (btns & XINPUT_GAMEPAD_LEFT_SHOULDER != 0) && (btns & XINPUT_GAMEPAD_BACK != 0)
        } else {
            false
        };

        let rescue_home = is_key_pressed(0x24, &mut home_down)
            || is_key_pressed(0x08, &mut backspace_down)
            || controller_rescue;

        if rescue_home {
            if let Some(origin) = spawn_origin_pos {
                log_msg(&format!(
                    "[RESCUE HOME] Resgatando jogador para a origem inicial de spawn: ({:.2}, {:.2}, {:.2})",
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

        // Alternância pelo controle: L3 + R3
        let mut toggle_controller = false;
        if let Some(ref gp) = gamepad_state {
            let l3_r3 = (gp.gamepad.w_buttons & XINPUT_GAMEPAD_LEFT_THUMB != 0)
                && (gp.gamepad.w_buttons & XINPUT_GAMEPAD_RIGHT_THUMB != 0);
            if l3_r3 && !controller_toggle_down {
                toggle_controller = true;
            }
            controller_toggle_down = l3_r3;
        }

        let primary_ptr = match get_primary_camera(camera) {
            Some(p) => p,
            None => {
                continue;
            }
        };

        let primary_cam = unsafe { &*primary_ptr };

        // Alternância do Freecam
        if toggle_p || toggle_f1 || toggle_controller {
            freecam_enabled = !freecam_enabled;
            FREECAM_ACTIVE.store(freecam_enabled, Ordering::SeqCst);

            if freecam_enabled {
                log_msg(">>> FREECAM ATIVADA! Pressione [T] para cair exatamente no local da câmera. <<<");
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

                // Salva a posição física do personagem para congelá-lo imóvel
                if let Some(coords_ptr) = get_player_coords_ptr() {
                    let p_pos = unsafe { *coords_ptr };
                    frozen_player_pos = Some(p_pos);
                    log_msg(&format!(
                        "Personagem imobilizado em: ({:.2}, {:.2}, {:.2})",
                        p_pos[0], p_pos[1], p_pos[2]
                    ));
                }

                set_player_no_dead(true);
                camera.camera_mask = 0b00100010;
                let init_matrix = primary_cam.matrix;
                update_all_cameras(camera, init_matrix, cam_fov);
            } else {
                log_msg(">>> FREECAM DESATIVADA! Restaurando controle normal do jogador. <<<");
                camera.camera_mask = 0;

                // Se Shift estiver pressionado durante o toggle de saída, restaura a posição inicial
                if is_key_down(0x10) {
                    if let Some(origin) = spawn_origin_pos {
                        teleport_player_clean(origin);
                        log_msg(&format!(
                            "Personagem retornado para a origem: ({:.2}, {:.2}, {:.2})",
                            origin[0], origin[1], origin[2]
                        ));
                    }
                }

                frozen_player_pos = None;
                mouse_initialized = false;
            }
        }

        if !freecam_enabled {
            continue;
        }

        camera.camera_mask = 0b00100010;

        // Manter o personagem congelado no lugar atual com velocidade ZERO
        if let Some(freeze_pos) = frozen_player_pos {
            teleport_player_clean(freeze_pos);
        }

        // =====================================================================
        // TELEPORTE SOB DEMANDA (TECLA 'T' OU LB + Y NO CONTROLE)
        // =====================================================================
        // Puxa o personagem para a posição exata da câmera com Delta P = 0 e momento zerado.
        // Desativa a Freecam imediatamente para que o personagem caia suavemente no local
        // sem ser arremessado, com física normal e sem morrer por queda!
        let teleport_to_cam = is_key_pressed(0x54, &mut t_key_down)
            || (if let Some(ref gp) = gamepad_state {
                let btns = gp.gamepad.w_buttons;
                (btns & XINPUT_GAMEPAD_LEFT_SHOULDER != 0) && (btns & XINPUT_GAMEPAD_Y != 0)
            } else {
                false
            });

        if teleport_to_cam {
            let target_pos = [cam_pos.x, cam_pos.y, cam_pos.z];
            log_msg(&format!(
                "[TELEPORTE T] Personagem posicionado em ({:.2}, {:.2}, {:.2}) sem inércia. Desativando Freecam para queda limpa!",
                target_pos[0], target_pos[1], target_pos[2]
            ));

            // 1. Teleporte limpo com Delta P = 0 e momento zerado (zero arremesso/catapulta)
            teleport_player_clean(target_pos);

            // 2. Garante No Dead ativo para pouso seguro
            set_player_no_dead(true);

            // 3. Desativa a freecam imediatamente para o jogo reassumir
            freecam_enabled = false;
            FREECAM_ACTIVE.store(false, Ordering::SeqCst);
            camera.camera_mask = 0;
            frozen_player_pos = None;
            mouse_initialized = false;
            continue;
        }

        // =====================================================================
        // CONTROLE DE VELOCIDADE
        // =====================================================================
        let mut speed_multiplier = 1.0f32;

        // Teclado: Shift = Boost, Alt = Slow
        if is_key_down(0x10) {
            speed_multiplier *= 3.5;
        }
        if is_key_down(0x12) {
            speed_multiplier *= 0.25;
        }

        // Teclado: Teclas 1 e 2 para ajustar velocidade base
        if is_key_down(0x31) {
            base_speed = (base_speed - 6.0 * dt).clamp(1.5, 120.0);
        }
        if is_key_down(0x32) {
            base_speed = (base_speed + 6.0 * dt).clamp(1.5, 120.0);
        }

        // Controle: B ou L3 = Boost, A ou X = Slow
        if let Some(ref gp) = gamepad_state {
            let btns = gp.gamepad.w_buttons;
            if (btns & XINPUT_GAMEPAD_B != 0) || (btns & XINPUT_GAMEPAD_LEFT_THUMB != 0) {
                speed_multiplier *= 3.5;
            }
            if (btns & XINPUT_GAMEPAD_A != 0) || (btns & XINPUT_GAMEPAD_X != 0) {
                speed_multiplier *= 0.25;
            }

            // D-Pad Cima / Baixo = Ajustar velocidade base
            if btns & XINPUT_GAMEPAD_LEFT_SHOULDER == 0 {
                if btns & XINPUT_GAMEPAD_DPAD_UP != 0 {
                    base_speed = (base_speed + 6.0 * dt).clamp(1.5, 120.0);
                }
                if btns & XINPUT_GAMEPAD_DPAD_DOWN != 0 {
                    base_speed = (base_speed - 6.0 * dt).clamp(1.5, 120.0);
                }
            }
        }

        let move_speed = base_speed * speed_multiplier;
        let rot_speed = 2.0f32;

        // =====================================================================
        // ROTAÇÃO DA CÂMERA PELO MOUSE (GIRO LIVRE GLOBAL DE 360° INFINITO)
        // =====================================================================
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
                    // Botão Direito do Mouse ou Alt = Modo Precisão Lenta
                    if is_key_down(0x02) || is_key_down(0x12) {
                        sens *= 0.25;
                    }
                    if is_key_down(0x10) {
                        sens *= 1.5;
                    }

                    cam_yaw += dx * sens;
                    cam_pitch += dy * sens;

                    // Mantém cam_yaw normalizado entre -PI e +PI para precisão trigonométrica contínua
                    if cam_yaw > std::f32::consts::PI {
                        cam_yaw -= std::f32::consts::PI * 2.0;
                    } else if cam_yaw < -std::f32::consts::PI {
                        cam_yaw += std::f32::consts::PI * 2.0;
                    }
                }

                // Recentralização simétrica baseada na distância do centro da tela:
                // Quando o cursor se afasta do centro mais de 150px em qualquer direção,
                // reposicionamos de volta no centro (cx, cy).
                // Isso garante que o mouse NUNCA alcance a borda física da tela em nenhum lado
                // (nem esquerda, nem direita, nem topo, nem base),
                // permitindo rotações contínuas de 360°, 720°, 1080° e infinitas para ambos os lados!
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

        // =====================================================================
        // ROTAÇÃO DA CÂMERA PELO JOYSTICK (ANALÓGICO DIREITO)
        // =====================================================================
        if let Some(ref gp) = gamepad_state {
            let (look_x, look_y) =
                apply_deadzone_and_curve(gp.gamepad.s_thumb_rx, gp.gamepad.s_thumb_ry, 0.18);
            if look_x.abs() > 0.001 || look_y.abs() > 0.001 {
                let stick_sens = 2.4f32;
                cam_yaw += look_x * stick_sens * dt;
                cam_pitch -= look_y * stick_sens * dt;
            }
        }

        // Setas do teclado
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

        // =====================================================================
        // INCLINAÇÃO (ROLL) E ZOOM (FOV)
        // =====================================================================
        // Teclado: Q / E para Roll, R para resetar
        if is_key_down(0x51) {
            cam_roll -= rot_speed * 0.8 * dt;
        }
        if is_key_down(0x45) {
            cam_roll += rot_speed * 0.8 * dt;
        }
        if reset_r {
            cam_roll = 0.0;
        }

        // Teclado: [ e ] para FOV
        if is_key_down(0xDB) {
            cam_fov = (cam_fov - 25.0 * dt).clamp(5.0, 130.0);
        }
        if is_key_down(0xDD) {
            cam_fov = (cam_fov + 25.0 * dt).clamp(5.0, 130.0);
        }

        // Controle: D-Pad Esquerda / Direita = Roll, R3 = Resetar Roll
        if let Some(ref gp) = gamepad_state {
            let btns = gp.gamepad.w_buttons;

            if btns & XINPUT_GAMEPAD_DPAD_LEFT != 0 {
                cam_roll -= rot_speed * 0.8 * dt;
            }
            if btns & XINPUT_GAMEPAD_DPAD_RIGHT != 0 {
                cam_roll += rot_speed * 0.8 * dt;
            }
            if (btns & XINPUT_GAMEPAD_RIGHT_THUMB != 0) && (btns & XINPUT_GAMEPAD_LEFT_THUMB == 0) {
                cam_roll = 0.0;
            }

            // LB + D-Pad Cima / Baixo = FOV Zoom In / Out
            if btns & XINPUT_GAMEPAD_LEFT_SHOULDER != 0 {
                if btns & XINPUT_GAMEPAD_DPAD_UP != 0 {
                    cam_fov = (cam_fov - 20.0 * dt).clamp(5.0, 130.0);
                }
                if btns & XINPUT_GAMEPAD_DPAD_DOWN != 0 {
                    cam_fov = (cam_fov + 20.0 * dt).clamp(5.0, 130.0);
                }
            }
        }

        cam_pitch = cam_pitch.clamp(-1.55, 1.55);

        // =====================================================================
        // MATRIZ DE ORIENTAÇÃO 6DOF
        // =====================================================================
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

        // =====================================================================
        // MOVIMENTAÇÃO 6DOF (WASD + JOYSTICK ESQUERDO + GATILHOS)
        // =====================================================================
        let mut move_dir = glm::vec3(0.0f32, 0.0, 0.0);

        // 1. Teclado: WASD + Espaço + Ctrl/C
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

        // 2. Analógico Esquerdo do Controle (LS)
        if let Some(ref gp) = gamepad_state {
            let (stick_x, stick_y) =
                apply_deadzone_and_curve(gp.gamepad.s_thumb_lx, gp.gamepad.s_thumb_ly, 0.18);
            move_dir += forward * stick_y;
            move_dir += right * stick_x;

            // Gatilhos analógicos para Subida e Descida proporcional à pressão
            let trig_deadzone = 20.0f32;
            let lt = if (gp.gamepad.b_left_trigger as f32) > trig_deadzone {
                ((gp.gamepad.b_left_trigger as f32 - trig_deadzone) / (255.0 - trig_deadzone))
                    .powf(1.4)
            } else {
                0.0
            };
            let rt = if (gp.gamepad.b_right_trigger as f32) > trig_deadzone {
                ((gp.gamepad.b_right_trigger as f32 - trig_deadzone) / (255.0 - trig_deadzone))
                    .powf(1.4)
            } else {
                0.0
            };

            let vertical_speed = rt - lt;
            if vertical_speed.abs() > 0.001 {
                move_dir += world_up * vertical_speed;
            }

            let btns = gp.gamepad.w_buttons;
            if btns & XINPUT_GAMEPAD_RIGHT_SHOULDER != 0 {
                move_dir += world_up * 1.0;
            }
        }

        if glm::length(&move_dir) > 0.001 {
            let len = glm::length(&move_dir);
            let dir = glm::normalize(&move_dir);
            let step = dir * (len.min(1.0) * move_speed * dt);
            cam_pos += step;
        }

        // Matriz de visão 4x4 da câmera
        let new_matrix = F32Matrix4x4(
            F32Vector4(right.x, right.y, right.z, 0.0),
            F32Vector4(up.x, up.y, up.z, 0.0),
            F32Vector4(forward.x, forward.y, forward.z, 0.0),
            F32Vector4(cam_pos.x, cam_pos.y, cam_pos.z, 1.0),
        );

        update_all_cameras(camera, new_matrix, cam_fov);
    }
}
