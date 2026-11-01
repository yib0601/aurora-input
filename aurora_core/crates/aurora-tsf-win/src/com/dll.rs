//! DLL 导出函数与 COM 类工厂。
//!
//! 原实现：`aurora_tsf/src/ClassFactory.cpp`（90 行）+ `src/dllmain.cpp`（99 行）
//! + `wind_tsf.def`（4 个导出）。
//!
//! # TSF 怎么找到我们
//!
//! 注册表里写的 CLSID 指向本 DLL，系统 `LoadLibrary` 后用 **`DllGetClassObject`**
//! 要类工厂，再 `CreateInstance` 拿 `ITfTextInputProcessor`。四个导出缺一不可：
//!
//! | 导出 | 作用 |
//! |---|---|
//! | `DllGetClassObject` | 给系统一个 `IClassFactory`，由它创建 TextService |
//! | `DllCanUnloadNow` | 系统问「能不能卸」，全仓引用计数为 0 才能卸 |
//! | `DllRegisterServer` | `regsvr32` 时把 CLSID / Profile 写进注册表 |
//! | `DllUnregisterServer` | 卸掉那些注册项 |
//!
//! # 引用计数的形状
//!
//! 两个计数是**分开**的：
//!
//! - `DLL_REF_COUNT`：本 DLL 里所有 COM 对象的活引用总数。`DllCanUnloadNow` 看它。
//! - 每个对象自己的 `ref_count`：`IUnknown::AddRef` / `Release` 管它。
//!
//! 对象创建时 `DllAddRef`，析构时 `DllRelease`——这样即使一个对象被泄漏，
//! 系统也不会在我们还有活对象时把 DLL 卸掉（那会让进程里所有函数指针悬空）。
//!
//! ⚠ 与原实现的唯一差别：原版用 `InterlockedIncrement` 手工维护，这里用
//! `AtomicU32`。语义相同，但少了「忘记用 Interlocked」这一类错误。

#![cfg(windows)]

use std::sync::atomic::{AtomicU32, Ordering};

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_UNEXPECTED, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::core::{GUID, HRESULT, IUnknown, Interface, Result, implement};

use crate::com::CLSID_TEXT_SERVICE;
use crate::com::text_service::TextService;

/// 本 DLL 的活 COM 对象总数。`DllCanUnloadNow` 看它。
static DLL_REF_COUNT: AtomicU32 = AtomicU32::new(0);

/// 给 DLL 引用计数 +1（每个 COM 对象创建时调）。
pub fn dll_add_ref() -> u32 {
    DLL_REF_COUNT.fetch_add(1, Ordering::SeqCst) + 1
}

/// 给 DLL 引用计数 -1（每个 COM 对象析构时调）。
pub fn dll_release() -> u32 {
    DLL_REF_COUNT.fetch_sub(1, Ordering::SeqCst) - 1
}

/// 测试专用导出：连一条管道并跑一次「发一帧 / 收一帧」。
///
/// 这不是 TSF 的一部分（TSF 不调它），而是给外部冒烟测试一个入口，
/// 用来在**真实 Windows 加载器**下验证传输层。生产路径不用它。
///
/// 返回 0 表示成功，其余为错误码。
#[unsafe(no_mangle)]
pub extern "system" fn AuroraTsfTestRoundtrip(pipe_name_utf16: *const u16) -> i32 {
    use crate::ipc::transport::PipeConnection;

    if pipe_name_utf16.is_null() {
        return -1;
    }
    // SAFETY: 调用方保证传入 NUL 结尾的 UTF-16 字符串。
    let len = unsafe {
        let mut n = 0usize;
        while *pipe_name_utf16.add(n) != 0 {
            n += 1;
        }
        n
    };
    let slice = unsafe { std::slice::from_raw_parts(pipe_name_utf16, len) };
    let name = String::from_utf16_lossy(slice);

    let conn = match PipeConnection::connect_with_retry(&name) {
        Ok(c) => c,
        Err(_) => return -2,
    };

    // 发一帧 ACK（内容不重要，服务端只验头部）
    let frame = aurora_ipc::codec::encode_ack();
    if conn.write_frame(&frame).is_err() {
        return -3;
    }
    if conn.read_frame().is_err() {
        return -4;
    }
    0
}

