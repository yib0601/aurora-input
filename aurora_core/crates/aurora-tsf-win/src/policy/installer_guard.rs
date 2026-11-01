//! `InstallerRunning` 闸门的纯判定逻辑。
//!
//! 原实现：`aurora_tsf/include/InstallerGuard.h`（C++，202 行）。逐条对齐。
//!
//! # 这个闸门是干什么的
//!
//! 安装/卸载期间安装器会杀掉主程序，而本 DLL 被各宿主进程加载着、随时可能把它重新
//! 拉起来，于是安装器先在 `HKLM\Software\<app>\InstallerRunning` 立一个标记，
//! 本 DLL 见到它就不去启动服务。装完由安装计划的最后一步清掉。
//!
//! # 为什么需要「标记的主人还活着吗」
//!
//! 那个标记**没有兜底**：安装器进程若异常死亡（panic / 被杀 / 装到一半失败退出），
//! 清除步骤永远跑不到，标记就永久留在注册表里。后果不是「装坏了」而是
//! **输入法整个不工作且没有任何提示** —— 服务永远起不来，设置页读不到配置，
//! 看起来就是「配置和自造词全没了」，连「重启服务」都没用（本 DLL 压根不去启动它），
//! 而重装一次旧版本反倒会好（那次安装跑完整了，把标记清掉了）。
//!
//! 这三个现象的组合正是 issue #120 的原始描述，2026-09-14 在 vm-win11 上实测复现：
//! 0.121.0 静默覆盖安装连续两次中途失败，每次都留下 `InstallerRunning=1`。
//!
//! # 判据
//!
//! 安装器额外写一个 `InstallerRunningOwner` = `"<pid>|<进程创建时间>"`，
//! 本 DLL 据此校验「立这个标记的进程」是否仍在运行。进程没了 → 标记是遗物，忽略它。
//!
//! ⚠️ 为什么另起一个值名，而不是把身份并进 `InstallerRunning` 本身：
//! 旧版 DLL 读该值用的是 `WCHAR value[8]` 的定长缓冲，值一旦超过 7 个字符，
//! `RegQueryValueExW` 返回 `ERROR_MORE_DATA` 而非 `ERROR_SUCCESS` —— 旧 DLL 会把
//! 「标记存在」读成「标记不存在」，于是安装期间照样去拉起服务，正是这个闸门要防的事。
//! 故 `InstallerRunning` 的值**必须**原样保持 `"1"`，身份信息另放一个值名。
//!
//! 值名是跨语言、跨仓约定，见 `Globals.h` 中 `AURORA_APP_REGKEY` 的注释。

/// 标记本身多久算陈旧（10 分钟）。
pub const STALE_AFTER_MS: u64 = 10 * 60 * 1000;
/// owner 进程创建时间多久算硬陈旧（24 小时）。
pub const OWNER_HARD_STALE_MS: u64 = 24 * 60 * 60 * 1000;

/// 立标记的进程身份。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    /// 进程创建时间（FILETIME，100ns 为单位）。
    pub create_time: u64,
}

/// 从字符流里解析一个十进制 u64，直到遇到 `stop`。
///
/// 溢出时返回 `false`（不静默截断）——截断出来的 pid 会指向无关进程，
/// 那种错误方向是「误判 owner 活着」，即闸门失效。
fn parse_u64(chars: &mut std::str::Chars<'_>, stop: char, out: &mut u64) -> bool {
    let mut v: u64 = 0;
    let mut any = false;
    // 只看不取：不是数字就退出，把该字符留给下面的终止符检查
    while let Some(d) = chars.clone().next().and_then(|c| c.to_digit(10)) {
        chars.next();
        any = true;
        let d = d as u64;
        // 溢出检查：v * 10 + d > u64::MAX
        if v > (u64::MAX - d) / 10 {
            return false;
        }
        v = v * 10 + d;
    }
    if !any {
        return false;
    }
    // 终止符：`stop` 是字面量字符；对「到串尾」用 '\0' 表示，
    // 此时输入已耗尽也算成功（C++ 版用 NUL 结尾的宽字符串，语义等价）。
    let term = chars.next();
    let ok = match (term, stop) {
        (Some(c), _) => c == stop,
        (None, '\0') => true,
        (None, _) => false,
    };
    if !ok {
        return false;
    }
    *out = v;
    true
}

