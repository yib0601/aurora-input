//! `ITfDisplayAttributeProvider` + `ITfDisplayAttributeInfo`：组合文本的显示样式。
//!
//! 原实现：`aurora_tsf/src/DisplayAttributeInfo.cpp`（295 行）+
//! `include/DisplayAttributeInfo.h`（70 行）。
//!
//! # 这是什么
//!
//! 输入法激活时，组合中的文本（还没上屏的编码）需要视觉区分。TSF 的做法是给文本
//! 打「显示属性」标签，宿主据此渲染下划线。
//!
//! 本输入法提供两个属性：
//!
//! | GUID | 用途 | 样式 |
//! |---|---|---|
//! | `GUID_DISPLAY_ATTRIBUTE_INPUT` | 输入中（编码区） | 点状下划线 |
//! | `GUID_DISPLAY_ATTRIBUTE_CONVERTED` | 已转换（未用） | 同 |
//!
//! # 只有下划线，没有改颜色
//!
//! `crText` / `crBk` / `crLine` 都是 `TF_CT_NONE`——底色与文字色由宿主主题决定，
//! 我们只加一条下划线。这样在多主题宿主里不会出现「白底白字」这类不可读的组合。
//!
//! # 为什么不用 `ITfDisplayAttributeMgr` 注册
//!
//! 显示属性要在**注册表**里登记（`RegisterCategories` 已做），然后由
//! `ITfCategoryMgr` 关联到本 CLSID。运行期宿主通过 `GetDisplayAttributeInfo(guid)`
//! 找我们要具体定义。这条链已在 `com/register.rs` 里接通。

#![cfg(windows)]

use windows::Win32::UI::TextServices::{
    IEnumTfDisplayAttributeInfo, ITfDisplayAttributeInfo, ITfDisplayAttributeInfo_Impl,
    ITfDisplayAttributeProvider, ITfDisplayAttributeProvider_Impl, TF_ATTR_INPUT, TF_CT_NONE,
    TF_DISPLAYATTRIBUTE, TF_LS_DOT,
};
use windows::core::{BSTR, GUID, Result, implement};

use crate::com::text_service::{GUID_DISPLAY_ATTRIBUTE_CONVERTED, GUID_DISPLAY_ATTRIBUTE_INPUT};

/// 「输入中」属性的描述文本。会出现在宿主的调试工具里。
const DESC_INPUT: &str = "Aurora Input - Input (dotted underline)";
/// 「已转换」属性的描述文本。
const DESC_CONVERTED: &str = "Aurora Input - Converted";

/// 构造一个显示属性。
///
/// 与 C++ 的构造函数逐字段对齐：
///
/// ```text
/// crText.type = TF_CT_NONE    文字颜色：不指定（用宿主主题）
/// crBk.type   = TF_CT_NONE    背景色：不指定
/// lsStyle     = TF_LS_DOT     点状下划线
/// fBoldLine   = FALSE         不加粗
/// crLine.type = TF_CT_NONE    下划线颜色：不指定
/// bAttr       = TF_ATTR_INPUT 属性类别
/// ```
fn make_display_attribute() -> TF_DISPLAYATTRIBUTE {
    let mut attr = TF_DISPLAYATTRIBUTE::default();
    attr.crText.r#type = TF_CT_NONE;
    attr.crBk.r#type = TF_CT_NONE;
    attr.lsStyle = TF_LS_DOT;
    attr.fBoldLine = windows::Win32::Foundation::BOOL(0);
    attr.crLine.r#type = TF_CT_NONE;
    attr.bAttr = TF_ATTR_INPUT;
    attr
}

/// 一个显示属性定义。
///
/// 两个 GUID 各一个实例，行为完全相同（只有 GUID 与描述不同）。
#[implement(ITfDisplayAttributeInfo)]
pub struct DisplayAttributeInfo {
    guid: GUID,
    description: &'static str,
    /// 当前属性值。
    ///
    /// `SetAttributeInfo` 允许宿主改它（规范允许，但我们用不到）——存下来而不是
    /// 拒之门外，这样宿主的行为可预期。`Reset` 恢复默认。
    current: std::sync::Mutex<TF_DISPLAYATTRIBUTE>,
}

