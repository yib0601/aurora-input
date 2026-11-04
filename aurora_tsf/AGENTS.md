<!-- Parent: ../AGENTS.md -->

# aurora_tsf（wind_tsf）— Windows TSF 输入法桥接层

> 已并入 **WindInput** 仓库（`aurora_tsf/`）：一个 C++17 的 Windows TSF DLL，经 Named Pipe 与 Rust 版 `aurora_input` 服务通信，是输入法在 Windows 宿主里的接口层。IPC 协议镜像 Rust 侧 `crates/aurora-ipc`（`include/BinaryProtocol.h`）；默认经 **clang + cargo-xwin 的 MSVC SDK 在 Linux 交叉编译**（`Makefile`，由 `../scripts/dev.sh` 的 `m1`/`dm1` 调用），亦保留 Windows 原生 MSVC/CMake 构建。下文历史性提到的 "Go service" 即指现在的 Rust 服务（协议未变）。

## 职责

C++17 DLL，为极光输入法（WindInput）实现 Windows Text Services Framework（TSF）接口：

- 向 Windows TSF 注册为系统级输入法
- 捕获键盘事件，经 Named Pipe IPC 转发给 `aurora_input` 服务（Rust）
- 管理组合、光标位置跟踪与候选选择
- 提供语言栏 UI 集成与热键管理
- 实现组合文本的显示属性（下划线）
- 经二进制协议与服务保持状态同步
- 提供 HostWindow 机制，在宿主进程（如开始菜单）内通过 CreateWindowInBand 创建带外层级窗口，解决 Win11 开始菜单候选框 z-order 问题

DLL 导出标准 TSF COM 接口（DllCanUnloadNow、DllGetClassObject、DllRegisterServer、DllUnregisterServer）。
两条构建路径编同一份源码：**Linux→Windows 交叉（`Makefile`，clang + cargo-xwin 的 MSVC SDK，本仓库默认）** 与 **Windows 原生 MSVC（`CMakeLists.txt`）**。

> **注意：wind_dwrite.dll 已移除。** DirectWrite 渲染改由对端服务直接调用系统 dwrite.dll，C++ 侧不再构建 wind_dwrite 目标。
> **MinGW 兼容垫片（历史遗留，当前构建路径不走它）**：mingw-w64 自带 TSF 头不完整，缺失的接口/GUID/常量由 `include/mingw_tsf_compat.h` +
> `src/mingw_tsf_compat.cpp` 补齐（整体 `#ifdef __MINGW32__`，MSVC 构建为空）。`Makefile` 现在用 clang 打 MSVC 目标，`__MINGW32__` 不再定义，这两个文件编成空。

> **本目录正被 Rust 版逐模块替换**：`aurora_core/crates/aurora-tsf-win` 与本目录对位，两者**不是并存关系**——该 crate 每替换一个模块就删掉对应的 C++ 实现（见其 `lib.rs` 顶部说明）。改本目录前先确认该模块是否已被替换。

## 关键文件

| 文件 | 说明 |
|------|------|
| `Makefile` | Linux→Windows 交叉编译（clang + cargo-xwin 的 MSVC SDK，本仓库默认路径）；依赖 clang-19+ / lld-19 / llvm-19，`DEV_VARIANT=1` 出 dev 变体 |
| `CMakeLists.txt` | MSVC/Windows 原生构建（C++17, UTF-8；唯一目标 wind_tsf.dll；从 res/version.rc.in 生成版本资源） |
| `include/mingw_tsf_compat.h` / `src/mingw_tsf_compat.cpp` | MinGW TSF 兼容垫片（仅 `__MINGW32__`；补缺失接口/GUID/常量） |
| `wind_tsf.def` | Module definition file (exports COM entry points) |
| `README.md` | Project documentation |

子目录：

| 目录 | 内容 |
|------|------|
| `include/` | 头文件，见 [`include/AGENTS.md`](include/AGENTS.md) |
| `src/` | 实现文件，见 [`src/AGENTS.md`](src/AGENTS.md) |
| `res/` | 资源文件：图标 + version.rc.in 模板，见 [`res/AGENTS.md`](res/AGENTS.md) |

## 硬约定

### IPC 通信

**Named Pipes：**

- `\\.\pipe\aurora_input` - 主命令管道（C++ → 服务，双向）
- `\\.\pipe\aurora_input_push` - 异步推送管道（服务 → C++，主动状态更新）

**二进制协议：**

