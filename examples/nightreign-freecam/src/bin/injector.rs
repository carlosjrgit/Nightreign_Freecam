use std::env;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[repr(C)]
struct PROCESSENTRY32W {
    dw_size: u32,
    cnt_usage: u32,
    th32_process_id: u32,
    th32_default_heap_id: usize,
    th32_module_id: u32,
    cnt_threads: u32,
    th32_parent_process_id: u32,
    pc_pri_class_base: i32,
    dw_flags: u32,
    sz_exe_file: [u16; 260],
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: i32, dwProcessId: u32) -> isize;
    fn VirtualAllocEx(
        hProcess: isize,
        lpAddress: *mut c_void,
        dwSize: usize,
        flAllocationType: u32,
        flProtect: u32,
    ) -> *mut c_void;
    fn WriteProcessMemory(
        hProcess: isize,
        lpBaseAddress: *mut c_void,
        lpBuffer: *const c_void,
        nSize: usize,
        lpNumberOfBytesWritten: *mut usize,
    ) -> i32;
    fn CreateRemoteThread(
        hProcess: isize,
        lpThreadAttributes: *mut c_void,
        dwStackSize: usize,
        lpStartAddress: unsafe extern "system" fn(*mut c_void) -> u32,
        lpParameter: *mut c_void,
        dwCreationFlags: u32,
        lpThreadId: *mut u32,
    ) -> isize;
    fn WaitForSingleObject(hHandle: isize, dwMilliseconds: u32) -> u32;
    fn CloseHandle(hObject: isize) -> i32;
    fn GetModuleHandleA(lpModuleName: *const u8) -> isize;
    fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> *mut c_void;
    fn CreateToolhelp32Snapshot(dwFlags: u32, th32ProcessID: u32) -> isize;
    fn Process32FirstW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
    fn Process32NextW(hSnapshot: isize, lppe: *mut PROCESSENTRY32W) -> i32;
}

const TH32CS_SNAPPROCESS: u32 = 0x00000002;
const PROCESS_ALL_ACCESS: u32 = 0x001F0FFF;
const MEM_COMMIT: u32 = 0x00001000;
const MEM_RESERVE: u32 = 0x00002000;
const PAGE_READWRITE: u32 = 0x04;

fn find_process_id(name: &str) -> Option<u32> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == -1 || snapshot == 0 {
            return None;
        }

        let mut entry = PROCESSENTRY32W {
            dw_size: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            cnt_usage: 0,
            th32_process_id: 0,
            th32_default_heap_id: 0,
            th32_module_id: 0,
            cnt_threads: 0,
            th32_parent_process_id: 0,
            pc_pri_class_base: 0,
            dw_flags: 0,
            sz_exe_file: [0u16; 260],
        };

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let proc_name = String::from_utf16_lossy(&entry.sz_exe_file)
                    .trim_matches(char::from(0))
                    .to_lowercase();
                if proc_name == name.to_lowercase() {
                    CloseHandle(snapshot);
                    return Some(entry.th32_process_id);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        None
    }
}

fn wait_for_enter() {
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
}

fn main() {
    println!("Elden Ring Nightreign - Freecam Injector");

    let pid = match find_process_id("nightreign.exe") {
        Some(id) => id,
        None => {
            eprintln!("nightreign.exe process not found.");
            eprintln!("Start the game and load into the world before injecting.");
            println!("\nPress Enter to exit...");
            wait_for_enter();
            return;
        }
    };

    println!("Target process found: PID {}", pid);

    let current_exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = current_exe.parent().unwrap_or_else(|| Path::new("."));

    let candidates = [
        exe_dir.join("agent.dll"),
        exe_dir.join("nightreign_freecam.dll"),
    ];

    let dll_path = candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| exe_dir.join("agent.dll"));

    if !dll_path.exists() {
        eprintln!("DLL not found: {}", dll_path.display());
        println!("\nPress Enter to exit...");
        wait_for_enter();
        return;
    }

    let dll_path_canonical = dll_path.canonicalize().unwrap_or(dll_path);
    println!("Target DLL: {}", dll_path_canonical.display());

    unsafe {
        let process = OpenProcess(PROCESS_ALL_ACCESS, 0, pid);
        if process == 0 {
            eprintln!("Failed to open process. Try running as Administrator.");
            println!("\nPress Enter to exit...");
            wait_for_enter();
            return;
        }

        let kernel32 = GetModuleHandleA(b"kernel32.dll\0".as_ptr());
        if kernel32 == 0 {
            eprintln!("Failed to get kernel32.dll handle.");
            CloseHandle(process);
            return;
        }

        let load_lib_addr = GetProcAddress(kernel32, b"LoadLibraryW\0".as_ptr());
        if load_lib_addr.is_null() {
            eprintln!("Failed to locate LoadLibraryW.");
            CloseHandle(process);
            return;
        }

        let mut path_u16: Vec<u16> = dll_path_canonical.as_os_str().encode_wide().collect();
        path_u16.push(0);
        let path_bytes = path_u16.len() * 2;

        let remote_mem = VirtualAllocEx(
            process,
            std::ptr::null_mut(),
            path_bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );

        if remote_mem.is_null() {
            eprintln!("Failed to allocate remote memory in target process.");
            CloseHandle(process);
            return;
        }

        let mut bytes_written: usize = 0;
        let write_res = WriteProcessMemory(
            process,
            remote_mem,
            path_u16.as_ptr() as *const c_void,
            path_bytes,
            &mut bytes_written as *mut usize,
        );

        if write_res == 0 {
            eprintln!("Failed to write DLL path to target memory.");
            CloseHandle(process);
            return;
        }

        let thread_fn: unsafe extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(load_lib_addr);

        let thread_handle = CreateRemoteThread(
            process,
            std::ptr::null_mut(),
            0,
            thread_fn,
            remote_mem,
            0,
            std::ptr::null_mut(),
        );

        if thread_handle == 0 {
            eprintln!("Failed to create remote thread.");
            CloseHandle(process);
            return;
        }

        println!("Injecting DLL...");
        WaitForSingleObject(thread_handle, 5000);

        CloseHandle(thread_handle);
        CloseHandle(process);

        println!("Injection successful.\n");
        println!("Controls:");
        println!("  P / F1 / L3+R3    : Toggle Freecam");
        println!("  W, A, S, D        : Move camera");
        println!("  Space / Ctrl      : Up / Down");
        println!("  Shift / Alt       : Boost / Slow speed");
        println!("  Mouse / R-Stick   : Look around (360)");
        println!("  Q / E (R to reset): Roll camera");
        println!("  [ / ]             : Adjust FOV");
        println!("  T / LB+Y          : Teleport character to camera & land");
        println!("  Home / Backspace  : Emergency return to spawn origin");
        println!("\nPress Enter to exit...");
        wait_for_enter();
    }
}
