//! `CMD_MODE_STATUS` 载荷解析：服务端推来的输入模式状态。
//!
//! 线格式（`aurora-ipc::codec::encode_mode_status`，与 Rust 侧同一份定义）：
//!
//! ```text
//! flags:u32 + effective_mode:u32 + labelLen:u32 + label(UTF-8)
//! ```
//!
//! 这是**下行**帧：服务端 → DLL。语言栏据此更新显示。
//!
//! # 为什么放在 policy 而不是 com
//!
//! 解析是纯字节逻辑，与 TSF 无关。放在这里可以在 Linux 上单测，并且能用
//! `aurora-ipc` 的**真编码器**产出字节做对账——跨语言边界的布局必须这么验，
//! 手写近似的测试只能证明「我照自己想的写对了」。

/// 全角位。
pub const FLAG_FULL_WIDTH: u32 = 0x01;
/// 中文标点位。
pub const FLAG_CHINESE_PUNCT: u32 = 0x02;

/// 一次模式状态更新。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeStatus<'a> {
    pub flags: u32,
    /// `0` = 中文，`1` = 英文。
    pub effective_mode: u32,
    pub label: &'a str,
}

impl ModeStatus<'_> {
    /// 是否中文模式。
    pub fn is_chinese(&self) -> bool {
        self.effective_mode == 0
    }

    pub fn is_full_width(&self) -> bool {
        self.flags & FLAG_FULL_WIDTH != 0
    }

    pub fn is_chinese_punct(&self) -> bool {
        self.flags & FLAG_CHINESE_PUNCT != 0
    }
}

/// 解析 `CMD_MODE_STATUS` 的**载荷**（不含 8 字节帧头）。
///
/// 返回 `None` 表示载荷不完整（长度不够 / 字符串越界）。
///
/// ⚠ 解析失败时调用方应**保持原状态不变**：显示错状态比不更新更糟。
pub fn parse(payload: &[u8]) -> Option<ModeStatus<'_>> {
    if payload.len() < 12 {
        return None;
    }
    let flags = u32::from_le_bytes(payload[0..4].try_into().ok()?);
    let effective_mode = u32::from_le_bytes(payload[4..8].try_into().ok()?);
    let label_len = u32::from_le_bytes(payload[8..12].try_into().ok()?) as usize;
    if payload.len() < 12 + label_len {
        return None;
    }
    // 非法 UTF-8 不当错误：标签只是显示文本，用空串回落更稳
    let label = std::str::from_utf8(&payload[12..12 + label_len]).unwrap_or("");
    Some(ModeStatus {
        flags,
        effective_mode,
        label,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(flags: u32, mode: u32, label: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_le_bytes());
        p.extend_from_slice(&mode.to_le_bytes());
        p.extend_from_slice(&(label.len() as u32).to_le_bytes());
        p.extend_from_slice(label.as_bytes());
        p
    }

    #[test]
    fn parses_chinese_status() {
        let p = payload(FLAG_CHINESE_PUNCT, 0, "拼音");
        let s = parse(&p).unwrap();
        assert!(s.is_chinese());
        assert!(s.is_chinese_punct());
        assert!(!s.is_full_width());
        assert_eq!(s.label, "拼音");
    }

    #[test]
    fn parses_english_status() {
        let p = payload(0, 1, "");
        let s = parse(&p).unwrap();
        assert!(!s.is_chinese());
        assert_eq!(s.label, "");
    }

    #[test]
    fn parses_full_width_flag() {
        let p = payload(FLAG_FULL_WIDTH, 0, "");
        let s = parse(&p).unwrap();
        assert!(s.is_full_width());
        assert!(!s.is_chinese_punct());
    }

    #[test]
    fn rejects_short_payload() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0u8; 5]).is_none());
        assert!(parse(&[0u8; 11]).is_none());
    }

    /// 字符串长度声明超出实际载荷 ⇒ 拒绝（不能越界读）。
    #[test]
    fn rejects_truncated_label() {
        let mut p = payload(0, 0, "abcdef");
        p.truncate(p.len() - 2);
        assert!(parse(&p).is_none());
    }

    /// 非法 UTF-8 不 panic，回落空串。
    #[test]
    fn invalid_utf8_falls_back_to_empty() {
        let mut p = Vec::new();
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&2u32.to_le_bytes());
        p.extend_from_slice(&[0xFF, 0xFE]); // 非法 UTF-8
        let s = parse(&p).unwrap();
        assert_eq!(s.label, "");
    }

    /// 声明长度巨大也不能 panic（防御）。
    #[test]
    fn huge_label_len_is_rejected() {
        let mut p = Vec::new();
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        p.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse(&p).is_none());
    }
}
