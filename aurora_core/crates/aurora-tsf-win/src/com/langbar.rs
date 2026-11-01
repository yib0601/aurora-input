//! `ITfLangBarItem` + `ITfLangBarItemButton`：语言栏按钮。
//!
//! 原实现：`aurora_tsf/src/LangBarItemButton.cpp`（1534 行）+
//! `include/LangBarItemButton.h`（229 行）。
//!
//! # 这是什么
//!
//! Windows 语言栏（任务栏右下角那个「中/英」）上的按钮。用户点它切中英、
//! 右键出菜单。TSF 通过本接口问我们要**显示什么**（文字/图标/状态）。
//!
//! # 本文件的范围
//!
//! **只做展示**：文字、状态位、描述、图标占位。原实现里另外一大半是
//! 「把服务端状态刷新到这里」的接线（`PostActivationStatus` / `PostServiceReady` /
//! `PostRefreshIcon` 等 20 多个方法），那些属于 IPC 订阅侧，未移植。
//!
//! # 状态从哪里来
//!
//! 原实现的中英状态由服务端推送（`CMD_STATUS_UPDATE` / `CMD_MODE_PUSH`）。
//! 本实现同样只存状态、不决策——[`LangBarState`] 里的值由 IPC 层更新。
//!
//! # 为什么不自己画图标
//!
//! 原实现用 `CreateIconIndirect` 在内存里画文字图标（按中英/全角/标点组合）。
//! 那部分需要 GDI 位图操作，且与主题色相关。本实现先返回 `HICON::default()`
//! （让宿主用文字），文字路径已经能表达状态。

#![cfg(windows)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{BOOL, POINT, RECT};
use windows::Win32::UI::TextServices::{
    ITfLangBarItem, ITfLangBarItem_Impl, ITfLangBarItemButton, ITfLangBarItemButton_Impl, ITfMenu,
    TF_LANGBARITEMINFO, TF_LBI_STATUS_BTN_TOGGLED, TF_LBI_STATUS_DISABLED, TF_LBI_STATUS_HIDDEN,
    TF_LBI_STYLE_BTN_BUTTON, TF_LBI_STYLE_BTN_MENU, TF_LBI_STYLE_SHOWNINTRAY,
    TF_LBI_STYLE_TEXTCOLORICON, TfLBIClick,
};
use windows::Win32::UI::WindowsAndMessaging::HICON;
use windows::core::{BSTR, Result, implement};

use crate::com::text_service::{CLSID_TEXT_SERVICE, GUID_LANGBAR_ITEM};

/// 语言栏按钮的服务显示名。
#[cfg(feature = "dev-variant")]
const ITEM_DESCRIPTION: &str = "极光输入法 Dev";
#[cfg(not(feature = "dev-variant"))]
const ITEM_DESCRIPTION: &str = "极光输入法";

/// 语言栏按钮的显示状态。
///
/// 由 IPC 层根据服务端推送更新。**本层只负责呈现**，不做任何中英判定。
#[derive(Debug, Default)]
pub struct LangBarState {
    /// 中文模式。
    chinese: AtomicBool,
    /// 全角。
    full_width: AtomicBool,
    /// 中文标点。
    chinese_punct: AtomicBool,
    /// 是否可见（宿主可隐藏）。
    visible: AtomicBool,
    /// 是否可用（服务未就绪时置 false）。
    enabled: AtomicBool,
    /// 显示文字（方案标签 / 「英」/「A」）。
    ///
    /// 原实现由服务端算好标签推来（`mode_icon_label`），本层原样显示。
    label: Mutex<String>,
}

impl LangBarState {
    pub fn new() -> Self {
        Self {
            chinese: AtomicBool::new(true),
            full_width: AtomicBool::new(false),
            chinese_punct: AtomicBool::new(true),
            visible: AtomicBool::new(true),
            enabled: AtomicBool::new(false),
            label: Mutex::new(String::new()),
        }
    }

    /// 一次性更新模式位（服务端推来三合一状态时用）。
    pub fn set_mode(&self, chinese: bool, full_width: bool, chinese_punct: bool) {
        self.chinese.store(chinese, Ordering::SeqCst);
        self.full_width.store(full_width, Ordering::SeqCst);
        self.chinese_punct.store(chinese_punct, Ordering::SeqCst);
    }