/// 测试专用导出：计算主命令管道名，写进调用方缓冲。
///
/// 返回写入的 UTF-16 码元数（不含 NUL）；缓冲不够返回 -1。
#[unsafe(no_mangle)]
pub extern "system" fn AuroraTsfTestPipeName(
    variant_suffix_utf16: *const u16,
    sid_suffix_utf16: *const u16,
    out: *mut u16,
    cap: u32,
) -> i32 {
    use crate::ipc::transport::command_pipe_name;

    fn to_string(p: *const u16) -> String {
        if p.is_null() {
            return String::new();
        }
        let mut n = 0usize;
        unsafe {
            while *p.add(n) != 0 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    }

    let name = command_pipe_name(
        &to_string(variant_suffix_utf16),
        &to_string(sid_suffix_utf16),
    );
    let units: Vec<u16> = name.encode_utf16().collect();
    if out.is_null() || (units.len() as u32) >= cap {
        return -1;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), out, units.len());
        *out.add(units.len()) = 0;
    }
    units.len() as i32
}

/// 测试专用导出：走一次完整的「按键 → 服务端 → 响应 → 吃/不吃」流程。
///
/// 返回：0 = 被吃（服务端消费了），1 = 未被吃（PassThrough），负数 = 错误。
#[unsafe(no_mangle)]
pub extern "system" fn AuroraTsfTestKeyRoundtrip(pipe_name_utf16: *const u16, vk: u32) -> i32 {
    use crate::ipc::client::{IpcClient, IpcConfig, SyncOutcome};
    use crate::policy::response_type::ResponseType;

    if pipe_name_utf16.is_null() {
        return -1;
    }
    let len = unsafe {
        let mut n = 0usize;
        while *pipe_name_utf16.add(n) != 0 {
            n += 1;
        }
        n
    };
    let slice = unsafe { std::slice::from_raw_parts(pipe_name_utf16, len) };
    let full_name = String::from_utf16_lossy(slice);

    // 从完整管道名反推配置（测试用，生产路径不会这么干）
    let config = if let Some(rest) = full_name.strip_prefix(r"\\.\pipe\") {
        // aurora_input{变体}{SID}
        if let Some(without_base) = rest.strip_prefix("aurora_input") {
            let (variant, sid) = if let Some(pos) = without_base.find("_S-") {
                (&without_base[..pos], &without_base[pos..])
            } else {
                (without_base, "")
            };
            IpcConfig::new(variant, sid)
        } else {
            IpcConfig::new("", "")
        }
    } else {
        IpcConfig::new("", "")
    };

    let client = std::sync::Arc::new(IpcClient::new(config));

    let payload = aurora_ipc::protocol::KeyPayload {
        key_code: vk,
        scan_code: 0,
        modifiers: 0,
        event_type: 0,
        toggles: 0,
        event_seq: 7,
        prev_char: 0x31,
    };

    match client.send_key_event(&payload) {
        SyncOutcome::Response(frame) => {
            let cmd = aurora_ipc::codec::decode_header(&frame)
                .map(|h| h.command)
                .unwrap_or(0);
            match ResponseType::from_command(cmd) {
                ResponseType::Consumed
                | ResponseType::CommitText
                | ResponseType::InsertTextWithCursor
                | ResponseType::CommitThenDefer
                | ResponseType::ReplaceBackward
                | ResponseType::UpdateComposition
                | ResponseType::ClearComposition => 0,
                _ => 1,
            }
        }
        SyncOutcome::ServiceUnavailable => -2,
        SyncOutcome::Transport(_) => -3,
    }
}

