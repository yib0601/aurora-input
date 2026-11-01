//! `CTextService` 的 COM 骨架：TSF 的主入口对象。
//!
//! 原实现：`aurora_tsf/src/TextService.cpp`（8185 行）+ `include/TextService.h`（963 行）。
//!
//! # 本文件的范围
//!
//! **只实现让 DLL 能被系统加载、激活、卸载的最小闭环**，不搬业务逻辑：
//!
//! - `ITfTextInputProcessor::Activate` / `Deactivate` —— 生命周期
//! - `IUnknown` —— 引用计数（由 `#[implement]` 生成）
//!
//! 其余 29 个接口（`ITfKeyEventSink` / `ITfThreadMgrEventSink` / `ITfTextEditSink` /
//! `ITfLangBarItemButton` / `ITfCandidateListUIElement` …）逐个另起文件，一次搬一个，
//! 每搬一个就删掉 C++ 版对应实现。
//!
//! # 为什么先只做生命周期
//!
//! `Activate` 能跑通意味着：DLL 被加载 → `DllGetClassObject` 返回工厂 →
//! `CreateInstance` 成功 → COM 对象能 `QueryInterface` → 能注册进 TSM 并收到
//! `Activate`。这整条链是 TSF 输入法的**最低上线条件**，也是历史故障最集中的地方
//! （`6dbc8595`：交叉编译的 DLL 就在这一步 COM 激活失败）。
//!
//! 先把它独立验证，后续每加一个 sink 都有可回归的基线。

#![cfg(windows)]

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use windows::Win32::UI::TextServices::{
    ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Impl, ITfLangBarItem,
    ITfTextInputProcessor, ITfTextInputProcessor_Impl, ITfTextInputProcessorEx,
    ITfTextInputProcessorEx_Impl, ITfThreadMgr,
};
use windows::core::{Interface, Result, implement};

use crate::com::dll::{dll_add_ref, dll_release};
use crate::com::events::{FocusState, ThreadFocusSink, ThreadMgrEventSink};
use crate::com::key_event_sink::{KeyEventSink, KeySinkState};
use crate::com::text_edit_sink::{EditState, TextEditSink};

/// 本输入法的 CLSID。
///
/// 值必须与安装脚本、注册表写入、`wind_tsf.def` 里的完全一致——改这里等于换一个
/// 输入法，已安装的机器全部失效。
///
/// 两个变体用**不同段的 GUID**（正式 EE3x / 开发 DEBx），使两者可同时安装、
/// 互不干扰。对位 `Globals.cpp` 的 `#ifdef AURORA_DEV_VARIANT`。
#[cfg(feature = "dev-variant")]
pub const CLSID_TEXT_SERVICE: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2_DEB0_5C57_45A2_9C63_FB54B34FD90A);
#[cfg(not(feature = "dev-variant"))]
pub const CLSID_TEXT_SERVICE: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2EE30_5C57_45A2_9C63_FB54B34FD90A);

/// 语言 Profile 的 GUID（同一个输入法的「哪一种配置模式」）。
#[cfg(feature = "dev-variant")]
pub const GUID_PROFILE: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2_DEB1_5C57_45A2_9C63_FB54B34FD90A);
#[cfg(not(feature = "dev-variant"))]
pub const GUID_PROFILE: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2EE31_5C57_45A2_9C63_FB54B34FD90A);

/// 语言栏按钮的 GUID。
#[cfg(feature = "dev-variant")]
pub const GUID_LANGBAR_ITEM: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2_DEB2_5C57_45A2_9C63_FB54B34FD90A);
#[cfg(not(feature = "dev-variant"))]
pub const GUID_LANGBAR_ITEM: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2EE32_5C57_45A2_9C63_FB54B34FD90A);

/// 显示属性（已转换）的 GUID。
#[cfg(feature = "dev-variant")]
pub const GUID_DISPLAY_ATTRIBUTE_CONVERTED: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2_DEB4_5C57_45A2_9C63_FB54B34FD90A);
#[cfg(not(feature = "dev-variant"))]
pub const GUID_DISPLAY_ATTRIBUTE_CONVERTED: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2EE34_5C57_45A2_9C63_FB54B34FD90A);

