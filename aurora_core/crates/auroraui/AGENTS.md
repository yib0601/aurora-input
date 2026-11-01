# AGENTS.md — auroraui 开发手册

> 面向在本仓库**改代码**的 AI agent 与贡献者。讲清"怎么干、守什么规矩、别踩什么坑"。
> 用户/库使用者请看 [`docs/API_GUIDE.md`](docs/API_GUIDE.md)；架构原理看 [`docs/DESIGN.md`](docs/DESIGN.md)。

auroraui = 轻量 Windows 桌面 GUI 框架（Win32 窗口 + GDI 呈现 + tiny-skia 图形 + DirectWrite 文字，无运行时、无 GC）。
核心指标是**极低内存**与**空闲零 CPU**——任何改动都不应破坏这两点。

内存要**连同窗口尺寸与 DPI 一起说**才有意义：大头是软件光栅按**物理**像素留的约 2.5 份
全屏 RGBA 缓冲，200% 缩放下同一窗口就是 100% 的 4 倍。当前基准（`scripts/measure_footprint.ps1`
实测）：最小窗口 480×320 私有 2.7MB@100% / 4.6MB@200%；关于窗 620×556 私有 5.5MB@100% /
14.2MB@200%。判断"是不是回归"请在同一 DPI 下跑同一个示例比对，别拿裸数字对号入座。

## 文档地图（先读哪个）
| 我要… | 看 |
|-------|-----|
| 用这个库写应用 | `docs/API_GUIDE.md` |
| 懂为什么这样设计（内存模型、三阶段布局、借用规避） | `docs/DESIGN.md` |
| 看已交付的阶段与验收方式 | `docs/ROADMAP.md` |
| 在仓库里加控件/改框架 | **本文件** |
| 改 Linux 后端 / 在无桌面机器上做真窗口验证 | `docs/LINUX_PORTING.md` |

---

## 0. 黄金法则（TL;DR）

1. **Widget 是纯内容**——不持有、不访问节点树。跨节点协调一律走共享状态 `Rc<Cell<T>>`。
2. **控件不硬编码视觉**——颜色/间距/字号一律读 `theme::current()`，新控件须接入 `Theme`。
3. **平台差异收口在 `platform/` 层**——控件与核心层保持平台无关（Windows / macOS / Linux 共用）。
4. **OS 重入前释放借用**——平台层调用可能回调进 `wnd_proc` 的 API 前，先放掉 `state` 的可变借用（两段式）。
5. **作者与审查分离**——实现后由独立 `code-reviewer` 审，不在同一上下文自审自批。
6. **提交不含 AI 元信息**——简体中文 conventional commit，无 `Constraint/Rejected/Confidence` 等 trailer、无 `Co-Authored-By`。

---

## 1. 环境与命令

```bash
cargo build                              # 编译库
cargo build --examples                   # 编译所有示例（改公共 API 后必跑）
cargo clippy --all-targets               # lint（零警告才算过）
cargo test                               # 单元测试（须全绿）

# 非默认档：CI 跑，本地默认档编译不到。改 src/render/gpu/、src/platform/、或任何会被
# 后端缓存键引用的类型（TextStyle、Image…）时必跑——漏过两次真实缺陷，见 CONTRIBUTING。
cargo clippy --no-default-features --all-targets
cargo clippy --all-targets --features gpu
cargo test --features gpu --lib

cargo run --release --example fullshowcase           # 运行综合示例窗口
cargo run --example <name> -- --screenshot out.png   # 离屏渲染存 PNG
powershell scripts/screenshots.ps1                   # 一键生成所有示例截屏
```

平台：Windows，PowerShell 为主（Bash 工具可用于 POSIX 脚本）。包管理 cargo。
Linux 上上述命令同样可跑（单测、截图回归都不需要 X 服务器）；真窗口验证见 `docs/LINUX_PORTING.md` §6。

---

## 2. 仓库地图

