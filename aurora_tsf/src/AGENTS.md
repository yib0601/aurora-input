<!-- Parent: ../AGENTS.md -->

# src/ — TSF DLL 实现文件

> `wind_tsf.dll` 的 C++17 实现：全部 .cpp 编译链接进唯一目标 `wind_tsf.dll`，按组件（text service / IPC / hotkey / UI / host window / logging）与入口点（dllmain）组织。

## 职责

实现 TSF DLL 的全部行为，声明见 `../include/`：

- DLL 入口与 COM 导出（`dllmain.cpp`）
- TSF 文本服务：组合、光标跟踪、状态同步（`TextService.cpp`）
- 按键捕获与修饰键状态机、barrier（`KeyEventSink.cpp`）
- 命名管道客户端与二进制协议、熔断器、异步读取（`IPCClient.cpp`）
- 语言栏 UI 与菜单（`LangBarItemButton.cpp`）
- 热键白名单与分类（`HotkeyManager.cpp`）、注册表注册（`Register.cpp`）
- 运行时可配置文件日志（`FileLogger.cpp`）、宿主进程内 Band 窗口渲染（`HostWindow.cpp`）

> **注意：`WindDWriteShim.cpp` 已移除。** DirectWrite 渲染改由 Rust 服务侧通过 CGO 直接调用系统 dwrite.dll，不再在 C++ 侧构建。

## 关键文件

| 文件 | 说明 |
|------|------|
| `dllmain.cpp` | DLL 入口（DllMain、DllCanUnloadNow、DllGetClassObject、DllRegisterServer、DllUnregisterServer） |
| `Globals.cpp` | 全局状态初始化（HINSTANCE、引用计数、GUID 定义） |
| `TextService.cpp` | `CTextService` 实现（TSF 集成、组合、光标跟踪、状态同步） |
| `KeyEventSink.cpp` | `CKeyEventSink` 实现（按键捕获、修饰键状态机、barrier 机制） |
| `IPCClient.cpp` | `CIPCClient` 实现（命名管道、二进制协议、熔断器、异步读取） |
| `ClassFactory.cpp` | `CClassFactory` 实现（`CTextService` 的 COM 类工厂） |
| `HotkeyManager.cpp` | `CHotkeyManager` 实现（热键查找、按键分类） |
| `LangBarItemButton.cpp` | `CLangBarItemButton` 实现（语言栏 UI、菜单、异步更新） |
| `CaretEditSession.cpp` | `CCaretEditSession` 实现（取光标位置的 TSF edit session） |
| `DisplayAttributeInfo.cpp` | 显示属性类（组合文本样式） |
| `Register.cpp` | 注册表集成（DllRegisterServer、DllUnregisterServer、profile/category 注册） |
| `FileLogger.cpp` | `CFileLogger` 实现（Init/Shutdown、读配置、写文件（每进程一文件，**无跨进程锁**）、5MB 自动轮转） |
| `HostWindow.cpp` | `CHostWindow` 实现（创建 Band 级分层窗口、共享内存渲染帧读取、渲染线程、动态解析 CreateWindowInBand/GetWindowBand API） |

## 硬约定

### dllmain.cpp

- `DllMain()` - 进程 attach/detach、线程初始化
- `DllCanUnloadNow()` - server lock 未释放则返回 S_FALSE，否则 S_OK
- `DllGetClassObject()` - 创建并返回 `CClassFactory`
- `DllRegisterServer()` - 向 Windows 注册（转交 `Register.cpp`）
- `DllUnregisterServer()` - 从 Windows 注销（转交 `Register.cpp`）

### TextService.cpp