impl DisplayAttributeInfo {
    pub fn new(guid: GUID, description: &'static str) -> Self {
        Self {
            guid,
            description,
            current: std::sync::Mutex::new(make_display_attribute()),
        }
    }

    /// 「输入中」属性。
    pub fn input() -> Self {
        Self::new(GUID_DISPLAY_ATTRIBUTE_INPUT, DESC_INPUT)
    }

    /// 「已转换」属性。
    pub fn converted() -> Self {
        Self::new(GUID_DISPLAY_ATTRIBUTE_CONVERTED, DESC_CONVERTED)
    }

    pub fn guid_value(&self) -> GUID {
        self.guid
    }
}

impl ITfDisplayAttributeInfo_Impl for DisplayAttributeInfo_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(self.guid)
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(BSTR::from(self.description))
    }

    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        if pda.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        let current = self
            .current
            .lock()
            .map(|g| *g)
            .unwrap_or_else(|_| make_display_attribute());
        // SAFETY: 调用方传了有效指针（上面已检查非空）。
        unsafe { *pda = current };
        Ok(())
    }

    fn SetAttributeInfo(&self, pda: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        if pda.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        // SAFETY: 上面已检查非空。
        let incoming = unsafe { *pda };
        if let Ok(mut g) = self.current.lock() {
            *g = incoming;
        }
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        if let Ok(mut g) = self.current.lock() {
            *g = make_display_attribute();
        }
        Ok(())
    }
}

/// `ITfDisplayAttributeProvider`：宿主来要属性定义。
#[implement(ITfDisplayAttributeProvider)]
pub struct DisplayAttributeProvider;

impl DisplayAttributeProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DisplayAttributeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ITfDisplayAttributeProvider_Impl for DisplayAttributeProvider_Impl {
    /// 枚举所有属性。
    ///
    /// ⚠ 未实现：`IEnumTfDisplayAttributeInfo` 需要手写 COM 枚举器对象
    /// （Next / Reset / Skip / Clone 四个方法）。宿主**主要走 `GetDisplayAttributeInfo`**
    /// （按 GUID 取），枚举只在诊断工具里用。
    ///
    /// 返回 `E_NOTIMPL` 而不是空枚举器：空枚举器会让诊断工具以为「这个输入法没有属性」，
    /// 而 `E_NOTIMPL` 明确表达「这一项没做」。
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Err(windows::Win32::Foundation::E_NOTIMPL.into())
    }

    /// 按 GUID 取属性定义。这是**主路径**。
    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        if guid.is_null() {
            return Err(windows::Win32::Foundation::E_POINTER.into());
        }
        // SAFETY: 上面已检查非空。
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 默认属性必须与 C++ 构造函数的逐字段一致。
    ///
    /// 这些值决定组合文本长什么样：改 `lsStyle` 会让下划线从点线变成实线，
    /// 改 `crText` 会覆盖宿主主题色（可能白底白字）。
    #[test]
    fn default_attribute_matches_cpp() {
        let a = make_display_attribute();
        assert_eq!(a.lsStyle, TF_LS_DOT, "点状下划线");
        assert_eq!(a.fBoldLine.as_bool(), false, "不加粗");
        assert_eq!(a.bAttr, TF_ATTR_INPUT);
        assert_eq!(a.crText.r#type, TF_CT_NONE, "文字色交给宿主主题");
        assert_eq!(a.crBk.r#type, TF_CT_NONE, "背景色交给宿主主题");
        assert_eq!(a.crLine.r#type, TF_CT_NONE, "下划线色交给宿主主题");
    }

    #[test]
    fn attribute_guids_are_distinct() {
        let input = DisplayAttributeInfo::input();
        let converted = DisplayAttributeInfo::converted();
        assert_ne!(input.guid_value(), converted.guid_value());
        assert_eq!(input.guid_value(), GUID_DISPLAY_ATTRIBUTE_INPUT);
        assert_eq!(converted.guid_value(), GUID_DISPLAY_ATTRIBUTE_CONVERTED);
    }

    #[test]
    fn descriptions_are_set() {
        assert!(!DESC_INPUT.is_empty());
        assert!(!DESC_CONVERTED.is_empty());
        assert_ne!(DESC_INPUT, DESC_CONVERTED, "两个描述要能区分");
    }
}