| 路径 | 职责 |
|------|------|
| `src/lib.rs` | crate 门面 + `prelude` |
| `src/app.rs` | `App` 构建器 + `UiHost`（交互宿主：render / 事件 / 焦点 / 菜单 / 触摸惯性） |
| `src/core.rs` | `Tree`（generational arena）+ `Node` + `Widget` trait + `EventCtx` + 三阶段布局 + 事件分发 |
| `src/ui/mod.rs` | `Element` 构建器（控件构造 + 链式修饰符 + `build`） |
| `src/ui/{inputs,select,progress,stepper,list,containers,virtual_list,row_source}.rs` | 各 `Widget` 实现 |
| `src/geometry.rs` | `Color / Point / Size / Rect / Insets` |
| `src/spec.rs` | `Align / Axis / Dimension / MeasureSpec` |
| `src/style.rs` | `Style`（内联视觉属性） |
| `src/theme.rs` | `Theme / Palette / Metrics` + thread_local `current()/set_current()` |
| `src/event.rs` | `Event / PointerEvent / KeyEvent / Key / MenuItem` |
| `src/anim.rs` | `request_repaint()` + 帧时钟 |
| `src/render/{mod,skia}.rs` | `Canvas` trait + tiny-skia 后端 |
| `src/text/{mod,dwrite}.rs` | `TextEngine` + DirectWrite |
| `src/platform/{mod,win32/mod,win32/clipboard}.rs` | `AppHandler` trait + Win32 窗口/消息循环/剪贴板 |
| `src/platform/linux/` · `src/text/linux/` | Linux（X11 / Wayland）平台层与自带文字栈 |
| `src/testing.rs` | 给下游写测试用：借 `EventCtx` 跑回调并收回副作用 |

**热点文件**（改动频繁且牵一发动全身，改前务必通读相关段落）：`src/ui/inputs.rs`、`src/platform/win32/mod.rs`、`src/core.rs`。

---

## 3. 架构铁律（违反即返工）

1. **Arena 存树，非 `Rc<RefCell>`**：节点在 `Tree` 的 generational arena，`NodeId{index,generation}` 删除后代际失效。布局递归由 `Tree` 独占 `&mut self` 驱动——从根上规避借用冲突。
2. **三阶段**：`measure`（控件返回固有尺寸）→ `arrange`（框架定位，控件不参与）→ `paint`（控件在分配到的 `bounds`/`content` 内绘制）。控件只实现 `measure`/`paint`/`on_event`。
3. **坐标系**：控件树用**逻辑坐标**，绘制层统一 ×`scale` 到物理像素；`on_event` 收到的 pos 已是逻辑坐标。文字测量与绘制按**物理字号同源**，勿混用。
4. **视觉走主题**：控件 `paint` 读 `theme::current()`；`Theme` 两层（`palette`+`metrics` 基底 / 每控件 `Option<Color>` 覆盖层，`None` 回退 palette）。详见记忆与 `docs/API_GUIDE.md` §7。
5. **平台缝合**：点击计数 / 剪贴板 / 输入法光标 / 触摸 / 动画驱动等平台差异，全部以 trait 收口在 `platform` 层；核心与控件零平台依赖。
6. **两段式借用**：`win32` 里凡是先借 `state_from(hwnd)`、随后又要调 `InvalidateRect`/`DestroyWindow`/弹菜单等可能同步重入 `wnd_proc` 的 API 时，**先把意图读出/写完并释放 state 借用，再调 OS API**。参考 `dispatch_pointer_event`/`dispatch_pan`/`dispatch_fling`。
7. **空闲零 CPU**：无动画时阻塞在 `GetMessageW`；需要持续动画的控件在 `paint` 内调 `anim::request_repaint()`，宿主按显示器刷新率（≤60fps，动画期 `timeBeginPeriod(1)`）配速。停止请求即回到零 CPU。

---

## 4. 新增控件接入清单（逐项勾完才算完成）

1. [ ] `Widget` 实现于 `src/ui/<name>.rs`；`paint` 读 `theme::current()`，**不硬编码**颜色/间距/字号。
2. [ ] `Theme` 加对应覆盖层 struct（`Option<Color>` 字段 + 回退方法），并接入 `Theme` 聚合 + TOML（`#[serde(default)]`）。
3. [ ] `Element::<name>()` 构造器（`src/ui/mod.rs`）+ 模块注册 + 必要的 `pub use` 导出。
4. [ ] **在 `examples/fullshowcase.rs` 的"控件"标签页加展示卡片**（硬性要求，集中展示避免单页过长）。
5. [ ] 契约单测：经真实 `dispatch_pointer`/`dispatch_key` + `abs_bounds` 验证交互，**不写 mock**；
   涉及选区/命中/布局缓存的，须**同时覆盖 Wrap 宽与显式宽**（见 `docs/DEVELOPMENT.md` §3「宽度不能只测一种」）。
6. [ ] 独立 `code-reviewer` 审查 → 修复 → 截图验证 → 单独 commit。