- `CTextService::Activate()` - 注册 thread manager 事件 sink、初始化各组件
- `CTextService::Deactivate()` - 注销 sink、清理
- `CTextService::OnSetFocus()` - 处理窗口焦点变化（含读取焦点控件 InputScope + 密码框检测，随 focus_gained 上报）
- `CTextService::_QueryInputScopeMask()` - 同步读锁读取焦点文档的 TSF InputScope 集合，编码为 bitmask（bit N = 枚举值 N 存在，IS_PASSWORD=31），失败返回 0。由 `CQueryInputScopeEditSession`（TextService.cpp 内联）在 selection + 文档起点两 range 合并读取
- `CTextService::_IsFocusKeyboardDisabled()` - 读焦点 context 的 `GUID_COMPARTMENT_KEYBOARD_DISABLED`（Weasel/小狼毫做法）判定密码框：Chromium 系浏览器密码框置位、无痕普通框不置位。命中则在 mask 补 IS_PASSWORD 位让服务抑制中文（图标不变）
- `CTextService::UpdateComposition()` - 经 edit session 更新内联组合文本
- `CTextService::InsertText()` - 无组合直接上屏文本
- `CTextService::EndComposition()` - 结束当前组合
- `CTextService::InsertTextAndStartComposition()` - 上屏 + 起新组合（顶码用）
- `CTextService::GetCaretPosition()` - 从 context 取光标位置
- `CTextService::SendCaretPositionUpdate()` - 向服务上报光标更新
- `CTextService::ToggleInputMode()` - 切换中/英模式
- `CTextService::_DoFullStateSync()` - 重连后与服务做一次全量状态同步
- `CTextService::NotifyCandidatesVisibilityChanged()` / UIElement 段 - TSF **UI-less** 支持：宿主
  （全屏游戏、SDL/Unreal、搜索框）在 `BeginUIElement` 回 `pbShow=FALSE` 或以
  `TF_TMAE_UIELEMENTENABLEDONLY` 激活时接管候选绘制。DLL 经 `CMD_UIELEMENT_STATE` 报「谁画」、
  每次候选变化经 `CMD_UIELEMENT_QUERY` 同步拉快照答 `ITfCandidateListUIElement`，宿主的
  `SetSelection/Finalize/Abort` 经 `CMD_UIELEMENT_ACTION` 回流；服务端按 pid 不弹本地候选窗。
  宿主不接管时 getter 仍是占位数据（`GetCount=1`），不拉快照。判据与状态位说明见
  `../include/UiElementPolicy.h` 头注释
- Edit session 辅助类族：`CUpdateCompositionEditSession`、`CEndCompositionEditSession` 等

### KeyEventSink.cpp

- `CKeyEventSink::OnKeyDown()` - 捕获 key down
- `CKeyEventSink::OnKeyUp()` - 捕获 key up
- `CKeyEventSink::OnTestKey()` - 只窥探、不消费
- `CKeyEventSink::OnPreservedKey()` - 处理热键（Ctrl+Space 等）
- `_UpdateModsOnKeyDown()` - key down 时更新修饰键状态机
- `_UpdateModsOnKeyUp()` - key up 时更新修饰键状态机
- `_GetModsSnapshot()` - 取本次事件的修饰键快照
- `_GetTogglesSnapshot()` - 取 CapsLock/NumLock/ScrollLock 状态
- `_SendKeyToService()` - 序列化按键事件发给服务
- `_HandleServiceResponse()` - 解析响应并落地（消费 / 放行）
- `_SendCommitRequest()` - 为 Space/Enter/数字键发带上屏 barrier 的请求（**已实现，尚未调用，见下方 Barrier 说明**）
- `_HandleCommitResult()` - 处理服务返回的上屏结果（**已实现，尚未调用**）
- `_CheckBarrierTimeout()` - 检查 barrier 是否超时（500ms）（每次 OnKeyDown 检查，有挂起时才生效）
- `_IsMatchingKeyUp()` - 把 KeyUp 与挂起的切换键 KeyDown 配对
- `_IsContextReadOnly()` - 检测只读输入框（浏览器支持）
- `OnCompositionUnexpectedlyTerminated()` - 处理被应用终止的组合
- 切换键轻敲判定（500ms 阈值区分模式切换与长按）

### IPCClient.cpp

