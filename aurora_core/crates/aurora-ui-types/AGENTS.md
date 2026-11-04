<!-- Parent: ../../../AGENTS.md -->

# aurora-ui-types

> 表现层协议（纯数据）：`UiCommand`/`UiEvent` 及载荷，协调器 ↔ 桌面/macOS/Android 前端共用。

## 职责

协调器 ↔ 任意前端的**表现层协议（纯数据）**：正向 `UiCommand`（协调器→渲染端）、反向 `UiEvent`（渲染端→协调器）及其载荷类型。消费者：桌面渲染线程（aurora-ui）、macOS `.app`（经 host-render 转发）、Android Kotlin 壳（经 FFI 回调）。进程内契约、非线协议（线协议在 aurora-ipc）。

## 关键文件

| 文件 | 说明 |
|------|------|
| `src/command.rs` | `UiCommand` + `GlobalHotkeyEntry` + `HOVER_PAGE_*`；`#[cfg(windows)] HostRenderArc` 是唯一平台载荷 |
| `src/event.rs` | `UiEvent`（渲染端反向事件） |
| `src/menu.rs` | 菜单族：`MenuItemSpec`/`MenuKind`（含稳定 id 双向映射 `to_menu_id`/`from_menu_id`）/`MenuAnchor`/`CandidateOp`/`ToolbarAction` |
| `src/candidate.rs` / `src/toast.rs` / `src/toolbar.rs` | `CandidateItem` / `ToastKind`·`ToastPosition` / `ToolbarState` |
| `src/caret_overlay.rs` | 候选窗定位调试浮窗的一帧数据 `CaretOverlayView` + 组合矩形判据分支 `RectVerdict`（输入诊断 HUD 已整块删除） |

## 硬约定

### 三条红线（破任何一条即破坏 Android 构建承诺）

1. **仅纯数据与纯映射逻辑**：零 IO、零日志（不引 tracing）、零 serde、零渲染。行为实现放消费方（渲染色表在 aurora-ui、剪贴板走 coordinator 的 HostServices）。
2. **新增依赖必须通过 `cargo check-android`**（alias 见 `aurora_core/.cargo/config.toml`；CI 强制）。当前依赖面仅 aurora-theme（SetTheme 载荷）。
3. **平台专属载荷一律 target-specific 依赖 + `#[cfg]` 变体**（先例：`SetHostRender` + cfg(windows) 的 aurora-bridge）。禁止无条件平台依赖。

### 演化约定

- **不要给枚举加 `#[non_exhaustive]`**：全部消费者同 workspace，穷尽匹配正是加变体时的编译期消费者清单——新变体一加，编译器逐个点名 aurora-ui 的 match 与 aurora-mobile 的映射层，机械化「逐一标注接/不接」流程。（移动端的**绑定层**在 WindInputAndroid 仓，不在本 workspace 内；它只看 aurora-mobile 的门面，故此处的编译期点名到 aurora-mobile 为止。）
- 加 `UiCommand` 变体时按域放进对应注释分节；aurora-ui 侧 `manager.rs` 的 `pub use` 清单同步补（重复列出会 E0252）。
- derive/手写 impl 是行为契约：`MenuItemSpec` 的 `PartialEq` 是 popup_menu 增量重绘判据。

## 构建与测试

纯数据 crate，host 直接 `cargo test -p aurora-ui-types`（任意平台）。

## 依赖

### 内部

- `aurora-theme`；`aurora-bridge`（仅 `cfg(windows)`，HostRenderArc 载荷）

### 外部

- 无

## 全局约束

按需引用根 `AGENTS.md` 的平台分层策略（本 crate 是那张表的第一格归属）。本 crate 特有：

- 三条红线（仅纯数据、依赖过 `cargo check-android`、平台载荷走 target-specific + cfg 变体）是跨平台构建承诺的一部分，改依赖或加变体前先过上一节。

<!-- MANUAL: 此行以下为人工补充区，重新生成时保留 -->