/// 解析 `"<pid>|<createTime>"`。
///
/// pid 必须是 1..=0xFFFF_FFFF（0 是非法 pid，Windows 里 0 表示系统空闲进程，
/// 但绝不会是安装器）。
pub fn parse_owner(value: &str) -> Option<Owner> {
    let mut chars = value.chars();
    let mut pid: u64 = 0;
    if !parse_u64(&mut chars, '|', &mut pid) {
        return None;
    }
    if pid == 0 || pid > 0xFFFF_FFFF {
        return None;
    }
    let mut create: u64 = 0;
    if !parse_u64(&mut chars, '\0', &mut create) {
        return None;
    }
    Some(Owner {
        pid: pid as u32,
        create_time: create,
    })
}

/// `now` 比 `reference` 老超过 `limit_ms` 吗（都用 FILETIME，100ns 为单位）。
///
/// `now <= reference` 时返回 `false`（时钟回退或同一时刻都不算陈旧）。
pub fn is_older_than(reference_ft: u64, now_ft: u64, limit_ms: u64) -> bool {
    if now_ft <= reference_ft {
        return false;
    }
    let elapsed_ms = (now_ft - reference_ft) / 10_000;
    elapsed_ms > limit_ms
}

/// 标记按年龄是否已陈旧。
pub fn is_stale_by_age(key_write_ft: u64, now_ft: u64) -> bool {
    is_older_than(key_write_ft, now_ft, STALE_AFTER_MS)
}

/// owner 进程创建时间是否已硬陈旧。
pub fn is_owner_hard_stale(owner_create_ft: u64, now_ft: u64) -> bool {
    is_older_than(owner_create_ft, now_ft, OWNER_HARD_STALE_MS)
}