- Header：8 字节（version、command、payload length）
- Payload：变长，UTF-8 编码文本
- 主要类型：KeyEvent、CommitRequest、CaretUpdate、FocusGained/Lost、IMEActivated/Deactivated、ToggleMode、MenuCommand 等

**跨语言协议同步（必读）：**
本目录的 [`include/BinaryProtocol.h`](include/BinaryProtocol.h) 与 [`src/IPCClient.cpp`](src/IPCClient.cpp) 是 Rust 侧真源（`aurora_core/crates/aurora-ipc` 的 `protocol.rs` / `codec.rs`）的 C++ 镜像。**修改命令码、Header 字段、Payload 结构、状态标志位时，必须双边同步**，否则会破坏 IPC 兼容性。判据不靠人眼：命令码由 `gen_protocol` 对账，字节布局由 `aurora-ipc/testdata/wire_vectors.json` 真值表对账（TSF / Linux / macOS 三端读同一份），两层都过才算改完。

**Circuit Breaker：**

- Handles service unavailability gracefully
- Max 3 consecutive failures before opening circuit
- 3-second reset interval before retry
- SERVICE_READY 同步期间以 `ERROR_PIPE_BUSY` 收尾的连接失败不计入（服务活着、只是监听实例
  被别的宿主抢走），按键路径照旧计数；收到 SERVICE_READY 时重置熔断器
- SERVICE_READY 后 IME_ACTIVATED 发不出去会按 `ServiceReadyRetryPolicy.h` 有限次带抖动重试
  （与组合态推迟共用 `TIMER_ID_SERVICE_READY`）。重试只在前台线程做；按键 / 焦点路径在本连接
  上同步过就停——再发 IME_ACTIVATED 会让服务端按初始模式重置中英
- 服务重启后主管道句柄是旧连接（`IsConnected()` 仍为真，写入报 232）：收到 SERVICE_READY 时
  `DiscardStaleConnection` 先丢掉它（Peek 失败才丢，活连接不动）；没经过 SERVICE_READY 的，
  `_SendBinaryMessage` 在**写过的**连接上遇到对端已断类错误时重连重写一次，不计熔断
- focus_gained 仍发不出去时 `ScheduleFocusRecoveryRetry` 另起定时器（`TIMER_ID_FOCUS_RECOVERY`，
  次数与间隔同就绪重试、计数分开），到点**只补 focus_gained**、不发 IME_ACTIVATED——补发窗口里
  用户可能已手切中英，重置它比工具栏晚出更糟。判据 `DecideFocusRecovery`；本连接上有过按键
  （`SendKeyEvent` 也置 `_stateEstablished`）或焦点 / 激活往来就停
- 回归测试 `tests/win/ipc_reconnect_test.cpp`：链接真实 IPCClient.cpp + 测试专用管道名的服务端，
  模拟重启；`make -C aurora_tsf/tests/win run`（wine），exe 也可拷到 Windows 直接跑。
  放子目录是因为 CI 会拿 g++ 编 `tests/` 顶层的 `*_test.cpp`

### 组件架构

**Core TSF Integration（TextService）**

- `CTextService` - Main TSF text input processor (ITfTextInputProcessor, ITfThreadMgrEventSink, ITfCompositionSink, ITfDisplayAttributeProvider)
- `CClassFactory` - COM class factory for instantiation
- `CDisplayAttributeInfo` - Composition text styling (underline effect)
- `CCaretEditSession` - TSF edit session for caret position retrieval（已修复 edit session 调用时序问题）
- Full state sync mechanism (`_DoFullStateSync()`) after reconnection

**Input Processing（KeyEventSink）**

- `CKeyEventSink` - Keyboard event capture (ITfKeyEventSink)
- Modifier key state machine (tracks Shift/Ctrl/Alt/Win state, replaces GetAsyncKeyState)
- Barrier mechanism for commit requests (Space/Enter/number key coordination with 服务)
- Barrier timeout handling (500ms default)
- Toggle key tap detection (500ms threshold)
- Composition state tracking and reset on focus loss
- Read-only context detection (browser support)

**IPC Communication（IPCClient）**

- `CIPCClient` - Named pipe client with circuit breaker, async reader thread
- Binary protocol serialization/deserialization (v1.1)
- Async reader for receiving state pushes from 服务
- Batch event support for performance optimization
- Timeout handling and error recovery (100ms connect, 50-100ms read/write)
- Circuit breaker state management (3 failure threshold, 3-second reset interval)
- Separate read pipe for async push notifications

**UI Integration（LangBarItemButton）**