/// 显示属性（输入中）的 GUID。
#[cfg(feature = "dev-variant")]
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2_DEB3_5C57_45A2_9C63_FB54B34FD90A);
#[cfg(not(feature = "dev-variant"))]
pub const GUID_DISPLAY_ATTRIBUTE_INPUT: windows::core::GUID =
    windows::core::GUID::from_u128(0x99C2EE33_5C57_45A2_9C63_FB54B34FD90A);

/// 每个实例的令牌：服务端靠它区分窗口。
///
/// 单调递增 + 高 32 位放进程 id，使「同一进程里的多个实例」与「不同进程的实例」
/// 都不会撞。原实现用一个静态计数器加进程 id 拼成，这里同理。
static NEXT_INSTANCE: AtomicU32 = AtomicU32::new(0);

fn make_client_token() -> u64 {
    let seq = NEXT_INSTANCE.fetch_add(1, Ordering::SeqCst) as u64;
    let pid = std::process::id() as u64;
    (pid << 32) | (seq & 0xFFFF_FFFF)
}

/// TSF 文本服务对象。
///
/// 生命周期：系统 `CreateInstance` → `Activate` → （使用）→ `Deactivate` → 释放。
/// 一个实例对应**一个线程**（TSF 按线程激活输入法）。
#[implement(
    ITfTextInputProcessor,
    ITfTextInputProcessorEx,
    ITfDisplayAttributeProvider
)]
pub struct TextService {
    /// 本实例的唯一令牌。服务端用它区分「哪个窗口在打字」。
    client_token: AtomicU64,
    /// 是否已激活。`Activate` 之后、`Deactivate` 之前为真。
    activated: std::sync::atomic::AtomicBool,
    /// 记录最近一次 `Activate` 收到的 TSM 指针（仅作实例标识，不解引用）。
    thread_mgr: std::sync::Mutex<Option<usize>>,
    /// 焦点与文档事件记账，与两个 sink 共享。
    pub(crate) focus: std::sync::Arc<FocusState>,
    /// 按键状态（修饰键、事件序号），与 KeyEventSink 共享。
    pub(crate) key_state: std::sync::Arc<KeySinkState>,
    /// 编辑事件状态（最近光标），与 TextEditSink 共享。
    pub(crate) edit_state: std::sync::Arc<EditState>,
    /// 语言栏按钮状态。
    pub(crate) langbar_state: std::sync::Arc<crate::com::langbar::LangBarState>,
    /// 语言栏按钮对象（AddItem 后要保持存活，否则宿主引用悬空）。
    langbar_item: std::sync::Mutex<Option<ITfLangBarItem>>,
    /// `ITfSource::AdviseSink` 返回的 cookie，`Deactivate` 时要 `UnadviseSink`。
    ///
    /// 不注销的后果：系统会继续往一个已释放的对象投递事件 → 进程崩溃。
    sink_cookies: std::sync::Mutex<Vec<(windows::core::GUID, u32)>>,
}