- `CIPCClient::Connect()` - 连命名管道（带超时）
- `CIPCClient::Disconnect()` - 关管道句柄
- `CIPCClient::IsServiceAvailable()` - 查熔断器与管道状态
- `SendKeyEvent()` - 二进制协议发按键事件
- `SendCommitRequest()` - 发带上屏 barrier 的请求
- `SendCaretUpdate()` - 向服务上报光标位置
- `SendFocusGained()` / `SendFocusLost()` - 焦点通知。FocusGained 为**同步**命令：服务在响应里回传 CMD_MODE_PUSH（权威 chineseMode/fullWidth），本端在 OnSetFocus 内同步写入 `_bChineseMode`，消除首次按键上屏英文竞态；重型状态（工具栏/热键/hostRender）仍由 push pipe CMD_ACTIVATION_STATUS_PUSH 异步回送。FocusLost 仍 async。FocusGainedPayload 现为 38 字节，末尾依次是 8 字节 InputScope bitmask（密码框等语义供服务决策）+ disabled(1B) + reason(1B)（输入闸/语言栏呈现的「密码框 vs context 禁用」判据；原输入诊断 HUD 的展示用途已随 HUD 删除）；定长段之后是三个变长段（bundleId 占位 / 顶层窗口类名 / 顶层窗口标题，布局见 BinaryProtocol.h）。标题只在 core 推开 `CONFIG_KEY_COMPAT_TITLE_MATCH`（本进程可能命中 compat 标题规则）时用 `InternalGetWindowText` 采集，日志只记长度（判据 `WindowTitlePolicy.h`）
- `SendInputStateReport()` - async 上报 `CMD_INPUT_STATE_REPORT`（14 字节：pid u32 + disabled u8 + reason u8 + inputScopeMask u64），用于 compartment 变更但焦点未变（不触发新 OnSetFocus）时让 core 即时重算密码框抑制态与呈现原因（**原「刷新输入诊断 HUD」的用途已随 HUD 整块删除，本上报仍是抑制判据的输入**）；调用点为 `TextService::OnChange` 的 `GUID_COMPARTMENT_KEYBOARD_DISABLED` 分支
- `SendDiagSnapshot()` - 诊断快照（**core 侧输入诊断 HUD 已整块删除**：`CONFIG_KEY_DIAG_SNAPSHOT` 不再被推开，本通道保持默认关、一次 Win32 调用都不做；协议与 C++ 侧保留未删）：async 上报 `CMD_DIAG_SNAPSHOT`（`DiagSnapshotHeader` 64 字节 + 三段 `len u16 + UTF-8` 类名：focus/root/foreground），内容为焦点窗口链、前台窗口、DocMgr/Context 实例 id、band。**由 `CONFIG_KEY_DIAG_SNAPSHOT` 门控，默认关**；采集器 `TextService::SendDiagSnapshotIfEnabled()` 在开关关闭时一次 Win32 调用都不做。调用点：`OnSetFocus`（focus_gained 之后，且在刷新 `_pLastActiveDocMgr` **之前**算 docMgrChanged）与 `OnChange` 的 KEYBOARD_DISABLED 分支。⚠ 刻意不并入 `CMD_FOCUS_GAINED`——那是宿主 UI 线程上的同步往返，首字延迟挂在它身上。焦点窗口句柄带 `WND_SRC_*` 来源标记（TSF view / GUI thread / foreground），三条通路语义不同，尤其 foreground 可能不属于本进程
- `SendIMEActivated()` / `SendIMEDeactivated()` - IME 状态通知（IMEActivated 已改 async；同上）
- `SendModeNotify()` - 上报模式变化（TSF 本地切换，async）
- `SendToggleMode()` - UI 触发的模式切换请求（sync）
- `SendCompositionTerminated()` - 上报组合被意外终止
- `SendAsync()` - 发异步消息（发完不等）
- `SendSync()` - 发同步消息（等响应）
- `ReceiveResponse()` - 从管道解析二进制响应
- `_SendBinaryMessage()` - 底层管道写
- `_ReceiveBinaryMessage()` - 底层管道读
- `_ParseResponse()` - 把二进制响应反序列化成 `ServiceResponse`
- `_WriteWithTimeout()` / `_ReadWithTimeout()` - 带超时的重叠 I/O
- `_RecordSuccess()` / `_RecordFailure()` - 熔断器计数
- `_ShouldAttemptOperation()` - 熔断器决策
- `_Utf8ToWide()` / `_WideToUtf8()` - 编码转换
- `_Log()` / `_LogError()` / `_LogDebug()` - 日志辅助
- 异步读取线程：`_AsyncReaderThread()`、`_AsyncReaderLoop()`、`StartAsyncReader()`、`StopAsyncReader()`
- 批量支持：`BeginBatch()`、`AddBatchEvent()`、`SendBatch()`、`ReceiveBatchResponse()`

### HotkeyManager.cpp

