//! 上行帧编码器的等价性测试（oracle 由 g++ 按 C++ 布局生成）。
//!
//! 上行帧的作者是前端（TSF / Fcitx5 / IMKit），服务端只解码——所以这些编码器此前
//! 只存在于各前端的 C++/Swift 里，各写一遍。本测试钉住 Rust 版与 C++ 版**逐字节一致**。
//!
//! oracle 生成方式见 `aurora_core/crates/aurora-ipc/tests/upstream_frames_match_cpp.rs`
//! 的注释；C++ 侧用 `__attribute__((packed))` 复刻 `#pragma pack(push,1)` 的结构体，
//! 走与 `CIPCClient::_SendBinaryMessage` 相同的组帧逻辑。

use aurora_ipc::upstream::*;
use aurora_ipc::*;

fn hex(f: &[u8]) -> String {
    f.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn key_event_matches_cpp_bytes() {
    let p = KeyPayload {
        key_code: 0x41,
        scan_code: 0x1E,
        modifiers: 0x05,
        event_type: 0,
        toggles: 3,
        event_seq: 0x1234,
        prev_char: 0x4E2D,
    };
    // C++: 01100101 12000000 41000000 1e000000 05000000 0003 3412 2d4e
    assert_eq!(
        hex(&encode_key_event(&p)),
        "0110010112000000410000001e00000005000000000334122d4e"
    );
}

#[test]
fn caret_update_v2_matches_cpp_bytes() {
    let c = CaretPayload {
        x: 10,
        y: 20,
        height: 16,
        composition_start_x: 5,
        composition_start_y: 18,
    };
    // C++: 01100103 18000000 0a000000 14000000 10000000 05000000 12000000 01000000
    assert_eq!(
        hex(&encode_caret_update(&c, caret_source::TSF_SELECTION)),
        "01100103180000000a0000001400000010000000050000001200000001000000"
    );
}

#[test]
fn focus_lost_matches_cpp_bytes() {
    // C++: 01100202 09000000 8877665544332211 01  —— 紧凑布局，无填充
    assert_eq!(
        hex(&encode_focus_lost(0x1122334455667788, 1)),
        "0110020209000000887766554433221101"
    );
}

#[test]
fn selection_changed_matches_cpp_bytes() {
    // C++: 01100203 04000000 3300 0000
    assert_eq!(
        hex(&encode_selection_changed(0x33)),
        "011002030400000033000000"
    );
}

#[test]
fn mode_notify_matches_cpp_bytes() {
    // C++: 01100502 02000000 0100
    assert_eq!(
        hex(&encode_mode_notify(true, false)),
        "01100502020000000100"
    );
}

#[test]
fn empty_frames_match_cpp_bytes() {
    // C++: 01100302 00000000
    assert_eq!(hex(&encode_ime_activated()), "0110030200000000");
}
