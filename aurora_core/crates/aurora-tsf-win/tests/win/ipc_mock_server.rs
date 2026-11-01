//! 模拟 Rust 服务端：创建 Named Pipe，收一帧，回 ACK。
//!
//! 目的是与 aurora_tsf_win.dll 的传输层做端到端验证：
//!   - 管道名计算一致
//!  - 帧能发出来
//!  - 响应能收回并解码

use std::ffi::c_void;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, ERROR_PIPE_CONNECTED};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() {
    let pipe_name = std::env::args().nth(1).unwrap_or_else(|| {
        r"\\.\pipe\aurora_input_test".to_string()
    });
    println!("=== 模拟服务端 ===");
    println!("管道: {pipe_name}");

    let w = wide(&pipe_name);

    // 创建管道
    let server: HANDLE = unsafe {
        CreateNamedPipeW(
            PCWSTR(w.as_ptr()),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,       // 单实例
            4096,    // 出缓冲
            4096,    // 入缓冲
            0,       // 默认超时
            None,    // 默认安全属性
        )
    };
    if server.is_invalid() {
        println!("FAIL 创建管道失败: {:?}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    println!("[1] 管道已创建");

    // 等客户端连（ConnectNamedPipe 阻塞）
    println!("[2] 等待客户端连接...");
    let connected = unsafe { ConnectNamedPipe(server, None) };
    match connected {
        Ok(()) => println!("[3] 客户端已连接"),
        Err(e) if e.code() == windows::Win32::Foundation::ERROR_PIPE_CONNECTED.to_hresult() => {
            println!("[3] 客户端已连接（已就绪状态）")
        }
        Err(e) => {
            println!("FAIL ConnectNamedPipe: {e:?}");
            unsafe { let _ = CloseHandle(server); }
            std::process::exit(1);
        }
    }

    // 读一帧
    let mut buf = vec![0u8; 4096];
    let mut read = 0u32;
    let ok = unsafe { ReadFile(server, Some(&mut buf), Some(&mut read), None) };
    if ok.is_err() {
        println!("FAIL 读帧: {:?}", std::io::Error::last_os_error());
        unsafe { let _ = CloseHandle(server); }
        std::process::exit(1);
    }
    buf.truncate(read as usize);
    println!("[4] 收到 {} 字节", buf.len());

    // 解头部
    if buf.len() >= 8 {
        match aurora_ipc::codec::decode_header(&buf) {
            Ok(h) => {
                let (cmd, len) = (h.command, h.length);
                println!("[5] 头部解析成功: cmd=0x{cmd:04X} len={len}");
            }
            Err(e) => println!("[5] 头部解析失败: {e}"),
        }
    }

    // 回一帧 ACK
    let ack = aurora_ipc::codec::encode_ack();
    let mut written = 0u32;
    let ok = unsafe { WriteFile(server, Some(&ack), Some(&mut written), None) };
    match ok {
        Ok(()) => println!("[6] ACK 已回（{} 字节）", written),
        Err(e) => println!("[6] 回 ACK 失败: {e:?}"),
    }

    unsafe {
        let _ = DisconnectNamedPipe(server);
        let _ = CloseHandle(server);
    }
    println!("\n=== 服务端完成 ===");
}
