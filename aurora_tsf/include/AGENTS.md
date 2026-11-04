<!-- Parent: ../AGENTS.md -->

# include/ — TSF DLL 头文件

> `wind_tsf.dll` 的公开与内部头文件：接口、结构体、协议定义的唯一声明处，所有头文件用 `#pragma once` 守卫、按 C++17/MSVC 编译。实现一律在 `../src/`。

## 职责

定义 TSF DLL 与外部世界（Windows TSF 宿主、Rust `aurora_input` 服务）之间的全部契约：

- TSF COM 接口实现类的类声明（`CTextService` / `CKeyEventSink` / `CIPCClient` / `CLangBarItemButton` 等）
- 与 Rust 服务共享的二进制协议结构、命令码、状态标志位
- 全局状态（`HINSTANCE`、DLL 引用计数、CLSID/GUID、管道名）
- 可复用的策略头（`HostBandPolicy.h` / `HostClickPolicy.h` / `WindowTitlePolicy.h` 等，纯判据，便于单测）

## 关键文件

| 文件 | 说明 |
|------|------|
| `Globals.h` | 日志宏、COM 工具、全局状态（HINSTANCE、DLL 引用计数、GUID、管道名） |
| `BinaryProtocol.h` | 二进制协议定义（5KB+ 的结构体、枚举、命令码）；与 Rust 服务共享 |
| `TextService.h` | `CTextService` 类（主 TSF 文本输入处理器、组合管理、IPC 协调） |
| `KeyEventSink.h` | `CKeyEventSink` 类（键盘事件捕获、修饰键状态机、barrier 机制） |
| `IPCClient.h` | `CIPCClient` 类（命名管道客户端、二进制协议、熔断器、异步读取线程） |
| `ClassFactory.h` | `CClassFactory` 类（用于实例化 `CTextService` 的 COM 类工厂） |
| `HotkeyManager.h` | `CHotkeyManager` 类（热键白名单、按键分类、修饰键跟踪） |
| `LangBarItemButton.h` | `CLangBarItemButton` 类（语言栏 UI、菜单、线程安全的异步更新） |
| `CaretEditSession.h` | `CCaretEditSession` 类（取光标位置的 TSF edit session） |
| `DisplayAttributeInfo.h` | `CDisplayAttributeInfoInput`、`CDisplayAttributeProvider`（组合文本样式） |
| `Register.h` | `RegisterServer`、`UnregisterServer` 函数（Windows 注册表集成） |
| `FileLogger.h` | `CFileLogger` 类（运行时可配置文件日志，单例，支持 none/file/debugstring/all 四种输出模式，5MB 自动轮转，多进程安全） |
| `HostWindow.h` | `CHostWindow` 类（开始菜单宿主进程代理渲染窗口，通过 CreateWindowInBand 解决 Win11 z-order 问题，使用共享内存接收 Rust 侧渲染帧 + 内嵌命中矩形；`_WndProc` 仅候选 kind 路由鼠标点选/悬停/滚轮经 SendAsync 回传服务。`Initialize(shmName, eventName, maxBufferSize, instanceId, ipcClient, kind, ownerOverride)`：`instanceId`=本连接 bridge clientID（存 `_instanceId`，渲染线程仅当帧的 `targetInstanceId==_instanceId` 才渲染否则隐藏，解决同进程多实例共用单 SHM 时"两层候选只隐一个"），`kind` 选窗口角色（候选含交互 / tooltip·状态纯显示），`ownerOverride` 让 tooltip/状态 owned 于候选 hwnd 保证 z-order 在上；`GetHwnd()` 暴露句柄供作 owner。每 kind 一实例，由 `CTextService::_pHostWindow[HOST_WINDOW_KIND_COUNT]` 持有） |


## 硬约定

### 头文件依赖图

```
TextService.h
├── KeyEventSink.h
├── IPCClient.h
├── LangBarItemButton.h
├── CaretEditSession.h
├── HotkeyManager.h
├── HostWindow.h
├── Globals.h
└── BinaryProtocol.h

HostWindow.h
├── Globals.h
└── BinaryProtocol.h

KeyEventSink.h
├── IPCClient.h
├── BinaryProtocol.h
└── Globals.h

IPCClient.h
├── BinaryProtocol.h
└── Globals.h

LangBarItemButton.h
└── Globals.h

HotkeyManager.h
├── BinaryProtocol.h
└── Globals.h

FileLogger.h
└── (standalone, no internal dependencies)
```

### BinaryProtocol.h（v1.1）的关键结构

**协议头（8 字节）：**

```cpp
struct IpcHeader {
    uint16_t version;   // Protocol version (PROTOCOL_VERSION = 0x1001 for v1.1)
    uint16_t command;   // Command type (CMD_KEY_EVENT, CMD_COMMIT_REQUEST, etc.)
    uint32_t length;    // Payload length in bytes
};
```

**异步标志：** version 字段的最高位（0x8000）标记异步请求（不等响应）

**按键事件载荷（16 字节）：**

```cpp
struct KeyPayload {
    uint32_t keyCode;     // Virtual key code
    uint32_t scanCode;    // Scan code
    uint32_t modifiers;   // Modifier flags (KEYMOD_SHIFT, KEYMOD_CTRL, KEYMOD_LSHIFT, KEYMOD_RSHIFT, etc.)
    uint8_t  eventType;   // 0=KeyDown, 1=KeyUp
    uint8_t  toggles;     // Toggle state (TOGGLE_CAPSLOCK, TOGGLE_NUMLOCK, TOGGLE_SCROLLLOCK)
    uint16_t eventSeq;    // Monotonic event sequence (for ordering)
};
```