- `CHotkeyManager::UpdateHotkeys()` - 用服务下发的白名单更新
- `CHotkeyManager::IsKeyDownHotkey()` / `IsKeyUpHotkey()` - O(1) 查找
- `CHotkeyManager::IsToggleModeKeyByVK()` - 判断该键是否切换模式
- `CHotkeyManager::ClassifyInputKey()` - 按键分类（字母/数字/标点等）
- `CHotkeyManager::CalcKeyHash()` - 计算查找用 key hash
- `CHotkeyManager::NormalizeModifiers()` - 抹掉左右修饰键差异

### LangBarItemButton.cpp

- `CLangBarItemButton::GetInfo()` - 返回语言栏项信息
- `CLangBarItemButton::GetStatus()` - 返回显示状态
- `CLangBarItemButton::OnClick()` - 处理语言栏左键点击
- `CLangBarItemButton::InitMenu()` - 构建右键菜单
- `CLangBarItemButton::OnMenuSelect()` - 处理菜单项选择
- `CLangBarItemButton::GetIcon()` - 返回语言栏图标
- `CLangBarItemButton::GetText()` - 返回 tooltip 文本
- `CLangBarItemButton::UpdateLangBarButton()` - 模式变化时更新图标/文本
- `CLangBarItemButton::UpdateCapsLockState()` - CapsLock 变化时更新指示
- `_MsgWndProc()` - 跨线程更新用的消息窗口
- `PostUpdateFullStatus()` - 经 WM_UPDATE_STATUS 做线程安全的状态更新
- `PostCommitText()` - 经 WM_COMMIT_TEXT 做线程安全的上屏
- `PostClearComposition()` - 经 WM_CLEAR_COMPOSITION 做线程安全的清组合
- `PostActivationStatus()` - 经 WM_ACTIVATION_STATUS 投递激活状态（IMEActivated/FocusGained 异步化后由 AsyncReader 调用，TSF 线程 handler 触发 `CTextService::ApplyActivationStatusResponse`）

### CaretEditSession.cpp

- `CCaretEditSession::DoEditSession()` - TSF edit session 回调
- `CCaretEditSession::GetCaretAndCompositionStartRect()` - 同步取 caret + 组合起点（selection 退化时降级用组合起点当 caret）
- `CCaretEditSession::RequestCaretRectAsync()` - 异步取坐标（非按键上下文必须用它，见函数注释）

### DisplayAttributeInfo.cpp

- `CDisplayAttributeInfoInput::GetAttributeInfo()` - 返回组合文本下划线样式
- `CDisplayAttributeProvider::EnumDisplayAttributeInfo()` - 枚举可用显示属性
- `CDisplayAttributeProvider::GetDisplayAttributeInfo()` - 按 GUID 取属性
- `CEnumDisplayAttributeInfo` - 显示属性枚举器

### Register.cpp

- `RegisterServer()` - 在 HKEY_CLASSES_ROOT 注册 CLSID
- `UnregisterServer()` - 注销 CLSID
- `RegisterProfile()` - 向 TSF manager 注册输入法 profile
- `UnregisterProfile()` - 注销 profile
- `RegisterCategories()` - 注册文本服务类别（TIP、INPUTPROCESSOR 等）
- `UnregisterCategories()` - 注销类别

### FileLogger.cpp

