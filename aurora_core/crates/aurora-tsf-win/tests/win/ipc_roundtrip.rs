//! 从 DLL 里调测试导出，验证传输层。
use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

/// GetProcAddress 要的是 **ASCII** 函数名，不是 UTF-16。
fn narrow(s: &str) -> Vec<u8> { s.bytes().chain(std::iter::once(0)).collect() }

type PipeNameFn = unsafe extern "system" fn(*const u16, *const u16, *mut u16, u32) -> i32;
type RoundtripFn = unsafe extern "system" fn(*const u16) -> i32;

fn main() {
    let dll = std::env::args().nth(1).unwrap_or_else(|| r"C:\dll\aurora_tsf_win.dll".into());
    let pipe = std::env::args().nth(2).unwrap_or_else(|| r"\\.\pipe\aurora_input_test".into());

    println!("=== 传输层端到端测试 ===");
    println!("DLL : {dll}");
    println!("管道: {pipe}");

    let w = wide(&dll);
    let hmod: HMODULE = unsafe { LoadLibraryW(PCWSTR(w.as_ptr())).expect("加载 DLL 失败") };
    println!("[1] DLL 已加载");

    // ── 验证管道名计算 ──
    let n = narrow("AuroraTsfTestPipeName");
    let f = unsafe { GetProcAddress(hmod, windows::core::PCSTR(n.as_ptr())) };
    if let Some(f) = f {
        let f: PipeNameFn = unsafe { std::mem::transmute(f) };
        let dev = wide("_dev");
        let sid = wide("_S-1-5-21-999");
        let mut buf = vec![0u16; 256];
        let len = unsafe { f(dev.as_ptr(), sid.as_ptr(), buf.as_mut_ptr(), 256) };
        if len > 0 {
            let name = String::from_utf16_lossy(&buf[..len as usize]);
            println!("[2] 管道名计算: {name}");
            let expect = r"\\.\pipe\aurora_input_dev_S-1-5-21-999";
            if name == expect {
                println!("[2] 与预期一致 ✓");
            } else {
                println!("[2] FAIL 预期 {expect}");
            }
        } else {
            println!("[2] FAIL 管道名计算返回 {len}");
        }
    } else {
        println!("[2] FAIL 缺导出 AuroraTsfTestPipeName");
    }

    // ── 端到端收发 ──
    let n = narrow("AuroraTsfTestRoundtrip");
    let f = unsafe { GetProcAddress(hmod, windows::core::PCSTR(n.as_ptr())) };
    match f {
        Some(f) => {
            let f: RoundtripFn = unsafe { std::mem::transmute(f) };
            let p = wide(&pipe);
            println!("[3] 发起 roundtrip...");
            let rc = unsafe { f(p.as_ptr()) };
            match rc {
                0 => println!("[4] roundtrip 成功 ✓（发出并收到一帧）"),
                -1 => println!("[4] FAIL 参数为空"),
                -2 => println!("[4] FAIL 连不上管道"),
                -3 => println!("[4] FAIL 写失败"),
                -4 => println!("[4] FAIL 读失败"),
                _ => println!("[4] FAIL 未知错误 {rc}"),
            }
        }
        None => println!("[3] FAIL 缺导出 AuroraTsfTestRoundtrip"),
    }

    unsafe { let _ = FreeLibrary(hmod); }
    println!("\n=== 完成 ===");
}