impl TextService {
    /// 创建一个未激活的实例。
    pub fn create() -> Self {
        dll_add_ref();
        Self {
            client_token: AtomicU64::new(make_client_token()),
            activated: std::sync::atomic::AtomicBool::new(false),
            thread_mgr: std::sync::Mutex::new(None),
            focus: std::sync::Arc::new(FocusState::new()),
            key_state: std::sync::Arc::new(KeySinkState::new()),
            edit_state: std::sync::Arc::new(EditState::new()),
            langbar_state: std::sync::Arc::new(crate::com::langbar::LangBarState::new()),
            langbar_item: std::sync::Mutex::new(None),
            sink_cookies: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// 本实例的令牌（诊断与服务端区分用）。
    pub fn client_token(&self) -> u64 {
        self.client_token.load(Ordering::SeqCst)
    }

    /// 是否已激活。
    pub fn is_activated(&self) -> bool {
        self.activated.load(Ordering::SeqCst)
    }
}

impl Drop for TextService {
    fn drop(&mut self) {
        dll_release();
    }
}

impl ITfTextInputProcessor_Impl for TextService_Impl {
    /// 系统激活本输入法。
    ///
    /// 后续实现在这里做的事（按原 C++ 顺序，本次未搬）：
    ///
    /// 1. 存 `ptim`（`ITfThreadMgr`）与 `tid`（`TfClientId`）
    /// 2. `ITfSource::AdviseSink(IID_ITfThreadMgrEventSink)` 挂焦点事件
    /// 3. 建 `ITfKeyEventSink` 并 `AdviseSink` 挂按键
    /// 4. 建语言栏按钮、显示属性提供者
    /// 5. 发 `IME_ACTIVATED` 给服务端（首个握手）
    ///
    /// 现在只做 1 + 标记激活状态：够让系统认为「激活成功」并继续 `Deactivate`。
    fn Activate(&self, ptim: Option<&ITfThreadMgr>, _tid: u32) -> Result<()> {
        use crate::com::langbar::LangBarItemButton;
        use windows::Win32::UI::TextServices::ITfTextEditSink as ITfTextEditSinkImpl;
        use windows::Win32::UI::TextServices::{
            ITfKeyEventSink, ITfSource, ITfTextEditSink, ITfThreadFocusSink, ITfThreadMgrEventSink,
        };
        use windows::Win32::UI::TextServices::{ITfLangBarItem, ITfLangBarItemMgr};

        let Some(ptim) = ptim else {
            // 没有 TSM 就无法挂任何 sink。原实现在这里会失败。
            return Err(windows::Win32::Foundation::E_INVALIDARG.into());
        };

        if let Ok(mut g) = self.thread_mgr.lock() {
            *g = Some(ptim.as_raw() as usize);
        }

        // 挂两个事件 sink。任何一步失败都回滚已挂的——否则 Deactivate 时
        // cookie 对不上，或者残留 sink 指向已释放对象。
        let source: ITfSource = ptim.cast()?;
        let mut cookies = Vec::new();

        let mgr_sink: ITfThreadMgrEventSink = ThreadMgrEventSink::new(self.focus.clone()).into();
        match unsafe { source.AdviseSink(&ITfThreadMgrEventSink::IID, &mgr_sink) } {
            Ok(cookie) => cookies.push((ITfThreadMgrEventSink::IID, cookie)),
            Err(e) => {
                tracing::warn!("挂 ITfThreadMgrEventSink 失败: {e}");
                return Err(e);
            }
        }

        let focus_sink: ITfThreadFocusSink = ThreadFocusSink::new(self.focus.clone()).into();
        match unsafe { source.AdviseSink(&ITfThreadFocusSink::IID, &focus_sink) } {
            Ok(cookie) => cookies.push((ITfThreadFocusSink::IID, cookie)),
            Err(e) => {
                // 回滚：注销已挂的 mgr sink
                tracing::warn!("挂 ITfThreadFocusSink 失败: {e}，回滚");
                if let Some(&(_iid, c)) = cookies.first() {
                    let _ = unsafe { source.UnadviseSink(c) };
                }
                return Err(e);
            }
        }

        // 按键 sink：TSF 的按键入口。
        //
        // ⚠ 挂不上时**只记警告不中断**——与 ITfThreadFocusSink 同策略。
        // 理由：按键 sink 挂在 ITfSource 上，而某些宿主/输入法框架（含 Wine 的
        // msctf 实现）不支持这个 IID。此时输入法仍然能被激活、能收焦点事件，
        // 只是按键走不到我们这里。中断整个 Activate 会让输入法完全无法启用，
        // 比「按键不通」更糟。
        //
        // ⚠ 但这意味着**不要假设按键一定会来**：真正的输入法实现里，按键通路
        // 不通就等于输入法不可用。此处的容错只是为了不阻断 Activate 本身，
        // 上层（或日志）需要能看出按键 sink 缺失。
        let key_sink: ITfKeyEventSink = KeyEventSink::new(self.key_state.clone()).into();
        match unsafe { source.AdviseSink(&ITfKeyEventSink::IID, &key_sink) } {
            Ok(cookie) => {
                cookies.push((ITfKeyEventSink::IID, cookie));
            }
            Err(e) => {
                tracing::warn!("挂 ITfKeyEventSink 失败: {e}（按键通路不可用，但继续激活）");
            }
        }

        // 编辑事件 sink：光标位置来源。失败不中断（同按键 sink 的策略）。
        let edit_sink: ITfTextEditSinkImpl = TextEditSink::new(self.edit_state.clone()).into();
        match unsafe { source.AdviseSink(&ITfTextEditSink::IID, &edit_sink) } {
            Ok(cookie) => cookies.push((ITfTextEditSink::IID, cookie)),
            Err(e) => {
                tracing::warn!("挂 ITfTextEditSink 失败: {e}（光标位置不可用，但继续激活）");
            }
        }

        // 语言栏按钮：注册到 ITfLangBarItemMgr。失败不中断（同其他可选能力）。
        match ptim.cast::<ITfLangBarItemMgr>() {
            Ok(mgr) => {
                let item = LangBarItemButton::new(self.langbar_state.clone());
                let item_iface: ITfLangBarItem = item.into();
                match unsafe { mgr.AddItem(&item_iface) } {
                    Ok(()) => {
                        // ⚠ 必须保住这个引用：AddItem 不增加引用计数，
                        // 我们释放后宿主持有的指针就悬空了。
                        if let Ok(mut g) = self.langbar_item.lock() {
                            *g = Some(item_iface);
                        }
                        tracing::info!("语言栏按钮已注册");
                    }
                    Err(e) => tracing::warn!("语言栏 AddItem 失败: {e}"),
                }
            }
            Err(e) => tracing::warn!("拿不到 ITfLangBarItemMgr: {e}（无语言栏按钮）"),
        }

        if let Ok(mut g) = self.sink_cookies.lock() {
            *g = cookies;
        }
        self.activated.store(true, Ordering::SeqCst);
        tracing::info!(
            "Activate 完成：{} 个 sink 已挂",
            self.sink_cookies.lock().map(|g| g.len()).unwrap_or(0)
        );
        Ok(())
    }

    /// 系统停用本输入法。
    ///
    /// 必须**幂等**：系统可能在我们已停用后再调一次（切换输入法、进程退出）。
    /// 后续实现要在这里逐个 `UnadviseSink`、发 `IME_DEACTIVATED`、清组合态。
    fn Deactivate(&self) -> Result<()> {
        use windows::Win32::UI::TextServices::ITfSource;

        // 先注销 sink（不注销 = 系统继续往已释放对象投递事件 → 崩溃）
        let cookies: Vec<_> = self
            .sink_cookies
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();

        let tsm_raw = self.thread_mgr.lock().ok().and_then(|g| *g);
        if let Some(raw) = tsm_raw {
            let ptim: ITfThreadMgr = unsafe { ITfThreadMgr::from_raw(raw as *mut _) };
            if let Ok(source) = ptim.cast::<ITfSource>() {
                for (_iid, cookie) in &cookies {
                    let _ = unsafe { source.UnadviseSink(*cookie) };
                }
            }
            // from_raw 拿走了一份引用，这里要还回去
            std::mem::forget(ptim);
        }

        // 移除语言栏按钮（宿主列表里不该留悬空条目）
        if let Ok(mut g) = self.langbar_item.lock() {
            if let Some(item) = g.take() {
                if let Ok(source) = {
                    self.thread_mgr
                        .lock()
                        .ok()
                        .and_then(|m| *m)
                        .map(|raw| unsafe { ITfThreadMgr::from_raw(raw as *mut _) })
                        .ok_or(())
                } {
                    if let Ok(mgr) =
                        source.cast::<windows::Win32::UI::TextServices::ITfLangBarItemMgr>()
                    {
                        let _ = unsafe { mgr.RemoveItem(&item) };
                    }
                }
            }
        }

        self.focus.on_kill_thread_focus();
        self.activated.store(false, Ordering::SeqCst);
        if let Ok(mut g) = self.thread_mgr.lock() {
            *g = None;
        }
        Ok(())
    }
}

impl ITfTextInputProcessorEx_Impl for TextService_Impl {
    /// 扩展激活：系统在**已激活**的情况下再调一次时走这里。
    ///
    /// 默认实现直接转 `Activate`——与原 C++ 版一致（它也没做额外的事）。
    fn ActivateEx(&self, ptim: Option<&ITfThreadMgr>, tid: u32, _dwflags: u32) -> Result<()> {
        self.Activate(ptim, tid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::com::dll::dll_ref_count;

    /// 每个实例的令牌都不同（服务端靠它区分窗口）。
    #[test]
    fn client_tokens_are_unique() {
        let a = TextService::create();
        let b = TextService::create();
        assert_ne!(a.client_token(), b.client_token());
    }

    /// 令牌高 32 位是本进程 id——服务端据此判断「同进程多实例」。
    #[test]
    fn client_token_embeds_pid() {
        let s = TextService::create();
        let pid = (s.client_token() >> 32) as u32;
        assert_eq!(pid, std::process::id(), "高 32 位必须是进程 id");
    }

    #[test]
    fn starts_inactive() {
        let s = TextService::create();
        assert!(!s.is_activated());
    }

    /// `Deactivate` 幂等：系统可能重复调用。
    #[test]
    fn deactivate_is_idempotent() {
        let s = TextService::create();
        // 未激活时直接 Deactivate 不该出错
        assert!(s.Deactivate().is_ok());
        assert!(s.Deactivate().is_ok());
        assert!(!s.is_activated());
    }

    /// 引用计数与实例数成对。
    #[test]
    fn ref_count_tracks_instances() {
        let before = dll_ref_count();
        {
            let _a = TextService::create();
            let _b = TextService::create();
            assert_eq!(dll_ref_count(), before + 2);
        }
        assert_eq!(dll_ref_count(), before);
    }

    /// CLSID / Profile 是被安装程序写进注册表的值，改动等于换一个输入法。
    /// 这条测试钉住它们（与 `BinaryProtocol.h` 的注释一致）。
    #[test]
    fn clsid_and_profile_are_pinned() {
        assert_eq!(
            CLSID_TEXT_SERVICE,
            windows::core::GUID::from_u128(0x99C2EE30_5C57_45A2_9C63_FB54B34FD90A)
        );
        assert_eq!(
            GUID_PROFILE,
            windows::core::GUID::from_u128(0x99C2EE31_5C57_45A2_9C63_FB54B34FD90A)
        );
        assert_ne!(CLSID_TEXT_SERVICE, GUID_PROFILE, "两者不能相同");
    }
}

impl ITfDisplayAttributeProvider_Impl for TextService_Impl {
    /// 枚举所有显示属性。
    ///
    /// 未实现（`E_NOTIMPL`），与 `DisplayAttributeProvider` 的实现一致：
    /// 宿主主路径是 `GetDisplayAttributeInfo(guid)` 按 GUID 取。
    fn EnumDisplayAttributeInfo(
        &self,
    ) -> Result<windows::Win32::UI::TextServices::IEnumTfDisplayAttributeInfo> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    /// 按 GUID 取显示属性定义。这是宿主的主路径。
    fn GetDisplayAttributeInfo(
        &self,
        guid: *const windows::core::GUID,
    ) -> Result<windows::Win32::UI::TextServices::ITfDisplayAttributeInfo> {
        use crate::com::display_attribute::DisplayAttributeInfo;
        if guid.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        let requested = unsafe { *guid };
        if requested == GUID_DISPLAY_ATTRIBUTE_INPUT {
            Ok(DisplayAttributeInfo::input().into())
        } else if requested == GUID_DISPLAY_ATTRIBUTE_CONVERTED {
            Ok(DisplayAttributeInfo::converted().into())
        } else {
            Err(windows::Win32::Foundation::E_INVALIDARG.into())
        }
    }
}