- `CFileLogger::Instance()` - 获取单例
- `CFileLogger::Init()` - 读取配置文件，构建日志路径，打开**常开**的追加句柄
- `CFileLogger::Shutdown()` - 关闭文件句柄，清理资源
- `CFileLogger::Write()` - 写入（本进程独占自己那个文件，**无跨进程锁**）
- `CFileLogger::IsEnabled()` - 内联快速路径检查（mode=none 时零开销）
- `_ReadConfig()` - 解析 `%LOCALAPPDATA%\AuroraInput\logs\tsf_log_config`（mode/level 两个键）
- `_BuildPaths()` - 构建 logDir（`logs\`，放配置）/ fileDir（`logs\tsf_log\`，放日志）/
  logPath（`wind_tsf.<宿主名>.<pid>.log`）/ oldPath / configPath
- `_OpenLogFile()` - 建目录（父子各一次，`CreateDirectoryW` 不建中间层）+ 开
  `FILE_APPEND_DATA` 句柄，并把已有文件大小计入 `_written`
- `_ResyncFile()` - 与磁盘真实状态对表，**每秒最多一次**（节流用 `GetTickCount`，它读共享
  用户页不是系统调用，故可以每行都问）。一次 `GetFileInformationByHandleEx(FileStandardInfo)`
  同时判三件事：`NumberOfLinks==0 || DeletePending` → 文件被删，关句柄重开（连目录一起重建）；
  否则以 `EndOfFile` 校正 `_written`（覆盖外部截断）。
  **不做这件事的后果**：文件被删后 `WriteFile` 仍返回成功，日志静默进入已摘名的幽灵文件，
  目录里什么都看不到——极易误判成「功能没跑」
- `_RotateNow()` - `_written` 超过 5MB 时关句柄→改名到 `.old.log`→重开（本进程独占，无需协调）
- `_WriteToFile()` - UTF-8 写入文件
- `_WriteToDebugString()` - 调用 `OutputDebugStringW`
- `_FormatTimestamp()` - 生成 `[HH:MM:SS.mmm]` 格式时间戳

**配置文件**（`%LOCALAPPDATA%\AuroraInput\logs\tsf_log_config`）：

```
mode=none    # none | file | debugstring | all
level=debug  # off | error | warn | info | debug | trace
```

### HostWindow.cpp

- `CHostWindow::Initialize(shmName, eventName, maxBufferSize, instanceId, ipcClient, kind, ownerOverride)` - 接收共享内存/事件名 + `instanceId`（本连接 bridge clientID，存 `_instanceId`，渲染线程据此与帧的 `targetInstanceId` 比对）+ **IPCClient 弱引用**（鼠标事件经 `SendAsync` 回传服务）+ `kind`（HostWindowKind：仅候选启用鼠标交互，tooltip/状态为纯显示）+ `ownerOverride`（band 窗口 owner；tooltip/状态传候选 hwnd 使其 z-order 压在候选之上不被遮挡）；调用 `_ResolveAPIs()` 和 `_CreateBandWindow()`，启动渲染线程。`GetHwnd()` 返回 band 窗口句柄供兄弟窗口作 owner
- `CHostWindow::Uninitialize()` - 停止渲染线程，销毁窗口，解除共享内存映射
- `CHostWindow::_ResolveAPIs()` - 动态从 user32.dll 获取 `CreateWindowInBand` 和 `GetWindowBand` 函数指针
- `CHostWindow::GetHostBand()` - 获取宿主进程的 DWM Band 等级：先看前台窗口（`GetForegroundHostBand()`，前台不属本进程返回 0），否则枚举本进程顶层窗口取最大值——后者只是猜，SearchHost 开机预启动时会猜到开始菜单之下，靠 `RecheckHostBand` 纠正
- `CHostWindow` 记下建窗依据：`GetProbedHostBand()` / `GetRequestedBand()` / `HasOwner()`，供复核与落位上报
- `CHostWindow::_CreateBandWindow()` - 在指定 Band 等级创建 `WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE` 无边框窗口；建后 `SetWindowLongPtr(GWLP_USERDATA, this)` 供静态 `_WndProc` 取实例。**band 回退**：受限 UWP/AppContainer 宿主（微软商店 `WinStore.App.exe` 等）下 `CreateWindowInBand(band=1, ZBID_DESKTOP)` 返回 `ERROR_INVALID_PARAMETER(87)`；失败且 band≠0 时自动回退 `band=0`(`ZBID_DEFAULT`) 重试（这类宿主本不在高 band，`WS_EX_TOPMOST`+owner 即可保层级）
- `CHostWindow::_RenderThread()` / `_RenderLoop()` - 渲染线程：等待事件信号 → 读取 SharedRenderHeader → 跳过过期帧 → **帧定向过滤**（`!visible || header.targetInstanceId != _instanceId` → `_HideWindow` 后 continue，否则才渲染）→ `_RenderFrame()`。同进程多实例（如两个记事本窗口）共用单块 SHM、各等独立 event，服务 signal 全部实例并盖 `targetInstanceId`，仅目标实例渲染、兄弟实例自隐藏——修复"两层候选只隐一个"
- `CHostWindow::_RenderFrame()` - 先 `_UpdateHitRects()` 快照内嵌命中矩形（render 线程写、UI 线程读，`_rectLock` 守护），再将像素数据经 `UpdateLayeredWindow` 渲染到分层窗口
- `CHostWindow::_WndProc()` - 静态窗口过程，经 GWLP_USERDATA 取实例后路由鼠标（**仅 `_windowKind==HOST_WINDOW_CANDIDATE` 路由；tooltip/状态纯显示落 DefWindowProc**）：`WM_LBUTTONDOWN`→`_OnMouseDown`（记命中 + SetCapture）、`WM_LBUTTONUP`→`_OnMouseUp`（**松开仍在同一项**才 `CmdCandidateSelect` index/翻页，判据 `include/HostClickPolicy.h`；按住期间窗口被收起则放弃）、`WM_MOUSEMOVE`+`WM_MOUSELEAVE`→`_OnMouseMove`/`_OnMouseLeave`（悬停→`CmdCandidateHover` index+屏幕锚点）、`WM_MOUSEWHEEL`→`_OnMouseWheel`（原始 delta→`CmdCandidateScroll`，**不**在 DLL 翻页，交服务决策）；均经 `SendAsync` 异步发送不阻塞宿主 UI 线程。`WM_MOUSEACTIVATE`→`MA_NOACTIVATE` 对所有 kind 生效（防夺焦）
- **多 host 窗口**：`CTextService::_pHostWindow[HOST_WINDOW_KIND_COUNT]` 每 kind 一个 CHostWindow。`_EnsureHostRenderSetup` 遍历 `ServiceResponse.hostRenderSetups`（CMD_HOST_RENDER_SETUP 多条目）两趟建窗：先候选（pass 0，作 z-order owner），再 tooltip/状态（pass 1，owner=候选 hwnd）。`_EnsureHostRenderSetup`（activation 推送）整组重建；建窗部分拆在 `_CreateHostWindows`。**band 复核**：`RecheckHostBand` 在一段输入的首键（`KeyEventSink::OnKeyDown`，`!hasComposition`）按前台 band 复核，判据见 `include/HostBandPolicy.h`（前台 band>1 且与建窗时 requested 不符 / 建时无 owner / 句柄失效 → 整组重建，保持 owner 一致）。**落位上报**：每次建成经 `CMD_EXT` 信封发 `diag.host_render_placed`（JSON，recheck 带旧窗 `prev_*`），服务端写中央日志——SearchHost 是 AppContainer，写不了 tsf_log，这是看它 band 判定的唯一途径；`_DestroyHostWindow` 反序销毁（owned 先于 owner）。**失败上报**：建窗后候选仍为 null（band=0 回退也失败）或 setup 请求失败时，经 `SendAsync(CMD_HOST_RENDER_FAILED, reason u32)` 通知服务（中央日志可观测 + 按 PID 去重 toast 提示用户已回退本地窗口）
- `CHostWindow::_HitTest()` - 客户区坐标命中测试 `_hitRects`，候选(≥0)优先于翻页按钮(-1/-2)，无命中返回 `INT_MIN`
- `CHostWindow::_HideWindow()` - 隐藏窗口（候选框消失时调用），同时清空 `_hitRects` 与悬停状态防止贴到隐藏帧

### 改这里时的约定

1. **理解 edit session** - `SetText`、`SetCaret` 这类 TSF API 必须在 `RequestEditSession` 上下文里调
2. **Barrier 机制（预留，尚未激活）** - `_SendCommitRequest`/`_HandleCommitResult` 已实现但 `_SendCommitRequest` 从未被调用。服务侧的 `HandleCommitRequest` 也是死代码。接入时在 `OnKeyDown` 中检测 Space/Enter/数字键触发点，调用 `_SendCommitRequest` 并等待 `_HandleCommitResult` 回调；服务侧 `handleSpaceInternal` 已正确使用 `selectedIndex`
3. **异步读取线程** - 后台收状态推送；用回调 + 消息窗口把 UI 更新搬回安全线程
4. **引用计数** - 所有 COM 对象都要 AddRef/Release；用 `SafeRelease()` 防泄漏
5. **命名管道超时** - 连接 100ms、读写 50-100ms；熔断器 3 次失败后打开、3 秒后重置

### 通用代码式样

**发按键事件：**

```cpp
// In CKeyEventSink::OnKeyDown():
uint32_t mods = _GetModsSnapshot();
uint8_t toggles = _GetTogglesSnapshot();
uint16_t seq = _GetNextEventSeq();
_pTextService->GetIPCClient()->SendKeyEvent(wParam, scanCode, mods, KEY_EVENT_DOWN, toggles, seq);
_HandleServiceResponse();  // Check if consumed or pass through
```

**更新组合：**

```cpp
// In CTextService::UpdateComposition():
CUpdateCompositionEditSession* pEditSession = new CUpdateCompositionEditSession(...);
_pThreadMgr->RequestEditSession(_tfClientId, pEditSession, TF_ES_SYNC, NULL);
pEditSession->Release();
```

**接收异步状态推送：**

```cpp
// In CIPCClient::_AsyncReaderLoop():
ServiceResponse response;
_ReceiveBinaryMessage(header, payload);
_ParseResponse(header, payload, response);
if (_statePushCallback) {
    _statePushCallback(response);  // Call registered callback
}
```

**熔断器逻辑：**

```cpp
// In CIPCClient::_ShouldAttemptOperation():
if (_circuitState == CircuitState::Open) {
    DWORD elapsed = GetTickCount() - _lastFailureTime;
    if (elapsed >= IPCConfig::CIRCUIT_RESET_INTERVAL_MS) {
        _circuitState = CircuitState::HalfOpen;  // Try again
    } else {
        return FALSE;  // Skip operation
    }
}
```

## 构建与测试

### 构建验证

- 所有 .cpp 文件（含 `HostWindow.cpp`）必须以 `/utf-8 /W3` 编译进 `wind_tsf.dll`
- `wind_tsf.dll` 必须经 `wind_tsf.def` 导出 4 个函数
- 不得出现 pragma pack 不一致的 C5260 警告

### 按键事件测试

1. 注册 DLL 并切到极光输入法
2. 在文本编辑器里按键
3. 在 IPC 日志里确认服务收到 KEY_EVENT
4. 确认 TSF context 里出现组合
5. 按 Space → 确认候选上屏（barrier 机制尚未接入，不会发送 CmdCommitRequest）

### IPC 测试

1. 用 NamedPipeMon 监视命名管道
2. 确认二进制协议格式与 `BinaryProtocol.h` 一致
3. 测熔断器：杀掉服务 → 确认熔断打开 → 重启服务 → 确认恢复
4. 测异步读取：从服务发一条状态推送 → 确认回调触发、UI 更新

### 组合测试

1. 输入一段需要组合的编码（例如 "shng" 出「上」）
2. 确认 `UpdateComposition` 被调用、文本与光标位置正确
3. 确认显示属性（下划线）已应用
4. 按 Enter → 确认 `InsertText` 且组合结束
5. 确认上屏后光标位置正确

## 依赖

### 内部

- 每个 .cpp 都包含同名 .h
- `TextService.cpp` 包含 `KeyEventSink`、`IPCClient`、`LangBarItemButton` 等
- `dllmain.cpp` 包含 `Globals`、`TextService`、`ClassFactory`、`Register`

### 外部

- Windows SDK：kernel32、ole32、user32（源内 `pragma comment` 链接）
- MSVC Runtime：libc、libcmt（C 运行时）
- TSF 库：msctf.lib、ctfutb.lib
- DirectWrite：不再由 C++ 侧链接；服务侧通过 CGO 直接调用系统 dwrite.dll

## 全局约束

跨组件与全仓约定按需引用根 [`AGENTS.md`](../../AGENTS.md)（协议三层闸门、C++ 吃键集必须 ⊆ Rust 出字集、日志级别与隐私要求、构建脚本），本节只补本目录特有的：

- **`OnTestKeyDown` / `OnKeyDown` 的闸门顺序不能随意调**：日志导出热键、`IsKeyboardDisabled`、密码框抑制、只读上下文各自的先后都有实测依据（见 `KeyEventSink.cpp` 与父目录的 FileLogging 一节），加新闸门前先确认它该排在哪一档。
- **吃键判据要在 C++ 侧就定死**：本层的「吃 / 不吃」早于 IPC 往返，凡 C++ 吃掉而服务最终不出字的键在严格 TSF 宿主上直接丢失（历史案例：全角模式丢键、密码框丢键）。
- **不新增跨进程共享状态**：DLL 的每个宿主进程各跑一份，任何跨进程协调都必须走服务侧（见 `FileLogger.cpp` 为何改成每进程一文件）。