- `CLangBarItemButton` - Language bar button (ITfLangBarItemButton, ITfSource)
- Mode/width/punctuation/toolbar toggle menu
- Context menu for settings/dictionary/about/exit
- Thread-safe updates via message window (for async callbacks)
- Caps Lock state indicator
- Screen-aware context menu positioning

**Hotkey Management（HotkeyManager）**

- `CHotkeyManager` - Hotkey whitelist from 服务
- O(1) lookup using unordered_set
- Classification: toggle mode, letter, number, punctuation, backspace, enter, escape, space, tab, page key, cursor key, select key
- Key normalization (left/right modifier handling)

**HostWindow（开始菜单宿主窗口代理）**

- `CHostWindow` - 在宿主进程（如 SearchHost.exe）内通过 `CreateWindowInBand`（user32.dll 非公开 API）创建与宿主同级 Band 的分层窗口
- 服务通过共享内存传递渲染帧（像素数据），HostWindow 的渲染线程读取 SharedRenderHeader 并 BitBlt 到分层窗口
- `_ResolveAPIs()` - 动态解析 CreateWindowInBand 和 GetWindowBand 函数指针
- `_GetHostBand()` - 获取宿主进程前台窗口的 Band 等级
- `_CreateBandWindow()` - 在宿主进程的 Band 等级创建无边框分层窗口
- `_RenderThread()` / `_RenderLoop()` - 渲染线程，等待事件信号后读取共享内存渲染一帧
- 支持跳过过期帧（lastSequence 机制）

**File Logging（FileLogger）**

- `CFileLogger` - 运行时可配置的文件日志单例（`FileLogger.h` / `FileLogger.cpp`）
- 四种输出模式：`none`（默认，零开销）/ `file` / `debugstring` / `all`
- 日志文件：`%LOCALAPPDATA%\AuroraInput\logs\tsf_log\wind_tsf.<宿主名>.<pid>.log`——
  **每进程一个**，且单独放在 `tsf_log\` 子目录里（文件数是「用过的宿主 × pid」量级，
  与 core 的 `aurora_input.log` 平铺会把主日志淹掉）
- 配置文件：`%LOCALAPPDATA%\AuroraInput\logs\tsf_log_config`（mode / level / dump_hotkey 三个键）——
  **留在 `logs\` 这一层不跟着进子目录**：它是用户手工创建的日志总开关，搬走会让存量失效
- `dump_hotkey=1` 才启用**环形缓冲 + Ctrl+Shift+F12 导出**，**出厂关**。这个热键的拦截排在
  `OnTestKeyDown` / `OnKeyDown` 的所有闸门之前（早于 `IsKeyboardDisabled`、密码框抑制、
  只读上下文），开着就意味着「只要本输入法激活，Ctrl+Shift+F12 永远到不了宿主」——
  而它在 VS / JetBrains 里是有主的快捷键，我们的处理还会往焦点处插一行提示文本。
  判据收在 `CKeyEventSink::_IsLogDumpHotkey`（吃与导出两处共用，不许各写一份）
- ⚠ 改完 `tsf_log_config` 要**重启宿主进程**才生效：唯一的重读入口 `ReloadConfig` 就挂在
  上面那个热键上，而它默认关着。建文件时把 mode / level / dump_hotkey 一次写齐
- dev 变体只换目录（`WindInputDev\logs\`），**文件名与配置名同正式版**——目录已隔离，
  文件名不再重复带 `_dev`
- 多进程安全：**每进程独占自己那个文件，无需任何跨进程同步**。此前是一把
  `Local\WindInput*TSFLogMutex` 串行化所有宿主，而抢锁发生在 TSF 输入线程上
- 常开追加句柄（`FILE_APPEND_DATA`），一行日志只剩一次 `WriteFile`
- 自动轮转：超过 5MB 时重命名为 `wind_tsf.<宿主名>.<pid>.old.log`
- **外部改动日志文件是受支持的**（`_ResyncFile`，每秒自检一次）：
  - 删掉文件或整个 `tsf_log\` 目录 → 一秒内自动重建，**不必重启宿主**
  - 清空文件（截断到 0）→ 追加句柄天然从头续写，不留 NUL 空洞，排查时可随时截断
  - 不做自检的话，删除后写入会**静默进入一个已摘名的幽灵文件**：`WriteFile` 照常返回
    成功、目录里却什么都没有，很容易误判成「这功能压根没跑」
- 过期文件的回收由 **core 服务启动时**做（`log_rotate::prune_stale_tsf_logs`，默认 7 天）：
  DLL 的 `Init()` 跑在 loader lock 下，不能做目录遍历
- 在 `dllmain.cpp` 的 DLL_PROCESS_ATTACH / DLL_PROCESS_DETACH 中 Init/Shutdown

### CLSID / GUID 正式化

CLSID 已替换为正式 UUID 并集中管理（Globals.h / Globals.cpp）：

- `c_clsidTextService` = `{99C2EE30-5C57-45A2-9C63-FB54B34FD90A}`
- `c_guidProfile`      = `{99C2EE31-5C57-45A2-9C63-FB54B34FD90A}`

安装脚本中的 InstallLayoutOrTip 调用使用同一套 GUID。

### 在本目录工作时的约定

When implementing features or fixes in wind_tsf:

1. **Read the binary protocol** (BinaryProtocol.h) before modifying IPC communication
2. **Understand TSF lifecycle:** Activate (thread manager registration) -> Initialize components -> Deactivate
3. **Use logging macros** from Globals.h (AURORA_LOG_ERROR_FMT, AURORA_LOG_DEBUG, etc.) instead of printf
4. **COM reference counting:** Use SafeRelease() template for interface cleanup
5. **Named pipes:** Connection is lazy (on-demand), with circuit breaker fallback
6. **Edit sessions:** For TSF API calls (composition, caret position), must be called within RequestEditSession
7. **HostWindow:** 只在 compat.toml 中 `host_render = true` 的进程中激活（由 Rust 服务通过 IPC 指令触发 Initialize）
8. **UI-less（宿主自绘候选）:** `ITfCandidateListUIElement` 在宿主 `pbShow=FALSE` 时答真实快照（`CMD_UIELEMENT_QUERY` 拉取），否则答占位；`_uiElementShown` 记的是宿主意愿，EndUIElement 不得清它。实现与判据见 `src/TextService.cpp` 的 UIElement 段与 `include/UiElementPolicy.h`

### 通用代码式样

**Key Event Handling：**

```cpp
// CKeyEventSink::OnKeyDown() flow:
1. Update modifier state machine (_UpdateModsOnKeyDown)
2. Check hotkey whitelist (CHotkeyManager::IsKeyDownHotkey)
3. For special keys (Space/Enter), create commit request with barrier
4. For normal input, send key event to 服务 via CIPCClient
5. Check service response (key consumed vs passed through)
```

**Composition Updates：**

```cpp
// CTextService::UpdateComposition() flow:
1. RequestEditSession with TF_ES_SYNC
2. Inside CUpdateCompositionEditSession:
   - Get composition range from context
   - Replace text with new composition
   - Set caret position
   - Apply display attribute