状态绑定沿用统一模型：开关 `Rc<Cell<bool>>`、索引 `Rc<Cell<usize>>`、数值 `Rc<Cell<f32/f64>>`、文本 `Rc<RefCell<String>>`。

---

## 5. 测试与验证（细则见 `docs/DEVELOPMENT.md` §3）

- **单测经真实路径**：构造 `Tree`、`build` Element、跑 `layout_root` + `dispatch_*`，断言状态与 `abs_bounds`/`scroll_y`。不 mock 框架内部。
- **视觉改动必须截图核对**：`--screenshot` 出 PNG；输入中/拖拽/尺寸下限这类状态用 `--type`/`--key`/`--drag`/`--size` 合成，参数与命中诊断见上面那份文档。
- **涉选区/命中/布局缓存**：同时覆盖 Wrap 宽与显式宽，只测显式宽会整批漏掉 `avail.w != content.w` 的路径（实例见该文档「宽度不能只测一种」）。
- 改公共 API 后跑 `cargo build --examples`。
- 只能真机验证的特性（触摸惯性等）：单测覆盖可测部分后请用户实测，别声称"已验证"。

---

## 6. 开发工作流

宽泛需求先探索再动手；2+ 独立任务并行。实现 → `cargo build` + `cargo clippy --all-targets` 零警告（碰渲染后端/平台层补跑 §1 非默认两档）→ 视觉改动 `--screenshot` 核对 → **独立 `code-reviewer` 审查**（不自审）→ 提交（见 §7）。

---

## 7. 提交规范

- **Conventional commit** + 简体中文 subject/body：`feat(scope): …` / `fix` / `refactor` / `docs` / `test` / `chore`，说清"做了什么、为什么"。
- **绝不含 AI 元信息**：无 `Constraint:`/`Rejected:`/`Confidence:`/`Scope-risk:` 等 trailer，无 `Co-Authored-By`、无工具署名。
- **不含个人信息**：提交信息与入库的注释/文档不写真实姓名、邮箱、内网地址、主机名、含用户名的本机路径；引用文件一律仓库相对路径。
- **原子提交**：一组相关改动一个 commit。只在用户要求时提交。

---

## 8. 命名与 API 约定（细则见 API_GUIDE §4）

- 背景/前景统一缩写 **`bg`/`fg`**（`Element::bg`、`App::bg`、`Style.bg`、`EventCtx::set_bg`）。
- 文本标签统一 **`impl Into<String>`**（`button`/`label`/`dropdown`/`list`/`tabs`）。
- 构造器=控件名（名词），布局/样式修饰符=属性名、**不加 `set_`**；getter **不加 `get_`**；`set_` 仅命令式副作用。
- 右键**默认不激活控件**；仅需右键的控件 override `Widget::wants_right_click() -> true`。
- text_input 专属修饰符（`password/multiline/wrap/read_only`）链到他控件 **debug 期 `panic`** 提示误用。

---

## 9. 高频陷阱

| 陷阱 | 规避 |
|------|------|
| 触摸惯性中途打断残留偏移 | 任何打断 fling 的路径（新触摸 Down / 指针 Down / pan / cancel）**必经 `clear_fling` 复位 `over_scroll`**。 |
| 动画冻结 / 空转 | 需要动画忘了 `request_repaint` 会冻结；停止条件忘了清请求会每帧空转。每帧 `reset_request` 后由控件/逻辑重新请求。 |
| DPI 坐标混用 | 控件内一律逻辑坐标；物理像素只在平台层/绘制层出现。 |
| OS 重入借用冲突 | 见铁律 §6 两段式借用。 |
| 改了视觉硬编码颜色 | 走 `theme::current()`，破坏主题一致性会被审查打回。 |
| 改公共 API 漏改示例 | `cargo build --examples` 兜底。 |
| 选区/命中类测试全绿但真机失效 | 测试若清一色钉死宽度，会漏掉 Wrap 宽下 `avail.w != content.w` 的路径。至少留一个不设宽用例，见 `docs/DEVELOPMENT.md` §3。 |
| `LF will be replaced by CRLF` 警告 | Windows 换行符提示，无害，忽略。 |

---

## 10. 文档维护（改完别忘了）

- 改/加**公共 API** → 同步 `docs/API_GUIDE.md`。
- 改**架构/不变量** → 同步 `docs/DESIGN.md`。
- 加**控件/能力** → 更新 `README.md` 控件表（必要时 `docs/ROADMAP.md`）。
- 改**开发约定/流程** → 更新本文件。

> 框架处于早期，以"最新设计 + 统一"为准，**不承诺向后兼容**。
