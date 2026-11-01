//! `regsvr32` 注册：把 CLSID 与输入法信息写进注册表。
//!
//! 原实现：`aurora_tsf/src/Register.cpp`（521 行）。
//!
//! # 注册分四步（原实现）
//!
//! 1. `RegisterCOMServer` —— `HKCR\CLSID\{...}\InprocServer32` 指向本 DLL
//! 2. `RegisterProfile` —— `ITfInputProcessorProfiles::RegisterProfile` 登记输入法
//! 3. `RegisterCategories` —— `ITfCategoryMgr` 登记类别（键盘处理器 / 显示属性）
//! 4. `RegisterServer` —— 依次调上面三个
//!
//! # 本文件的范围
//!
//! 三步全部搬完。仍**未搬**的是原实现的 Dota2 兼容别名逻辑
//! （`_ReadRecordedAlias` / `_ProfileNameToRegister` 的别名保留分支）——
//! 那是「升级时不冲掉用户自定义的输入法名」，属于锦上添花，不影响安装。
//!
//! # 变体
//!
//! dev 变体用**不同的 CLSID**（`com::text_service` 里那份常量按 feature 切换），
//! 使正式版与开发版可以同时安装、互不干扰。

#![cfg(windows)]

use windows::Win32::Foundation::{E_FAIL, E_UNEXPECTED, ERROR_SUCCESS};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::core::{GUID, PCWSTR};

use crate::com::text_service::CLSID_TEXT_SERVICE;

/// 输入法显示名。
///
/// ⚠ 与 `Globals.h` 的 `TEXTSERVICE_NAME` / `TEXTSERVICE_DESC` 逐字一致。
/// 改这里要同步改那边（C++ 版仍在用），以及安装器清单。
#[cfg(feature = "dev-variant")]
pub const TEXTSERVICE_DESC: &str = "极光输入法 Dev (WindInputDev)";
#[cfg(not(feature = "dev-variant"))]
pub const TEXTSERVICE_DESC: &str = "极光输入法 (WindInput)";

/// 取本 DLL 的完整路径。
fn module_path() -> Option<Vec<u16>> {
    let mut buf = vec![0u16; 32768]; // UNICODE_STRING 上限
    let n = unsafe { GetModuleFileNameW(None, &mut buf) };
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    buf.truncate(n as usize);
    buf.push(0); // 尾部 NUL，供 PCWSTR 使用
    Some(buf)
}

/// 把 GUID 转成注册表用的 `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` 形式。
///
/// COM 的 `StringFromGUID2` 出来的就是这个形状，手工拼等价。
fn guid_to_registry_key(guid: &GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

/// 写一个 `REG_SZ` 值（含结尾 NUL）。
///
/// # Safety
/// `key` 必须是有效且已打开的注册表键。
unsafe fn set_sz(key: HKEY, name: Option<&str>, value: &str) -> bool {
    #![allow(unsafe_op_in_unsafe_fn)]
    let wide_value: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let name_wide: Option<Vec<u16>> =
        name.map(|n| n.encode_utf16().chain(std::iter::once(0)).collect());
    let name_ptr = name_wide
        .as_ref()
        .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr()));

    unsafe {
        let bytes =
            std::slice::from_raw_parts(wide_value.as_ptr() as *const u8, wide_value.len() * 2);
        RegSetValueExW(key, name_ptr, 0, REG_SZ, Some(bytes)) == ERROR_SUCCESS
    }
}

/// 创建（或打开）一个 `HKCR` 下的子键。
///
/// # Safety
/// 由调用方保证在注册表操作上下文中。
unsafe fn create_key(path: &str) -> Option<HKEY> {
    #![allow(unsafe_op_in_unsafe_fn)]
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CLASSES_ROOT,
            PCWSTR(wide.as_ptr()),
            0, // reserved，必须为 0
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if result == ERROR_SUCCESS {
        Some(key)
    } else {
        None
    }
}

