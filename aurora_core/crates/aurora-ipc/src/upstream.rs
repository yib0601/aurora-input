//! **上行帧**（前端 → 服务）的编码器。
//!
//! # 为什么单开一个模块
//!
//! `codec.rs` 里的编码器几乎全是**下行**（服务 → 前端）：服务端是下行方向的作者。
//! 而上行方向的作者是三个前端（TSF / Fcitx5 / IMKit），服务端只解码，所以 `codec.rs`
//! 长期只有对应的 `decode_*`，没有 `encode_*`。
//!
//! 这在 C++ 版里不是问题——前端自己拼帧。但协议有四个实现时，"谁都能编"就意味着
//! "谁都会编错"：字节布局在 C++ / Swift 各写一遍，错了不报错、只表现为某类消息静默
//! 失效或字段乱码。线格式真值表（`testdata/wire_vectors.json`）已经钉住了现有帧，
//! 但**没有编码器就没法复用**，前端还是得手写。
//!
//! 本模块把上行帧的编码收进 SSOT，前端直接调用。新增上行帧时：布局写这里、
//! 真值表加一条、三端消费同一份。
//!
//! # 兼容约定
//!
//! 上行帧多处允许**变长兼容**（服务端按 payload 长度分支）。编码器一律产出**最新版本**，
//! 旧服务端按长度读它认识的前缀部分。不要在编码器里做「按服务端能力裁剪」——那是服务端
//! 的事，客户端无从而知。

use crate::codec::frame;
use crate::protocol::*;

