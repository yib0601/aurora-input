//! COM 层：TSF 接口实现与 DLL 导出。
//!
//! 全部 `cfg(windows)`：`windows` crate 的 `Win32_UI_TextServices` 在非 Windows 上
//! 不参与编译，故本层在 Linux 上只做「不编译」，不做「编译通过但不链接」——
//! 那会让 CI 给出虚假信心。
//!
//! # 移植进度
//!
//! | 文件 | 对应的 C++ | 状态 |
//! |---|---|---|
//! | `dll.rs` | `ClassFactory.cpp` + `dllmain.cpp` + `wind_tsf.def` | 骨架 |
//! | `text_service.rs` | `TextService.cpp` | 生命周期 |
//! | `events.rs` | `TextService.cpp` 的焦点回调 | 事件接收 + 状态记账 |
//! | `key_event_sink.rs` | `KeyEventSink.cpp`（3683 行） | 按键入口 + 修饰键状态（业务逻辑未移植） |
//! | （待）`thread_mgr_events.rs` | `TextService.cpp` 的焦点回调 | 未开始 |
//! | `text_edit_sink.rs` | `TextService.cpp` 的 OnEndEdit + CaretEditSession | 光标读取（组合 range 未做） |
//! | `display_attribute.rs` | `DisplayAttributeInfo.cpp`（295 行） | 显示属性定义 |
//! | `langbar.rs` | `LangBarItemButton.cpp`（1534 行） | 语言栏按钮展示（菜单/IPC 接线未做） |
//! | （待）`uielement.rs` | `TextService.cpp` 的候选列表 UIElement | 未开始 |
//! | `register.rs` | `Register.cpp`（521 行） | 第 1 步（COM Server） |
//!
//! ⚠ `DllRegisterServer` 只做了四步中的第 1 步（COM Server 注册）。
//! 还缺 `RegisterProfile` / `RegisterCategories`——没有 Profile 项时系统列表里
//! 看不到这个输入法。在那三步搬完前，注册会**明确返回失败**而不是谎报成功。

pub mod composition;
pub mod display_attribute;
pub mod dll;
pub mod events;
pub mod key_event_sink;
pub mod langbar;
pub mod register;
pub mod text_edit_sink;
pub mod text_service;

pub use dll::{DllCanUnloadNow, DllGetClassObject, DllRegisterServer, DllUnregisterServer};
pub use text_service::{CLSID_TEXT_SERVICE, GUID_PROFILE, TextService};