/// 测试专用导出：验证语言栏按钮能否构造并 QueryInterface。
///
/// 返回 0 = 成功，负数 = 错误。
#[unsafe(no_mangle)]
pub extern "system" fn AuroraTsfTestLangBar() -> i32 {
    use crate::com::langbar::{LangBarItemButton, LangBarState};
    use windows::Win32::UI::TextServices::{ITfLangBarItem, ITfLangBarItemButton};
    use windows::core::Interface;

    let state = std::sync::Arc::new(LangBarState::new());
    state.set_enabled(true);
    let item = LangBarItemButton::new(state);

    let iface: ITfLangBarItem = item.into();
    // 走一遍 GetInfo / GetStatus / GetTooltipString
    let mut info = windows::Win32::UI::TextServices::TF_LANGBARITEMINFO::default();
    if unsafe { iface.GetInfo(&mut info) }.is_err() {
        return -1;
    }
    if unsafe { iface.GetStatus() }.is_err() {
        return -2;
    }
    if unsafe { iface.GetTooltipString() }.is_err() {
        return -3;
    }
    // 再查 ITfLangBarItemButton
    if iface.cast::<ITfLangBarItemButton>().is_err() {
        return -4;
    }
    0
}

/// 测试专用导出：验证组合状态机的运行时对象能构造并转移状态。
///
/// 返回：0 = 全部正常，负数 = 某步失败。
#[unsafe(no_mangle)]
pub extern "system" fn AuroraTsfTestComposition() -> i32 {
    use crate::com::composition::{CompositionRuntime, EditTask};
    use crate::policy::composition::{Action, CompositionState};

    let rt = std::sync::Arc::new(CompositionRuntime::new());

    // 初始空闲
    if rt.state() != CompositionState::Idle {
        return -1;
    }
    // 空文本不建组合
    if rt.plan_text(0) != Action::None {
        return -2;
    }
    // 首个文本建组合
    if !matches!(rt.plan_text(3), Action::Start { .. }) {
        return -3;
    }
    // 后续更新
    if !matches!(rt.plan_text(5), Action::UpdateText { .. }) {
        return -4;
    }
    // 结束
    if !matches!(
        rt.plan_end(true),
        Action::End {
            clear_text: true,
            ..
        }
    ) {
        return -5;
    }
    // 清理
    rt.on_cleanup_done();
    if rt.state() != CompositionState::Idle {
        return -6;
    }
    // 宿主强杀路径
    rt.plan_text(2);
    let reason = rt.on_host_terminated();
    if reason != crate::policy::composition::EndReason::TerminatedByHost {
        return -7;
    }
    if rt.plan_end(true) != Action::None {
        return -8; // 强杀后不该再产生动作
    }
    let _ = EditTask::End { clear_text: true };
    0
}

/// 当前活引用数（诊断用）。
pub fn dll_ref_count() -> u32 {
    DLL_REF_COUNT.load(Ordering::SeqCst)
}

/// 本项目的 `IClassFactory` 实现。
#[implement(IClassFactory)]
pub struct ClassFactory;

impl ClassFactory {
    pub fn new() -> Self {
        dll_add_ref();
        Self
    }
}

impl Default for ClassFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        dll_release();
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // 不支持聚合：TSF 不用，允许聚合会让生命周期推理复杂化而无收益。
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        if ppvobject.is_null() || riid.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        unsafe {
            *ppvobject = core::ptr::null_mut();
            let service: IUnknown = TextService::create().into();
            service.query(riid, ppvobject).ok()
        }
    }

    fn LockServer(&self, _flock: windows::Win32::Foundation::BOOL) -> Result<()> {
        // TSF 不锁服务；返回成功以保持兼容（原实现同样忽略）。
        Ok(())
    }
}

// ── 四个导出 ────────────────────────────────────────────────────────────