/// `CaretPayload` 的线格式（20 字节）。
///
/// 结构体本身只有 `from_bytes`（服务端只解不编）；编码是前端的事，故放在本模块。
fn caret_bytes(c: &CaretPayload) -> [u8; 20] {
    let mut b = [0u8; 20];
    for (i, v) in [
        c.x,
        c.y,
        c.height,
        c.composition_start_x,
        c.composition_start_y,
    ]
    .into_iter()
    .enumerate()
    {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// `FocusGainedPayload` 的 39 字节定长段。
fn focus_gained_fixed_bytes(p: &FocusGainedPayload) -> [u8; 39] {
    let mut b = [0u8; 39];
    b[0..20].copy_from_slice(&caret_bytes(&p.caret));
    b[20..28].copy_from_slice(&p.client_token.to_le_bytes());
    b[28..36].copy_from_slice(&p.input_scope_mask.to_le_bytes());
    b[36] = p.disabled;
    b[37] = p.reason;
    b[38] = p.caret_source as u8;
    b
}

// ── 按键事件（最高频，18 字节定长）──────────────────────────────────────

/// 编码 `CMD_KEY_EVENT` 上行帧。
///
/// payload = `KeyPayload`(18B)：keyCode:u32 + scanCode:u32 + modifiers:u32 +
/// eventType:u8 + toggles:u8 + eventSeq:u16 + prevChar:u16。
pub fn encode_key_event(payload: &KeyPayload) -> Vec<u8> {
    frame(CMD_KEY_EVENT, payload.to_bytes().to_vec())
}

// ── 光标位置（20 / 24 / 40 字节三档，服务端按长度分支）──────────────────

// 光标来源的常量定义在 `protocol::caret_source`（那边还有 PRE_REFLOW 等扩展值），
// 这里 re-export 一份，调用方 `upstream::caret_source::*` 与 `protocol::caret_source::*` 等价。
pub use crate::protocol::caret_source;

/// 编码 `CMD_CARET_UPDATE` 上行帧（**v2，24 字节**：`CaretPayload` + source）。
pub fn encode_caret_update(caret: &CaretPayload, source: i32) -> Vec<u8> {
    let mut p = Vec::with_capacity(24);
    p.extend_from_slice(&caret_bytes(caret));
    p.extend_from_slice(&source.to_le_bytes());
    frame(CMD_CARET_UPDATE, p)
}

/// 编码 `CMD_CARET_UPDATE` 上行帧（**v3，40 字节**：v2 + 组合范围包围矩形）。
///
/// `comp_rect` 四值全 0 表示「未取到」（服务端据此判定本帧没有组合矩形）。
///
/// ⚠ 组合矩形当前**只上报、不参与任何决策**：各宿主对跨行 range 的 `GetTextExt` 行为
/// 尚未逐一验证（规范说返回包围矩形，实现可能只返回首行、可能失败）。先用真机日志看清
/// 各宿主的真实返回值。
pub fn encode_caret_update_v3(
    caret: &CaretPayload,
    source: i32,
    comp_rect: (i32, i32, i32, i32),
) -> Vec<u8> {
    let mut p = Vec::with_capacity(40);
    p.extend_from_slice(&caret_bytes(&caret.clone()));
    p.extend_from_slice(&source.to_le_bytes());
    for v in [comp_rect.0, comp_rect.1, comp_rect.2, comp_rect.3] {
        p.extend_from_slice(&v.to_le_bytes());
    }
    frame(CMD_CARET_UPDATE, p)
}

// ── 焦点 ────────────────────────────────────────────────────────────────

/// 编码 `CMD_FOCUS_GAINED` 上行帧（39 字节定长段 + 三个变长段）。
///
/// 布局：`[39B FocusGainedPayload][bundleIdLen:u32][bundleId][windowClassLen:u32]
/// [windowClass][windowTitleLen:u32][windowTitle]`
///
/// # 为什么 bundleId 段在 Windows 上也要发（len=0 占位）
///
/// 不是冗余，是让两个平台共用同一条线性走法——否则 windowClass 段的偏移会因平台而异。
///
/// # ⚠ 必须发满 39 字节
///
/// 服务端 `FocusGainedPayload::from_bytes` 的下限是 39（旧 38）。发短了解码恒失败，
/// 焦点重型段（按应用初始模式 / 密码框强制英文 / 焦点状态气泡）整段不执行。
pub fn encode_focus_gained(
    payload: &FocusGainedPayload,
    bundle_id: &str,
    window_class: &str,
    window_title: &str,
) -> Vec<u8> {
    let fixed = focus_gained_fixed_bytes(payload);
    let mut p = Vec::with_capacity(
        fixed.len() + 12 + bundle_id.len() + window_class.len() + window_title.len(),
    );
    p.extend_from_slice(&fixed);
    for seg in [bundle_id, window_class, window_title] {
        p.extend_from_slice(&(seg.len() as u32).to_le_bytes());
        p.extend_from_slice(seg.as_bytes());
    }
    frame(CMD_FOCUS_GAINED, p)
}

/// 编码 `CMD_FOCUS_LOST` 上行帧（9 字节：clientToken:u64 + reason:u8）。
///
/// ⚠ 注意偏移：这是**紧凑布局**（pack(1)），`u8` 紧跟 `u64`，没有对齐填充。
pub fn encode_focus_lost(client_token: u64, reason: u8) -> Vec<u8> {
    let mut p = Vec::with_capacity(9);
    p.extend_from_slice(&client_token.to_le_bytes());
    p.push(reason);
    frame(CMD_FOCUS_LOST, p)
}

// ── 选区变化（4 字节）───────────────────────────────────────────────────

/// 编码 `CMD_SELECTION_CHANGED` 上行帧。
///
/// payload = `prevChar:u16 + reserved:u16`（共 4 字节）。
/// 从 `ITfTextEditSink::OnEndEdit` 发出，告知服务端「光标移出了组合范围」（如鼠标点击）。
pub fn encode_selection_changed(prev_char: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(4);
    p.extend_from_slice(&prev_char.to_le_bytes());
    p.extend_from_slice(&0u16.to_le_bytes()); // reserved，留零
    frame(CMD_SELECTION_CHANGED, p)
}

// ── 提交请求（12 字节 + 变长输入缓冲）───────────────────────────────────

/// 编码 `CMD_COMMIT_REQUEST` 上行帧。
///
/// payload = `barrierSeq:u16 + triggerKey:u16 + modifiers:u32 + inputLength:u32 + input(UTF-8)`
///
/// `input_length` 是**字节数**（UTF-8），不是字符数。
pub fn encode_commit_request(
    barrier_seq: u16,
    trigger_key: u16,
    modifiers: u32,
    input: &str,
) -> Vec<u8> {
    let ib = input.as_bytes();
    let mut p = Vec::with_capacity(12 + ib.len());
    p.extend_from_slice(&barrier_seq.to_le_bytes());
    p.extend_from_slice(&trigger_key.to_le_bytes());
    p.extend_from_slice(&modifiers.to_le_bytes());
    p.extend_from_slice(&(ib.len() as u32).to_le_bytes());
    p.extend_from_slice(ib);
    frame(CMD_COMMIT_REQUEST, p)
}

// ── 无载荷上行帧 ────────────────────────────────────────────────────────

/// 编码 `CMD_IME_ACTIVATED`（**空载荷**，不带异步标志）。
pub fn encode_ime_activated() -> Vec<u8> {
    frame(CMD_IME_ACTIVATED, Vec::new())
}

/// 编码 `CMD_IME_DEACTIVATED`（空载荷）。
pub fn encode_ime_deactivated() -> Vec<u8> {
    frame(CMD_IME_DEACTIVATED, Vec::new())
}

/// 编码 `CMD_COMPOSITION_TERMINATED`（空载荷）。
pub fn encode_composition_terminated() -> Vec<u8> {
    frame(CMD_COMPOSITION_TERMINATED, Vec::new())
}

/// 编码 `CMD_CARET_PENDING`（空载荷）。
pub fn encode_caret_pending() -> Vec<u8> {
    frame(CMD_CARET_PENDING, Vec::new())
}

/// 编码 `CMD_MODE_NOTIFY` 上行帧（payload = chineseMode:u8 + clearInput:u8）。
pub fn encode_mode_notify(chinese_mode: bool, clear_input: bool) -> Vec<u8> {
    let p = vec![u8::from(chinese_mode), u8::from(clear_input)];
    frame(CMD_MODE_NOTIFY, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_caret() -> CaretPayload {
        CaretPayload {
            x: 0,
            y: 0,
            height: 0,
            composition_start_x: 0,
            composition_start_y: 0,
        }
    }
    use crate::codec::{decode_header, decode_key_payload};

    /// 所有上行帧的头部都必须自洽：命令码对、length 等于实际载荷长度。
    ///
    /// C++ 侧读帧时先信 `header.length` 再切载荷。这个字段写错，前端会去读帧外的内存
    /// 或截断一帧——真机上表现为随机丢字，几乎无法从日志反推。
    fn assert_frame_well_formed(f: &[u8], want_cmd: u16) {
        assert!(f.len() >= IpcHeader::SIZE);
        let h = decode_header(f).expect("头部不合法");
        let (cmd, len) = (h.command, h.length);
        assert_eq!(cmd, want_cmd);
        assert_eq!(
            len as usize,
            f.len() - IpcHeader::SIZE,
            "length 与实际载荷不符"
        );
        assert!(!h.is_async(), "上行同步帧不带异步标志");
    }

    #[test]
    fn key_event_frame() {
        let p = KeyPayload {
            key_code: 0x41,
            scan_code: 0x1E,
            modifiers: 0x05,
            event_type: 0,
            toggles: 0x03,
            event_seq: 0x1234,
            prev_char: 0x4E2D,
        };
        let f = encode_key_event(&p);
        assert_frame_well_formed(&f, CMD_KEY_EVENT);
        assert_eq!(f.len(), 8 + 18);
        // 载荷能被解码器吃回去
        let back = decode_key_payload(&f[8..]).expect("解不回来");
        let (kc, pc) = (back.key_code, back.prev_char);
        assert_eq!(kc, 0x41);
        assert_eq!(pc, 0x4E2D);
    }

    #[test]
    fn caret_update_v2_is_24_bytes() {
        let c = CaretPayload {
            x: 10,
            y: 20,
            height: 16,
            composition_start_x: 5,
            composition_start_y: 18,
        };
        let f = encode_caret_update(&c, caret_source::TSF_SELECTION);
        assert_frame_well_formed(&f, CMD_CARET_UPDATE);
        assert_eq!(f.len() - 8, 24);
    }

    #[test]
    fn caret_update_v3_is_40_bytes() {
        let c = CaretPayload {
            x: 10,
            y: 20,
            height: 16,
            composition_start_x: 5,
            composition_start_y: 18,
        };
        let f = encode_caret_update_v3(&c, caret_source::TSF_SELECTION, (5, 18, 100, 34));
        assert_frame_well_formed(&f, CMD_CARET_UPDATE);
        assert_eq!(f.len() - 8, 40);
    }

    #[test]
    fn focus_lost_is_9_bytes_compact() {
        let f = encode_focus_lost(0x1122334455667788, 1);
        assert_frame_well_formed(&f, CMD_FOCUS_LOST);
        assert_eq!(f.len() - 8, 9, "pack(1)：u8 紧跟 u64，无对齐填充");
        assert_eq!(&f[8..16], &0x1122334455667788u64.to_le_bytes());
        assert_eq!(f[16], 1);
    }

    #[test]
    fn selection_changed_is_4_bytes() {
        let f = encode_selection_changed(0x33);
        assert_frame_well_formed(&f, CMD_SELECTION_CHANGED);
        assert_eq!(f.len() - 8, 4);
        assert_eq!(&f[8..10], &0x33u16.to_le_bytes());
        assert_eq!(&f[10..12], &[0, 0], "reserved 留零");
    }

    #[test]
    fn commit_request_layout() {
        let f = encode_commit_request(7, 0x20, 0x02, "nihao");
        assert_frame_well_formed(&f, CMD_COMMIT_REQUEST);
        let p = &f[8..];
        assert_eq!(p.len(), 12 + 5);
        assert_eq!(u16::from_le_bytes([p[0], p[1]]), 7, "barrierSeq");
        assert_eq!(u16::from_le_bytes([p[2], p[3]]), 0x20, "triggerKey=空格");
        assert_eq!(
            u32::from_le_bytes([p[4], p[5], p[6], p[7]]),
            0x02,
            "modifiers"
        );
        assert_eq!(
            u32::from_le_bytes([p[8], p[9], p[10], p[11]]),
            5,
            "inputLength"
        );
        assert_eq!(&p[12..], b"nihao");
    }

    /// 输入缓冲是 UTF-8：length 必须记字节数，不是字符数。
    #[test]
    fn commit_request_length_is_bytes_not_chars() {
        let f = encode_commit_request(1, 0x20, 0, "你好");
        let p = &f[8..];
        assert_eq!(
            u32::from_le_bytes([p[8], p[9], p[10], p[11]]),
            6,
            "「你好」是 6 字节 UTF-8，不是 2"
        );
    }

    #[test]
    fn empty_payload_frames() {
        for (f, cmd) in [
            (encode_ime_activated(), CMD_IME_ACTIVATED),
            (encode_ime_deactivated(), CMD_IME_DEACTIVATED),
            (encode_composition_terminated(), CMD_COMPOSITION_TERMINATED),
            (encode_caret_pending(), CMD_CARET_PENDING),
        ] {
            assert_frame_well_formed(&f, cmd);
            assert_eq!(f.len(), 8, "空载荷：只有 8 字节头");
        }
    }

    #[test]
    fn mode_notify_payload() {
        let f = encode_mode_notify(true, false);
        assert_frame_well_formed(&f, CMD_MODE_NOTIFY);
        assert_eq!(&f[8..], &[1, 0]);
    }

    #[test]
    fn focus_gained_includes_three_variable_segments() {
        let payload = FocusGainedPayload {
            caret: zero_caret(),
            client_token: 0xABCD,
            input_scope_mask: 0,
            disabled: 0,
            reason: 0,
            caret_source: caret_source::UNKNOWN,
        };
        let f = encode_focus_gained(&payload, "", "", "");
        assert_frame_well_formed(&f, CMD_FOCUS_GAINED);
        // 39 定长 + 3 个长度前缀（各 4 字节）
        assert_eq!(f.len() - 8, 39 + 12, "空串段也发长度前缀（保持线性走法）");
    }

    /// 三个变长段的长度前缀必须能顺序读出。
    #[test]
    fn focus_gained_segments_are_walkable() {
        let payload = FocusGainedPayload {
            caret: zero_caret(),
            client_token: 1,
            input_scope_mask: 0,
            disabled: 0,
            reason: 0,
            caret_source: caret_source::UNKNOWN,
        };
        let f = encode_focus_gained(&payload, "com.app", "CabinetWClass", "标题");
        let p = &f[8..];

        let mut off = 39;
        let mut segs = Vec::new();
        for _ in 0..3 {
            let len = u32::from_le_bytes([p[off], p[off + 1], p[off + 2], p[off + 3]]) as usize;
            off += 4;
            segs.push(std::str::from_utf8(&p[off..off + len]).expect("非法 UTF-8"));
            off += len;
        }
        assert_eq!(segs, vec!["com.app", "CabinetWClass", "标题"]);
        assert_eq!(off, p.len(), "三段读完应恰好到帧末");
    }
}
