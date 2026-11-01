//! 文档上下文是否可编辑。
//!
//! 原实现：`aurora_tsf/include/EditableContextPolicy.h`。
//!
//! 浏览器这类宿主会在没有活动文本框时给一个 `TF_SD_READONLY` 的上下文（Chrome 尤其
//! 典型）。判据决定要不要在它上面开组合——开错会在地址栏、页面上留下幽灵组合。

/// `ITfContext::GetStatus` 返回「空上下文」的 HRESULT。
pub const EMPTY_CONTEXT_HR: u32 = 0x8004_0509;
/// HRESULT 的失败位。
pub const FAILURE_BIT: u32 = 0x8000_0000;
/// `TF_SD_READONLY` 在 dynamic flags 里的位。
pub const READ_ONLY_DYN_FLAG: u32 = 0x1;

/// 上下文是否可编辑。
///
/// - `get_status_hr`：`GetStatus` 的返回值（HRESULT 语义按 u32 处理）。
/// - `dynamic_flags`：`GetStatus` 填出的 flags。
///
/// 失败时**只有「空上下文」这一种失败算不可编辑**，其余失败一律**算可编辑**。
///
/// 反直觉但刻意如此（用 g++ 编译原头文件实测确认）：Chrome 的 doc manager 在没聚焦到
/// 文本框时恒返回 `EMPTY_CONTEXT_HR`，判成不可编辑就永远打不了字；反过来，`E_FAIL` /
/// `E_INVALIDARG` / `TS_E_NOOBJECT` 这些失败是**瞬时**的（对象还没就绪、上下文正在换），
/// 判成不可编辑会让「聚焦后第一个键」静默失效。宁可试一次再失败，也不要一开始就不试。
#[inline]
pub fn context_is_editable(get_status_hr: u32, dynamic_flags: u32) -> bool {
    if (get_status_hr & FAILURE_BIT) != 0 {
        return get_status_hr != EMPTY_CONTEXT_HR;
    }
    (dynamic_flags & READ_ONLY_DYN_FLAG) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const S_OK: u32 = 0;

    #[test]
    fn ok_and_not_readonly_is_editable() {
        assert!(context_is_editable(S_OK, 0));
    }

    #[test]
    fn ok_but_readonly_is_not_editable() {
        assert!(!context_is_editable(S_OK, READ_ONLY_DYN_FLAG));
    }

    /// Chrome 的典型形态：没聚焦文本框时返回「空上下文」，判成不可编辑。
    ///
    /// 这是**唯一**算作不可编辑的失败（实测自 C++ 原实现）。
    #[test]
    fn empty_context_is_not_editable() {
        assert!(!context_is_editable(EMPTY_CONTEXT_HR, 0));
    }

    /// 其余失败一律算可编辑——它们是瞬时的，判死会让聚焦后第一个键静默失效。
    #[test]
    fn other_failures_are_editable() {
        assert!(context_is_editable(0x8000_4005, 0), "E_FAIL");
        assert!(context_is_editable(0x8007_0057, 0), "E_INVALIDARG");
        assert!(context_is_editable(0x8004_0506, 0), "TS_E_NOOBJECT");
    }
}