/// 第 1 步：注册 COM Server（`HKCR\CLSID\{...}\InprocServer32` → 本 DLL）。
///
/// 这就是系统 `LoadLibrary` 我们、调 `DllGetClassObject` 的依据。没有这一项，
/// TSF 根本不知道这个 DLL 存在。
pub fn register_com_server() -> windows::core::Result<()> {
    let Some(module) = module_path() else {
        return Err(E_FAIL.into());
    };
    let module_str = String::from_utf16_lossy(&module[..module.len() - 1]);

    let clsid = guid_to_registry_key(&CLSID_TEXT_SERVICE);

    unsafe {
        // ① HKCR\CLSID\{...} 默认值 = 描述
        let clsid_path = format!("CLSID\\{clsid}");
        let Some(key) = create_key(&clsid_path) else {
            return Err(E_UNEXPECTED.into());
        };
        let ok = set_sz(key, None, TEXTSERVICE_DESC);
        let _ = RegCloseKey(key);
        if !ok {
            return Err(E_UNEXPECTED.into());
        }

        // ② HKCR\CLSID\{...}\InprocServer32 默认值 = DLL 路径；ThreadingModel = Apartment
        let inproc_path = format!("CLSID\\{clsid}\\InprocServer32");
        let Some(key) = create_key(&inproc_path) else {
            return Err(E_UNEXPECTED.into());
        };
        let ok = set_sz(key, None, &module_str);
        // TSF 标准线程模型：Apartment（weasel / SampleIME 同为 Apartment）。
        //
        // ⚠ 历史上这里曾是 Both——Go 时代为「Win11 现代应用兼容」引入的 workaround 带入，
        // 原始症状已失传。改回 Apartment 是与 TSF 标准对齐，**不是**「顺手改的」：
        // Both 会让 COM 在任意套间里调我们的代码，而 TSF 的很多接口是线程亲和的
        // （`ITfThreadMgr` 绑定创建它的线程），跨套间调用会静默失败。
        let ok_threading = set_sz(key, Some("ThreadingModel"), "Apartment");
        let _ = RegCloseKey(key);
        if !ok || !ok_threading {
            return Err(E_UNEXPECTED.into());
        }
    }

    Ok(())
}

/// 第 1 步的反向：删掉 COM Server 注册项。
pub fn unregister_com_server() -> windows::core::Result<()> {
    use windows::Win32::System::Registry::RegDeleteTreeW;

    let clsid = guid_to_registry_key(&CLSID_TEXT_SERVICE);
    let path = format!("CLSID\\{clsid}");
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        // 删整棵子树（含 InprocServer32）。键不存在时同样算成功——
        // 反注册应当幂等，重复执行不该报错。
        let _ = RegDeleteTreeW(HKEY_CLASSES_ROOT, PCWSTR(wide.as_ptr()));
    }
    Ok(())
}

/// 完整注册：COM Server + Profile。
///
/// 顺序不可换：先有 `InprocServer32` 指向 DLL，系统才可能加载我们；
/// 再登记 Profile，输入法才出现在列表里。
///
/// 第 3 步（`ITfCategoryMgr` 类别注册）尚未移植——缺它时输入法能出现在列表里，
/// 但某些宿主可能不把它当「键盘处理器」看待。原实现有三类：
/// `GUID_TFCAT_TIP_KEYBOARD` / `GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER` / 语言相关类别。
pub fn register_all() -> windows::core::Result<()> {
    register_com_server()?;
    register_profile()?;
    register_categories()?;
    Ok(())
}

/// 完整反注册：与注册相反的顺序。
pub fn unregister_all() -> windows::core::Result<()> {
    // 与注册相反：先摘类别，再摘 profile，最后摘 COM Server。
    // 顺序反了会留下指向不存在对象的孤儿条目。
    unregister_categories()?;
    unregister_profile()?;
    unregister_com_server()?;
    Ok(())
}

// ── 第 2 步：登记输入法 Profile ────────────────────────────────────────

/// 本输入法的语言 ID：`0x0804` = 中文（简体）。
///
/// ⚠ 与 `Globals.h` 的 `TEXTSERVICE_LANGID` 逐字一致。
pub const TEXTSERVICE_LANGID: u16 = 0x0804;

/// 图标在 DLL 资源里的序号（`TEXTSERVICE_ICON_INDEX`）。
pub const TEXTSERVICE_ICON_INDEX: u32 = 0;