/// 该不该**阻止**启动服务。
///
/// 规则：
/// - 没有标记 ⇒ 不阻止
/// - 有标记且能确认 owner 还活着 ⇒ 阻止（正在安装）
/// - 有标记但 owner 已死 / 信息缺失 / 硬陈旧 ⇒ 视为遗物，不阻止
/// - 有标记、无 owner 信息、且标记本身已陈旧 ⇒ 不阻止（老版本安装器留下的）
#[inline]
pub fn should_block_start(
    has_flag: bool,
    has_owner: bool,
    owner_alive: bool,
    stale_by_age: bool,
    owner_hard_stale: bool,
) -> bool {
    if !has_flag {
        return false;
    }
    if has_owner {
        // owner 硬陈旧 = 那个进程创建时间久远到不可能是本次安装（可能是 pid 复用）
        if owner_hard_stale {
            return false;
        }
        return owner_alive;
    }
    // 没有 owner 信息：只有标记还很新时才信它（老安装器没写 owner 值）
    !stale_by_age
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FILETIME 里 1 秒 = 10_000_000。
    const SEC: u64 = 10_000_000;
    const MIN: u64 = 60 * SEC;

    #[test]
    fn parse_valid_owner() {
        let o = parse_owner("1234|133000000000000000").unwrap();
        assert_eq!(o.pid, 1234);
        assert_eq!(o.create_time, 133000000000000000);
    }

    #[test]
    fn parse_rejects_malformed() {
        for bad in [
            "",                              // 空
            "1234",                          // 缺 |
            "1234|",                         // 缺 createTime
            "|133000000000000000",           // 空 pid
            "0|133000000000000000",          // pid=0 非法
            "abc|123",                       // 非数字
            "4294967296|123",                // pid 超 u32
            "1234|133000000000000000|extra", // 多余字段
        ] {
            assert!(parse_owner(bad).is_none(), "应拒绝: {bad:?}");
        }
    }

    /// 溢出必须拒绝，不能静默截断——截断出的 pid 指向无关进程，
    /// 会误判 owner 活着，让闸门永远生效。
    #[test]
    fn parse_rejects_overflow() {
        assert!(
            parse_owner("99999999999999999999|123").is_none(),
            "pid 溢出 u64"
        );
    }

    #[test]
    fn pid_upper_bound_is_inclusive() {
        assert!(parse_owner("4294967295|123").is_some(), "u32::MAX 合法");
    }

    #[test]
    fn is_older_than_boundaries() {
        let now = 100 * MIN;
        assert!(is_older_than(0, now, 10 * 60 * 1000), "100 分钟 > 10 分钟");
        assert!(
            !is_older_than(now - 5 * MIN, now, 10 * 60 * 1000),
            "5 分钟 < 10 分钟"
        );
        // 恰好等于阈值：不算（严格大于）
        assert!(!is_older_than(now - 10 * MIN, now, 10 * 60 * 1000));
    }

    /// 时钟回退：now <= ref 时不算陈旧。
    #[test]
    fn clock_backwards_is_not_stale() {
        assert!(!is_older_than(100 * MIN, 50 * MIN, 10 * 60 * 1000));
        assert!(!is_older_than(100 * MIN, 100 * MIN, 10 * 60 * 1000));
    }

    #[test]
    fn no_flag_never_blocks() {
        assert!(!should_block_start(false, true, true, false, false));
        assert!(!should_block_start(false, false, false, true, true));
    }

    /// 核心场景：安装器活着 ⇒ 阻止（正在安装，别去抢服务）。
    #[test]
    fn live_owner_blocks() {
        assert!(should_block_start(true, true, true, false, false));
    }

    /// 核心场景（issue #120）：安装器已死 ⇒ 标记是遗物，不阻止。
    #[test]
    fn dead_owner_does_not_block() {
        assert!(
            !should_block_start(true, true, false, false, false),
            "安装器已死，标记是遗物——必须放行，否则输入法永久不工作"
        );
    }

    /// pid 复用防护：owner 创建时间久远到不可能是本次安装。
    #[test]
    fn hard_stale_owner_does_not_block() {
        assert!(
            !should_block_start(true, true, true, false, true),
            "owner 硬陈旧（>24h）说明是 pid 复用的别的进程"
        );
    }

    /// 老版本安装器不写 owner 值：只有标记还很新时才信它。
    #[test]
    fn no_owner_falls_back_to_age() {
        assert!(
            should_block_start(true, false, false, false, false),
            "标记新鲜，信它"
        );
        assert!(
            !should_block_start(true, false, false, true, false),
            "标记已陈旧 10 分钟以上，视为老安装器遗留"
        );
    }

    #[test]
    fn stale_and_hard_stale_helpers() {
        // now 取足够大，才能容纳「25 小时前」这个基准点
        let hour: u64 = 60 * MIN;
        let now: u64 = 100 * hour; // 100 小时
        assert!(
            is_stale_by_age(now - 11 * MIN, now),
            "11 分钟 > 10 分钟阈值"
        );
        assert!(!is_stale_by_age(now - 9 * MIN, now), "9 分钟 < 10 分钟阈值");
        assert!(
            is_owner_hard_stale(now - 25 * hour, now),
            "25 小时 > 24 小时阈值"
        );
        assert!(
            !is_owner_hard_stale(now - 23 * hour, now),
            "23 小时 < 24 小时阈值"
        );
    }

    /// 常量是被安装器与本 DLL 共同依赖的时间口径，改动等于改行为。
    #[test]
    fn constants_are_pinned() {
        assert_eq!(STALE_AFTER_MS, 600_000, "10 分钟");
        assert_eq!(OWNER_HARD_STALE_MS, 86_400_000, "24 小时");
    }
}
