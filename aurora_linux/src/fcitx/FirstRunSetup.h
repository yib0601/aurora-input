// 首次启用的自动配置（「装完即用」引导）。
//
// 目标：用户装上包、重启 fcitx5 后**不需要打开任何配置界面**就能打中文。原先要手工做三件事，
// 各自对应 Fcitx5 的一个出厂默认值：
//
//   1. 把「极光输入法」加进输入法组。全新用户的 profile 里组内只有 keyboard-us（Fcitx5 按
//      `default/<locale>` 建初始组），极光不在组里就**根本选不到**；
//   2. `Behavior/ShareInputState=No`（出厂）——每个应用各记各的状态，新开的应用从键盘布局
//      起步，用户每换个窗口都得再按一次切换键；
//   3. `Behavior/ActiveByDefault=False`（出厂）——输入上下文不自动激活，第一次要用还得先按
//      Ctrl+Space。
//
// 三件事都走 Fcitx5 的**进程内** API（InputMethodManager / globalConfig），不调外部 gdbus、
// 不由外部写用户文件。这不是设计文档 §8 当初否决的那条路：那里否决的是「外部脚本改
// profile」——运行中的 fcitx5 退出时会把整份 profile 写回、冲掉外部改动；进程内 setGroup
// 是 fcitx5 自己改自己，没有那个竞态。
//
// 三条红线：
//
//   - **只补不改**：组里已经有极光就一个字段都不碰；ShareInputState / ActiveByDefault 也只在
//     仍是出厂默认值时才改。绝不做「强制刷成推荐配置」。
//   - **幂等 + 可被用户否决**：标记文件记「这台机器引导过了」。光看「组里有没有极光」不够——
//     用户可能**故意**删掉它，删完之后每次启动都自动加回去就成了关不掉的强制行为
//     （AGENTS.md「出厂预置的落点必须是用户能清空的载体」禁止的形态）。有标记就一律尊重现状。
//   - **失败不阻断**：任何一步失败都只是「这次没自动配上」，绝不影响打字与 addon 加载；
//     异常一律吞掉并记日志。
//
// 为什么能放在 addon 里：addon 配置是 `OnDemand=False`（见 data/aurorainput-addon.conf.in），
// fcitx5 启动时必然加载它，于是这段代码在用户会话里、fcitx5 主线程上、addon 构造期执行，
// 时机一定早于用户第一次按键。若 OnDemand 还是 True，本模块会因「极光不在组里 → addon 不
// 加载 → 引导不跑 → 极光仍不在组里」而永远不执行。

#pragma once

namespace fcitx {
class Instance;
} // namespace fcitx

namespace windlinux {

/// 执行一次首启引导（幂等、只补不改）。addon 构造时调用。
///
/// `instance` 由调用方（WindEngine）传自己的 `instance_`：Fcitx5 没有全局单例访问器，
/// addon 拿到的就是这一份。
///
/// 环境变量 `AURORA_NO_FIRST_RUN_SETUP=1` 整体关掉（e2e 用：它自己写死 fcitx 配置，
/// 引导去改会让用例前提漂移）。
///
/// 为什么不在包安装脚本里做：postinst 以 root 跑，改不了用户配置；而放到这里才能在
/// **用户会话**内、用 fcitx5 自己的 API 完成。
void ensureFirstRunSetup(fcitx::Instance* instance);

} // namespace windlinux