**Barrier 机制（用于上屏请求）：**

```cpp
struct CommitBarrier {
    uint16_t barrierSeq;        // Sequence to match with response
    uint16_t triggerKey;        // Triggering key (Space/Enter/number)
    std::string inputBuffer;    // Current input buffer state
};
```

**状态标志位：**

```cpp
STATUS_CHINESE_MODE     = 0x0001  // Chinese/English mode
STATUS_FULL_WIDTH       = 0x0002  // Full-width/half-width
STATUS_CHINESE_PUNCT    = 0x0004  // Chinese/English punctuation
STATUS_TOOLBAR_VISIBLE  = 0x0008  // Toolbar visibility
STATUS_MODE_CHANGED     = 0x0010  // Mode was just changed (transient)
STATUS_CAPS_LOCK        = 0x0020  // CapsLock state
```

### 全局状态

**Globals.h 声明：**

```cpp
extern HINSTANCE g_hInstance;           // DLL instance handle
extern LONG g_lServerLock;              // Server lock count (COM)
extern const CLSID c_clsidTextService;  // CLSID for COM factory
extern const GUID c_guidProfile;        // Profile GUID
extern const GUID c_guidLangBarItemButton; // Language bar item GUID
```

**命名管道：**

```cpp
PIPE_NAME      = L"\\\\.\\pipe\\aurora_input"       // Main IPC pipe
PUSH_PIPE_NAME = L"\\\\.\\pipe\\aurora_input_push"  // Async push pipe
```

### 改头文件时的约定

1. **头文件保持最小** - 实现移到 `src/` 的 .cpp 里
2. **所有公开 API 写内联注释**，说明参数与返回值
3. **维持二进制协议兼容** - 不与服务端协调就不得改结构体尺寸
4. **网络协议结构体一律 `pack(1)`**（所有 `IpcHeader`、`*Payload`、`*Header`）
5. **枚举用 `enum class`**（类型安全），除非二进制兼容要求裸枚举
6. **不用 iostream** - 用 `AURORA_LOG_*` 宏

### FileLogger 的用法

`FileLogger.h` 是零内部依赖的独立头，可在任意 .cpp 里直接用：

```cpp
#include "FileLogger.h"

// Fast-path check (inlined, zero overhead when mode=none)
if (CFileLogger::Instance().IsEnabled(CFileLogger::LogLevel::Debug)) {
    CFileLogger::Instance().Write(CFileLogger::LogLevel::Debug, L"Key event received");
}
```

运行时开启日志：建 `%LOCALAPPDATA%\AuroraInput\logs\tsf_log_config`：

```
mode=file
level=debug
```

### 头文件里的日志

头里尽量只放声明，实现放 .cpp；用前向声明避免循环依赖：

```cpp
// Good: forward declare and use in method signature
class CTextService;
class CIPCClient;

// Bad: include the full header (circular dependency risk)
#include "TextService.h"
```

### 协议演进

改 `BinaryProtocol.h` 时：

1. 破坏性变更**要递增 `PROTOCOL_VERSION`**
2. **新命令码加在既有命令码之前**（例：批量用 `0x0F01`）
3. **载荷只允许在尾部追加字段**（向后兼容）
4. **格式变更要写注释标出字节偏移**
5. **同步更新服务端**，过渡期要同时认新旧两个版本

### 通用代码式样

**安全释放接口：**

```cpp
// Prefer SafeRelease() template from Globals.h
ITfThreadMgr* pTm = ...;
SafeRelease(pTm);  // Calls Release() and sets to nullptr
```

**安全删除内存：**

```cpp
// Prefer SafeDelete() template from Globals.h
CKeyEventSink* pSink = new CKeyEventSink();
// ... use it ...
SafeDelete(pSink);  // Calls delete and sets to nullptr
```

**分级日志：**

```cpp
AURORA_LOG_ERROR_FMT(L"Failed to connect: %d", errorCode);
AURORA_LOG_INFO(L"Key event received");
AURORA_LOG_DEBUG_FMT(L"Modifiers: 0x%04X", modifiers);
```

## 依赖

### 内部

- `Globals.h` 被其余头文件普遍包含（日志、COM 工具）
- `BinaryProtocol.h` 被协议密集的头文件包含（`IPCClient.h`、`KeyEventSink.h`）

### 外部

- `<windows.h>` - Windows SDK 基础
- `<msctf.h>` - TSF 接口（`ITfTextInputProcessor`、`ITfContext` 等）
- `<ctfutb.h>` - 语言栏接口（`ITfLangBarItemButton`）
- `<string>` - `std::wstring`、`std::string`
- `<vector>` - 动态数组
- `<unordered_set>` - O(1) 热键查找

## 全局约束

跨组件与全仓约定按需引用根 [`AGENTS.md`](../../AGENTS.md)（尤其「跨组件硬约定」里的协议三层闸门与 C++ 吃键集 ⊆ Rust 出字集），本节只补本目录特有的：

- **本目录只放声明与纯判据**：任何有状态、会调度、会阻塞的实现都不许进头文件；策略头保持无副作用，才能被单测直接调。
- **协议真源在 Rust 侧**（`aurora_core/crates/aurora-ipc`），`BinaryProtocol.h` 是镜像。改结构体尺寸后必须让 `testdata/wire_vectors.json` 那套真值表重新对账，别只跑 C++ 编译。