/// 解析图标文件路径。
///
/// # 为什么图标源与 COM 注册用的路径是两件事
///
/// `InprocServer32` 必须指**系统副本**（位数要匹配，32 位 TSF 宿主加载 32 位 DLL）；
/// 而图标必须指一个**两种位数都解析得到**的路径——若图标也指 32 位副本，64 位进程
/// 里输入法列表会显示空白图标。
///
/// 原实现的 `_ResolveIconFile` 处理这个差异（取同目录下的主 DLL）。这里保持同样的
/// 策略：优先用传入路径同目录下的非 `_dev` 版本，退而用自身。
fn resolve_icon_file(module: &str) -> String {
    // dev 变体指向正式版图标（两者图标一致，而正式版两种位数都在）
    if let Some(pos) = module.rfind("_dev") {
        let mut candidate = String::with_capacity(module.len());
        candidate.push_str(&module[..pos]);
        candidate.push_str(&module[pos + 4..]);
        // 实际是否存在由系统在取图标时判定；这里不做文件探测（避免一次多余 IO）。
        return candidate;
    }
    module.to_string()
}

/// 第 2 步：用 `ITfInputProcessorProfileMgr::RegisterProfile` 登记输入法。
///
/// 这是 Win8+ 的现代注册方式。没有这一步，系统输入法列表里**看不到**这个输入法——
/// 用户会以为「装好了但没用」。
pub fn register_profile() -> windows::core::Result<()> {
    use windows::Win32::Foundation::BOOL;
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Input::KeyboardAndMouse::HKL;
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfileMgr,
    };

    let Some(module) = module_path() else {
        return Err(E_FAIL.into());
    };
    let module_str = String::from_utf16_lossy(&module[..module.len() - 1]);
    let icon = resolve_icon_file(&module_str);

    // 描述串：转 UTF-16 且**去掉结尾 NUL**——`RegisterProfile` 接的是切片，
    // 长度由切片决定；带 NUL 会让系统在名字后面多显示一个空字符。
    let desc: Vec<u16> = TEXTSERVICE_DESC.encode_utf16().collect();
    let icon_wide: Vec<u16> = icon.encode_utf16().collect();

    unsafe {
        let mgr: ITfInputProcessorProfileMgr =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;

        mgr.RegisterProfile(
            &CLSID_TEXT_SERVICE,
            TEXTSERVICE_LANGID,
            &crate::com::text_service::GUID_PROFILE,
            &desc,
            &icon_wide,
            TEXTSERVICE_ICON_INDEX,
            HKL::default(),
            0,       // dwPreferredLayout
            BOOL(1), // bEnabledByDefault：装完即可用
            0,       // dwFlags
        )?;
        let _ = mgr;
    }
    Ok(())
}

/// 第 2 步的反向：注销 Profile。
pub fn unregister_profile() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::TextServices::{
        CLSID_TF_InputProcessorProfiles, ITfInputProcessorProfiles,
    };

    unsafe {
        let profiles: ITfInputProcessorProfiles =
            CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        // 先摘 profile 再摘 CLSID；顺序反了会留下指向不存在 CLSID 的孤儿 profile。
        let _ = profiles.RemoveLanguageProfile(
            &CLSID_TEXT_SERVICE,
            TEXTSERVICE_LANGID,
            &crate::com::text_service::GUID_PROFILE,
        );
        let _ = profiles.Unregister(&CLSID_TEXT_SERVICE);
    }
    Ok(())
}

// ── 第 3 步：登记 TSF 类别 ──────────────────────────────────────────────

/// 需要登记的 TSF 类别。
///
/// 与小狼毫（weasel）保持一致的完整列表——这是 Win11 开始菜单 / UWP 兼容的关键。
/// 缺 `TIPCAP_COMLESS` 时 UWP / AppContainer 应用里完全打不了字，而桌面应用正常，
/// 极难归因。
///
/// ⚠ 两个 `PROPSTYLE_*` 不在列表里：windows crate 没有它们的权威 GUID 值。
/// C++ 版在 MinGW 下同样跳过（见 `mingw_tsf_compat.h`），行为一致。
fn category_list() -> Vec<windows::core::GUID> {
    use windows::Win32::UI::TextServices::*;
    vec![
        GUID_TFCAT_CATEGORY_OF_TIP,             // 基础：标识为 TIP
        GUID_TFCAT_TIP_KEYBOARD,                // 基础：键盘输入法
        GUID_TFCAT_TIPCAP_SECUREMODE,           // 安全模式（锁屏 / 开始菜单搜索）
        GUID_TFCAT_TIPCAP_UIELEMENTENABLED,     // UI 元素支持
        GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT, // 输入模式区间
        GUID_TFCAT_TIPCAP_COMLESS,              // 【关键】COM-less 支持，UWP/AppContainer 必需
        GUID_TFCAT_TIPCAP_WOW16,                // WOW16 兼容
        GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,     // Win8+ 沉浸式应用支持
        GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,       // 系统托盘支持
        GUID_TFCAT_PROP_AUDIODATA,              // 属性：音频数据
        GUID_TFCAT_PROP_INKDATA,                // 属性：墨迹数据
        GUID_TFCAT_PROPSTYLE_STATIC,            // 属性样式：静态
        GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,    // 显示属性提供者
        GUID_TFCAT_DISPLAYATTRIBUTEPROPERTY,    // 显示属性
    ]
}