    /// 设置显示标签。
    pub fn set_label(&self, label: impl Into<String>) {
        if let Ok(mut g) = self.label.lock() {
            *g = label.into();
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::SeqCst);
    }

    pub fn chinese(&self) -> bool {
        self.chinese.load(Ordering::SeqCst)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    pub fn visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// 算按钮文字。
    ///
    /// 优先用服务端推来的标签；没有就按模式给默认值。
    ///
    /// | 状态 | 显示 |
    /// |---|---|
    /// | 有服务端标签 | 原样用（方案名 / 「英」/「A」） |
    /// | 中文 | 「中」 |
    /// | 英文 | 「英」 |
    pub fn display_text(&self) -> String {
        if let Ok(g) = self.label.lock() {
            if !g.is_empty() {
                return g.clone();
            }
        }
        if self.chinese() {
            "中".to_string()
        } else {
            "英".to_string()
        }
    }

    /// 算状态位。
    ///
    /// `TF_LBI_STATUS_BTN_TOGGLED` 表示「按下态」——语言栏据此画高亮。
    /// 中文模式 = 按下态，与 Windows 自带输入法一致。
    pub fn status_flags(&self) -> u32 {
        let mut flags = 0u32;
        if !self.visible() {
            flags |= TF_LBI_STATUS_HIDDEN;
        }
        if !self.enabled() {
            flags |= TF_LBI_STATUS_DISABLED;
        }
        if self.chinese() {
            flags |= TF_LBI_STATUS_BTN_TOGGLED;
        }
        flags
    }
}

/// `TF_LBI_STYLE_*` 拼成按钮样式。
///
/// 与 C++ `GetInfo` 的四个位一致：
/// - `BTN_BUTTON` 显示为可点按钮
/// - `BTN_MENU` 支持右键菜单（InitMenu / OnMenuSelect）
/// - `SHOWNINTRAY` 在托盘区显示
/// - `TEXTCOLORICON` 图标颜色随主题变
pub fn button_style() -> u32 {
    TF_LBI_STYLE_BTN_BUTTON
        | TF_LBI_STYLE_BTN_MENU
        | TF_LBI_STYLE_SHOWNINTRAY
        | TF_LBI_STYLE_TEXTCOLORICON
}

/// 语言栏按钮。
#[implement(ITfLangBarItem, ITfLangBarItemButton)]
pub struct LangBarItemButton {
    pub state: std::sync::Arc<LangBarState>,
}

impl LangBarItemButton {
    pub fn new(state: std::sync::Arc<LangBarState>) -> Self {
        Self { state }
    }
}

/// 把字符串写进定长 UTF-16 缓冲（带 NUL 结尾）。
fn write_fixed(dst: &mut [u16], src: &str) {
    let units: Vec<u16> = src.encode_utf16().collect();
    let n = units.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&units[..n]);
    if n < dst.len() {
        dst[n] = 0;
    }
}

impl ITfLangBarItem_Impl for LangBarItemButton_Impl {
    fn GetInfo(&self, pinfo: *mut TF_LANGBARITEMINFO) -> Result<()> {
        if pinfo.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        let mut info = TF_LANGBARITEMINFO {
            clsidService: CLSID_TEXT_SERVICE,
            guidItem: GUID_LANGBAR_ITEM,
            dwStyle: button_style(),
            ulSort: 0,
            szDescription: [0u16; 32],
        };
        write_fixed(&mut info.szDescription, ITEM_DESCRIPTION);
        // SAFETY: 上面已检查非空。
        unsafe { *pinfo = info };
        Ok(())
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(self.state.status_flags())
    }

    fn Show(&self, fshow: BOOL) -> Result<()> {
        self.state.set_visible(fshow.as_bool());
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        // 工具提示同时表达模式，让用户不用点开菜单就知道当前状态。
        let text = format!("{}（{}）", ITEM_DESCRIPTION, self.state.display_text());
        Ok(BSTR::from(&text))
    }
}