/// 系统来要类工厂。
///
/// # Safety
/// 由 COM 运行时调用，指针契约由 COM 规定。
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    unsafe {
        if rclsid.is_null() || riid.is_null() || ppv.is_null() {
            return windows::Win32::Foundation::E_POINTER;
        }
        *ppv = core::ptr::null_mut();

        let clsid = *rclsid;
        if clsid != CLSID_TEXT_SERVICE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }

        let factory: IClassFactory = ClassFactory::new().into();
        let hr = factory.query(riid, ppv);
        if hr.is_err() {
            return hr;
        }
    }
    S_OK
}

/// 系统问能不能卸本 DLL。
///
/// 还有活对象 ⇒ `S_FALSE`（不卸）。全部释放后才 `S_OK`。
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if dll_ref_count() == 0 {
        S_OK
    } else {
        windows::Win32::Foundation::S_FALSE
    }
}

/// `regsvr32` 注册：COM Server → Profile → 类别，三步全部完成。
///
/// 顺序不可换：先有 `InprocServer32` 指向 DLL，系统才可能加载我们；
/// 再登记 Profile，输入法才出现在列表里；最后登记类别，UWP / 开始菜单才认它。
#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    use crate::com::register;

    match register::register_all() {
        Ok(()) => {
            tracing::info!("注册完成：COM Server + Profile + Categories");
            S_OK
        }
        Err(e) => {
            tracing::error!("注册失败: {e}");
            E_UNEXPECTED
        }
    }
}

/// `regsvr32 /u` 反注册：与注册相反的顺序（类别 → Profile → COM Server）。
///
/// 顺序反了会留下指向不存在对象的孤儿条目——系统输入法列表里出现一个
/// 选了就没反应的输入法。
#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    use crate::com::register;

    match register::unregister_all() {
        Ok(()) => {
            tracing::info!("反注册完成");
            S_OK
        }
        Err(e) => {
            tracing::error!("反注册失败: {e}");
            E_UNEXPECTED
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DLL 引用计数成对增减。
    #[test]
    fn dll_ref_count_is_balanced() {
        let before = dll_ref_count();
        let f = ClassFactory::new();
        assert_eq!(dll_ref_count(), before + 1);
        drop(f);
        assert_eq!(dll_ref_count(), before);
    }

    /// 还有活对象时不能卸。
    ///
    /// 这条规则的实际后果很严重：若在还有活对象时允许卸 DLL，系统会
    /// `FreeLibrary` 掉我们，而进程里所有指向我们代码的函数指针立刻悬空。
    #[test]
    fn cannot_unload_while_objects_alive() {
        let before = dll_ref_count();
        if before != 0 {
            // 其它测试并行跑时可能已有计数，只验证相对变化
            let f = ClassFactory::new();
            assert_ne!(dll_ref_count(), 0);
            drop(f);
            return;
        }
        let f = ClassFactory::new();
        assert_ne!(DllCanUnloadNow(), S_OK, "有活对象时不能返回 S_OK");
        drop(f);
        assert_eq!(DllCanUnloadNow(), S_OK, "全部释放后可以卸");
    }

    /// E_NOINTERFACE 路径不该泄漏引用计数。
    ///
    /// 原实现里 `*ppvObj` 先置空再判定，拿不到接口时直接返回——这里的
    /// `query` 同理。这条测的是「失���的查询不改变全局计数」。
    #[test]
    fn failed_query_does_not_leak() {
        let before = dll_ref_count();
        {
            let factory: IClassFactory = ClassFactory::new().into();
            let bogus = GUID::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888);
            let mut out: *mut core::ffi::c_void = core::ptr::null_mut();
            let hr = unsafe { factory.query(&bogus, &mut out) };
            assert_eq!(hr, E_NOINTERFACE);
            assert!(out.is_null());
        }
        assert_eq!(dll_ref_count(), before);
    }
}