/// 第 3 步：登记 TSF 类别。
///
/// 单条失败只记警告不中断——原实现如此。理由：类别是**附加**能力，
/// 缺个别类别表现为「某类应用里功能受限」，而中断整个注册会让用户完全用不了。
pub fn register_categories() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::TextServices::{CLSID_TF_CategoryMgr, ITfCategoryMgr};

    let mgr: ITfCategoryMgr =
        unsafe { CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)? };

    let mut failed = 0u32;
    for cat in category_list() {
        let hr = unsafe { mgr.RegisterCategory(&CLSID_TEXT_SERVICE, &cat, &CLSID_TEXT_SERVICE) };
        if hr.is_err() {
            failed += 1;
            tracing::warn!("类别注册失败: {hr:?}");
        }
    }

    // 显示属性要单独关联到 provider（否则候选窗的下划线样式不生效）
    let hr = unsafe {
        mgr.RegisterCategory(
            &CLSID_TEXT_SERVICE,
            &windows::Win32::UI::TextServices::GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
            &crate::com::text_service::GUID_DISPLAY_ATTRIBUTE_INPUT,
        )
    };
    if hr.is_err() {
        tracing::warn!("显示属性关联失败: {hr:?}");
        failed += 1;
    }

    if failed > 0 {
        tracing::warn!("类别注册有 {failed} 项失败（不阻断，功能可能受限）");
    }
    Ok(())
}

/// 第 3 步的反向。**列表必须与注册时完全一致**，否则会留注册表残留。
pub fn unregister_categories() -> windows::core::Result<()> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::TextServices::{CLSID_TF_CategoryMgr, ITfCategoryMgr};

    let mgr: ITfCategoryMgr =
        unsafe { CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)? };

    for cat in category_list() {
        let _ = unsafe { mgr.UnregisterCategory(&CLSID_TEXT_SERVICE, &cat, &CLSID_TEXT_SERVICE) };
    }
    let _ = unsafe {
        mgr.UnregisterCategory(
            &CLSID_TEXT_SERVICE,
            &windows::Win32::UI::TextServices::GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
            &crate::com::text_service::GUID_DISPLAY_ATTRIBUTE_INPUT,
        )
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册表键名的格式必须是 `{8-4-4-4-12}` 大写十六进制。
    ///
    /// 格式错一个字符，注册项就落在别处，系统找不到我们的 DLL，
    /// 而 `regsvr32` 却报「注册成功」——这种错最难查。
    #[test]
    fn guid_registry_format() {
        let key = guid_to_registry_key(&CLSID_TEXT_SERVICE);
        assert_eq!(key, "{99C2EE30-5C57-45A2-9C63-FB54B34FD90A}");
        assert_eq!(key.len(), 38, "含两个花括号共 38 字符");
        assert!(key.starts_with('{') && key.ends_with('}'));
    }

    /// 全零 GUID 也要能正确��式化（边界）。
    #[test]
    fn zero_guid_format() {
        let key = guid_to_registry_key(&GUID::zeroed());
        assert_eq!(key, "{00000000-0000-0000-0000-000000000000}");
    }

    /// 描述文本与 C++ 侧 `Globals.h` 逐字一致。
    ///
    /// 两处写得不一样时，用户在输入法列表里看到的名字会随安装方式变化。
    #[test]
    fn description_matches_cpp() {
        #[cfg(feature = "dev-variant")]
        assert_eq!(TEXTSERVICE_DESC, "极光输入法 Dev (WindInputDev)");
        #[cfg(not(feature = "dev-variant"))]
        assert_eq!(TEXTSERVICE_DESC, "极光输入法 (WindInput)");
    }
}