```

**State Synchronization：**

```cpp
// Full state sync (after reconnection):
1. Call _DoFullStateSync() which sends IMEActivated
2. 服务 responds with StatusUpdate (mode, width, punct, caps lock state)
3. CTextService::_SyncStateFromResponse() applies status
4. Update language bar and internal state flags
```

**Async Reader Thread：**

```cpp
// Async push from 服务 (e.g., user clicked candidate):
1. CIPCClient::StartAsyncReader() spawns async read thread
2. Thread listens on separate push pipe
3. Calls registered callbacks:
   - StatePushCallback for status updates
   - CommitTextCallback for candidate selection
   - ClearCompositionCallback for mode toggle via menu
4. Main thread posts message to CLangBarItemButton::_hMsgWnd for UI updates
```

### 常见任务

**加一个 TSF 事件**

1. 给 `CTextService` 加事件 handler（实现对应 `ITf*Interface`）
2. 路由到相应组件（KeyEventSink、IPCClient 等）
3. 用 `AURORA_LOG_*` 宏记日志
4. 用 Windows Input Method Tester（imm32tst.exe）验证

**加一条 IPC 命令**

1. 在 BinaryProtocol.h 定义命令 ID（`CMD_*` 常量）
2. 需要时加 payload 结构体（必须 packed）
3. 在 `CIPCClient` 实现发送方法
4. 在 `CIPCClient::_ParseResponse()` 实现解析
5. 同步更新服务侧处理该命令
6. 验证二进制协议兼容性（真值表要一起过）

**排查 IPC 问题**

1. 开启文件日志：建 `%LOCALAPPDATA%\AuroraInput\logs\tsf_log_config`，写 `mode=file` 与 `level=debug`
   （要用 Ctrl+Shift+F12 导出环形缓冲的话再加一行 `dump_hotkey=1`，见 File Logging 一节；
   这些键只在**宿主进程启动时**读一次）
2. 看日志 `%LOCALAPPDATA%\AuroraInput\logs\tsf_log\wind_tsf.<host>.<pid>.log`（每个宿主进程一个文件；
   按时间戳合并多个宿主：`sort -m -k1,2 tsf_log/wind_tsf.*.log`，格式与 `aurora_input.log` 逐字对齐）
3. 要实时输出：设 `mode=debugstring` 并用 DebugView.exe
4. 用 NamedPipeMon.exe 监视命名管道
5. 查服务侧日志里的解析错误
6. 核对协议版本一致（BinaryProtocol.h 的 PROTOCOL_VERSION）

## 构建与测试

### Linux→Windows 交叉编译（clang + cargo-xwin 的 MSVC SDK，默认）

⚠️ 依赖 `clang++-19` / `lld-19` / `llvm-rc-19` 与 `~/.cache/cargo-xwin/xwin` 的 MSVC CRT/SDK
（由 cargo-xwin 下载）；`../scripts/dev.sh` 的 `setup_xwin_env` 会一并设好 `XWIN_ARCH=x86,x86_64`。

```bash
cd aurora_tsf
make                       # → build/wind_tsf.dll（PE32+ x64，静态链接）
make DEV_VARIANT=1         # → build_dev/wind_tsf_dev.dll
make VERSION=1.0.0         # 指定版本号写入资源
# 或经仓库脚本：../scripts/dev.sh m1（release）/ dm1（dev），会同时出 x64 + x86
```

> ⚠️ 交叉编出的 DLL 在带安全加固的宿主里 COM 激活失败，**只能用来验证「编不编得过」**；
> 交付产物一律由 Windows 编译机的原生 MSVC 产出（见根 `AGENTS.md`「构建 / 测试」）。

### MSVC/Windows 原生（可选）

```bash
cd aurora_tsf && mkdir -p build && cd build
cmake .. -G "Visual Studio 17 2022" -A x64
cmake --build . --config Release      # → build/Release/wind_tsf.dll
# 版本号：cmake .. -DAPP_VERSION_STR="1.0.0" -DAPP_VERSION_MAJOR=1 ...
```

### 构建验证

- `cmake --build . --config Release` 必须成功、无 C++ 编译错误
- 产出唯一目标：`wind_tsf.dll`（不再产出 wind_dwrite.dll）
- wind_tsf.dll 必须导出 4 个函数：DllCanUnloadNow、DllGetClassObject、DllRegisterServer、DllUnregisterServer

### 注册

- 必须调用 `DllRegisterServer()` 才能注册进 Windows TSF
- 创建 `HKEY_CURRENT_USER\Software\Microsoft\Windows NT\CurrentVersion\IMEUI...` 注册表项
- 向 TSF manager 注册 Profile（GUID）

### 手工测试

- 注册 DLL：`regsvr32 wind_tsf.dll`
- 在 Windows 设置里切换输入法并选极光输入法
- 打中文：键盘输入应触发服务
- 语言栏应显示模式指示
- 语言栏右键菜单应可用

### 协议验证

- 用 Named Pipe Monitor 抓 DLL 与服务之间的二进制消息
- 核对 payload 结构与 BinaryProtocol.h 的定义一致

## 依赖

### 内部

- `BinaryProtocol.h` - 与服务共享的二进制协议定义
- `Globals.h` - 日志宏、COM 工具、全局状态

### 外部

- **Windows SDK：** msctf.h、ctfutb.h（TSF 接口）
- **Windows 系统库：** kernel32、ole32、user32、winuser.h（COM、窗口管理）
- **C++ 标准库：** string、vector、unordered_set

## 全局约束

跨组件与全仓约定按需引用根 [`AGENTS.md`](../AGENTS.md)——尤其「跨组件硬约定」里的**协议面三层闸门**与 **C++ 吃键集必须 ⊆ Rust 出字集**，以及「构建 / 测试」一节里「编译机必须是 Windows + 原生 MSVC」那条（本目录的交付产物只能由编译机产出）。本节只补本目录特有的：

- **宿主进程里跑的是这份 C++，不是 Rust**：凡是能由 TSF 线程直接观测到的行为（吃键、光标、语言栏、Band 窗口），判据都要在 C++ 侧可解释；别把「服务端事后会纠正」当设计。
- **每次改了协议/状态位**：先跑 `make` 确认编得过，再让真值表对账，最后才上靶机验证输入链路（wine 只够验非宿主逻辑）。