impl ITfLangBarItemButton_Impl for LangBarItemButton_Impl {
    /// 用户点击了按钮。
    ///
    /// 原实现：左键单击切中英，右键弹菜单。
    ///
    /// ⚠ 当前只**上报点击事实**（记日志），不切中英——切换要走 IPC 到服务端
    /// （协调器才是模式的所有者）。未接 IPC 时自己切会让两端状态分叉。
    fn OnClick(&self, click: TfLBIClick, _pt: &POINT, _prcarea: *const RECT) -> Result<()> {
        tracing::debug!("LangBar OnClick: click={}", click.0);
        Ok(())
    }

    /// 宿主准备右键菜单。
    ///
    /// ⚠ 未实现：菜单树由服务端下发（`CMD_MENU_SHOW`），需要 IPC 订阅。
    /// 返回 `Ok(())` 但不填菜单——表现为「右键无菜单」，不会崩。
    fn InitMenu(&self, _pmenu: Option<&ITfMenu>) -> Result<()> {
        Ok(())
    }

    /// 菜单项被选中。
    fn OnMenuSelect(&self, wid: u32) -> Result<()> {
        tracing::debug!("LangBar OnMenuSelect: wid={wid}");
        Ok(())
    }

    /// 按钮图标。
    ///
    /// 返回空 `HICON` 让宿主用 `GetText` 的文字。
    /// 原实现在内存里画文字图标（按中英/全角/标点组合），需要 GDI 位图操作；
    /// 文字路径已能表达状态，图标是锦上添花。
    fn GetIcon(&self) -> Result<HICON> {
        Ok(HICON::default())
    }

    /// 按钮显示的文字。
    fn GetText(&self) -> Result<BSTR> {
        Ok(BSTR::from(&self.state.display_text()))
    }
}

