# Windows 侧冒烟测试

在**真实的 Windows 加载器**下验证 `aurora_tsf_win.dll`。

三个可跑的程序：

| 程序 | 验证什么 |
|---|---|
| `dll_smoke.rs` | DLL 加载、COM 实例化、引用计数 |
| `ipc_mock_server.rs` | 模拟 Rust 服务端（Named Pipe 服务） |
| `ipc_roundtrip.rs` | 从 DLL 调传输层，与服务端收发一帧 |

## 为什么不在 `cargo test` 里

这些需要**能加载 PE 的环境**。Linux 上 `cargo test` 跑不了——
编出来的二进制要在 Windows 加载器下执行才行。所以本目录是独立的小工程
（有自己的 `Cargo.toml` 与 `[workspace]`），手工构建运行。

## 环境：Wine 即可，不必真 Windows

Wine 11.20 的 `msctf.dll` 有完整实现（`TF_CreateThreadMgr` 等全部导出），
能测到 COM 激活与引用计数这一层。**不能替代真机**的地方见文末。

```bash
sudo dnf install wine-core wine-common        # Fedora
# 或 apt install wine64

export WINEPREFIX=~/.wine-tsf WINEDEBUG=-all
wineboot --init
```

## 跑法

```bash
# 1) 构建被测 DLL（+crt-static 必需：否则目标机缺 VC++ 运行库加载失败）
cd ../../..
RUSTFLAGS="-C target-feature=+crt-static" \
  cargo xwin build -p aurora-tsf-win --target x86_64-pc-windows-msvc --release

# 2) 构建测试程序
cd crates/aurora-tsf-win/tests/win
cargo xwin build --bins --target x86_64-pc-windows-msvc --release

# 3) 部署
T=target/x86_64-pc-windows-msvc/release
mkdir -p $WINEPREFIX/drive_c/{dll,test}
cp ../../../target/x86_64-pc-windows-msvc/release/aurora_tsf_win.dll $WINEPREFIX/drive_c/dll/
cp $T/{dlltest,ipcclient,ipctest}.exe $WINEPREFIX/drive_c/test/
```

### 冒烟测试（DLL 加载 + COM）

```bash
cd $WINEPREFIX/drive_c/test
wine dlltest.exe 'C:\dll\aurora_tsf_win.dll'
```

### IPC 端到端（两个终端）

```bash
# 终端 A：模拟服务端
wine ipctest.exe '\\.\pipe\aurora_input_test'

# 终端 B：客户端（DLL 里的传输层）
wine ipcclient.exe 'C:\dll\aurora_tsf_win.dll' '\\.\pipe\aurora_input_test'
```

## 已验证的结果（2026-11-01，Wine 11.20）

### DLL 冒烟

```
[1] LoadLibrary 成功
[2] 四个导出全部可见
[3] DllGetClassObject 成功
[4] CreateInstance 成功
[5] QueryInterface(ITfTextInputProcessor) 成功
[6] ThreadMgr 就绪 (TfClientId=1)
[6b] TextService::Activate(真实 TSM) 成功
[7] Deactivate 成功
[8] 有活对象时 DllCanUnloadNow = S_FALSE  ✓
[9] 全部释放后 = S_OK                      ✓ 无引用泄漏
[10] DllRegisterServer = E_UNEXPECTED        (Wine 缺 TF_InputProcessorProfiles)
```

### IPC 端到端

```
客户端: [2] 管道名计算 \\.\pipe\aurora_input_dev_S-1-5-21-999 ✓
        [4] roundtrip 成功 ✓
服务端: [3] 客户端已连接
        [4] 收到 8 字节
        [5] 头部解析成功: cmd=0x0001 len=0    ← CMD_ACK，用 aurora-ipc 解
        [6] ACK 已回
```

## Wine 的已知缺口

| 项 | 状态 | 影响 |
|---|---|---|
| `ITfKeyEventSink` 挂载 | **不支持**（`Unhandled Sink: {aa80e7f5}`） | 按键通路无法在 Wine 下测 |
| `ITfTextEditSink` 挂载 | **不支持**（`Unhandled Sink: {8127d409}`） | 光标读取无法在 Wine 下测 |
| `CLSID_TF_InputProcessorProfiles` | 缺失 | `DllRegisterServer` 返回 `E_UNEXPECTED` |
| `ITfContextComposition` | **不支持**（`E_NOINTERFACE`） | **组合无法在 Wine 下建起来** |

### 组合的验证边界（实测记录）

Wine 能建真实的 `ITfThreadMgr` / `ITfDocumentMgr` / `ITfContext`
（`CreateContext` 成功，`ec=3`），但 **context 上 QI `ITfContextComposition`
返回 `E_NOINTERFACE`**，`StartComposition` 这条路在 Wine 下走不通。

试过的替代路径也失败：`ITfContextOwnerCompositionServices` 虽然 QI 成功，
但它的接口声明里**没有继承** `ITfContextComposition`
（windows crate 的 `interface_hierarchy!` 显示它只继承 `IUnknown`，
二次 cast 同样返回 `E_NOINTERFACE`）。

⇒ **组合的真实行为只能在真 Windows 上验证**。本仓库的
`policy::composition` 状态机用 20 个单测钉住了判据，但
「`StartComposition` 真的能建起来」这一条**未经实测**。

第一条促成了一个真实修复：原本「按键 sink 挂不上就回滚全部并让 Activate 失败」，
实测证明这是错的——输入法仍能激活、能收焦点事件，中断整个 Activate 比
「按键不通」更糟。改为只记 WARN。

## ⚠ 这块测试不能替代真机

证明了的：PE 格式、导入表、CRT 静态链接、COM 工厂与对象生命周期、
`ITfTextInputProcessor` 的 vtable 布局、Named Pipe 收发。

**没证明的**：真实宿主里的 COM 激活行为。仓库 `AGENTS.md` 记录的 `6dbc8595`
就是「交叉编译的 DLL 在加固宿主（企业微信 / TIM / UU 浏览器）里 COM 激活失败」
—— 那类问题只有真机宿主能暴露。Wine ≠ 真 Windows。