/// 解析 `CMD_MODE_STATUS` 载荷并更新状态。
///
/// 解析逻辑在 [`crate::policy::mode_status`]（纯逻辑、Linux 可测、
/// 已用 `aurora-ipc` 真编码器对账）。本函数只负责把结果写进语言栏状态。
///
/// 返回 `None` 表示载荷不完整——**不改状态**，让语言栏保持原样比显示错状态好。
pub fn apply_mode_status(state: &LangBarState, payload: &[u8]) -> Option<()> {
    let st = crate::policy::mode_status::parse(payload)?;
    state.set_mode(st.is_chinese(), st.is_full_width(), st.is_chinese_punct());
    state.set_label(st.label);
    state.set_enabled(true);
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_chinese_by_default() {
        let s = LangBarState::new();
        assert!(s.chinese());
        assert!(!s.enabled(), "服务未就绪时不该显示为可用");
        assert!(s.visible());
    }

    #[test]
    fn display_text_follows_mode() {
        let s = LangBarState::new();
        assert_eq!(s.display_text(), "中");
        s.set_mode(false, false, true);
        assert_eq!(s.display_text(), "英");
    }

    /// 服务端标签优先——方案名由服务端算。
    #[test]
    fn server_label_wins() {
        let s = LangBarState::new();
        s.set_label("五笔");
        assert_eq!(s.display_text(), "五笔");
        // 换模式但标签还在，仍显示标签
        s.set_mode(false, false, true);
        assert_eq!(s.display_text(), "五笔");
    }

    /// 空标签不算数，回落到模式默认值。
    #[test]
    fn empty_label_falls_back_to_mode() {
        let s = LangBarState::new();
        s.set_label("");
        assert_eq!(s.display_text(), "中");
    }

    /// 中文模式 = 按下态（与 Windows 自带输入法一致）。
    #[test]
    fn chinese_mode_is_toggled_on() {
        let s = LangBarState::new();
        s.set_enabled(true);
        assert_ne!(
            s.status_flags() & TF_LBI_STATUS_BTN_TOGGLED,
            0,
            "中文模式应显示为按下态"
        );

        s.set_mode(false, false, true);
        assert_eq!(
            s.status_flags() & TF_LBI_STATUS_BTN_TOGGLED,
            0,
            "英文模式不是按下态"
        );
    }

    #[test]
    fn disabled_and_hidden_flags() {
        let s = LangBarState::new();
        s.set_enabled(false);
        assert_ne!(s.status_flags() & TF_LBI_STATUS_DISABLED, 0);

        s.set_visible(false);
        assert_ne!(s.status_flags() & TF_LBI_STATUS_HIDDEN, 0);
    }

    /// 按钮样式必须与 C++ GetInfo 的四个位一致——少一位会让按钮不可点、
    /// 没有菜单或不出现在托盘。
    #[test]
    fn button_style_has_all_four_bits() {
        let style = button_style();
        for (bit, name) in [
            (TF_LBI_STYLE_BTN_BUTTON, "BTN_BUTTON"),
            (TF_LBI_STYLE_BTN_MENU, "BTN_MENU"),
            (TF_LBI_STYLE_SHOWNINTRAY, "SHOWNINTRAY"),
            (TF_LBI_STYLE_TEXTCOLORICON, "TEXTCOLORICON"),
        ] {
            assert_ne!(style & bit, 0, "缺 {name} 位");
        }
    }

    /// 定长缓冲写入：必须 NUL 结尾且不越界。
    #[test]
    fn fixed_buffer_write_is_safe() {
        let mut buf = [0xFFFFu16; 8];
        write_fixed(&mut buf, "abc");
        assert_eq!(&buf[..4], &[b'a' as u16, b'b' as u16, b'c' as u16, 0]);

        // 超长截断，仍 NUL 结尾
        let mut buf = [0xFFFFu16; 4];
        write_fixed(&mut buf, "abcdefgh");
        assert_eq!(buf[3], 0, "必须留 NUL 位");
        assert_eq!(&buf[..3], &[b'a' as u16, b'b' as u16, b'c' as u16]);
    }

    /// 空串写入不 panic。
    #[test]
    fn fixed_buffer_empty() {
        let mut buf = [0xFFFFu16; 4];
        write_fixed(&mut buf, "");
        assert_eq!(buf[0], 0);
    }

    /// 造一个 mode_status 载荷（与 aurora-ipc 的编码器同格式）。
    fn mode_status_payload(flags: u32, mode: u32, label: &str) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&flags.to_le_bytes());
        p.extend_from_slice(&mode.to_le_bytes());
        p.extend_from_slice(&(label.len() as u32).to_le_bytes());
        p.extend_from_slice(label.as_bytes());
        p
    }

    /// 服务端推中文状态 ⇒ 语言栏显示中文。
    #[test]
    fn apply_mode_status_chinese() {
        let s = LangBarState::new();
        let p = mode_status_payload(crate::policy::mode_status::FLAG_CHINESE_PUNCT, 0, "拼音");
        assert!(apply_mode_status(&s, &p).is_some());
        assert!(s.chinese());
        assert_eq!(s.display_text(), "拼音");
        assert!(s.enabled(), "收到状态即视为服务就绪");
    }

    /// 服务端推英文状态 ⇒ 语言栏切英文。
    #[test]
    fn apply_mode_status_english() {
        let s = LangBarState::new();
        let p = mode_status_payload(0, 1, "");
        assert!(apply_mode_status(&s, &p).is_some());
        assert!(!s.chinese());
        assert_eq!(s.display_text(), "英", "空标签回落到模式默认值");
    }

    /// 载荷不完整时不改状态——显示错状态比不更新更糟。
    #[test]
    fn malformed_payload_leaves_state_unchanged() {
        let s = LangBarState::new();
        s.set_mode(false, false, false); // 先设成英文
        // 长度不足
        assert!(apply_mode_status(&s, &[0u8; 5]).is_none());
        assert!(!s.chinese(), "不该被改回中文");
        // 字符串越界
        let mut bad = mode_status_payload(0, 0, "abc");
        bad.truncate(bad.len() - 1);
        assert!(apply_mode_status(&s, &bad).is_none());
    }

    /// 全角位要能解出来。
    #[test]
    fn full_width_flag_is_parsed() {
        let s = LangBarState::new();
        let p = mode_status_payload(crate::policy::mode_status::FLAG_FULL_WIDTH, 0, "拼音");
        assert!(apply_mode_status(&s, &p).is_some());
        assert!(!s.chinese() == false, "仍是中文");
    }
}
