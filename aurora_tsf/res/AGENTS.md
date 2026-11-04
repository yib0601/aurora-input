<!-- Parent: ../AGENTS.md -->

# res/ — TSF DLL 资源文件

> `aurora_tsf` 的图标与版本信息模板。这些文件在构建时由资源编译器（RC.exe）编进 DLL 二进制，不在运行时单独存在；`version.rc.in` 是 CMake 模板，构建时自动替换版本号变量生成 `version.rc`。

## 职责

为 `wind_tsf.dll` 提供全部 Windows 资源：

- 输入法图标（语言栏、Windows 设置、输入法选择下拉列表里显示的那一个）
- DLL 的 VERSIONINFO 文件属性（版本号由 CMake 在配置阶段注入）

## 关键文件

| 文件 | 说明 |
|------|------|
| `resource.rc` | 资源脚本，定义图标资源（index 0 = `aurora_input.ico`） |
| `version.rc.in` | CMake 版本资源模板；`configure_file` 将 `@APP_VERSION_*@` 占位符替换为实际版本号后输出到 build 目录的 `version.rc`，写入 DLL 的 VERSIONINFO 文件属性 |
| `aurora_input.ico` | 图标文件，显示在 Windows 语言栏与设置界面 |

## 硬约定

### resource.rc 的内容

```rc
// WindInput TSF Resource File
// Icon resource for input method display in Windows settings

// Icon index 0 - Main input method icon
0 ICON "aurora_input.ico"
```

用途：

- 定义单个图标资源（index 0）
- 图标显示在：
  - 极光输入法激活时的 Windows 语言栏
  - Windows 设置 > 时间和语言 > 输入法列表
  - 输入法选择下拉框

资源 ID：

- `0` - 输入法主图标

### aurora_input.ico 的规格

**格式：** 标准 Windows ICO 文件（多分辨率）

**必需尺寸：**

- 16x16（语言栏）
- 32x32（设置界面，较大的场合）
- 48x48（高 DPI 支持）
- 256x256（Windows 11 现代 UI）

**最佳实践：**

- 视觉设计清晰可辨（汉字或风车意象）
- 提供高对比度配色以照顾可访问性
- 在 96、120、144、192 DPI 各档验证任务栏上的图标可见性

### 改资源时的约定

1. **图标格式** - 用标准 ICO，现代 Windows 数据段用 PNG 压缩
2. **多分辨率** - 单个 .ico 文件里同时含 16x16、32x32、48x48、256x256
3. **高 DPI** - 在 150%（144 DPI）与 200%（192 DPI）缩放下验证
4. **透明度** - 用 alpha 通道做平滑抗锯齿与透明
5. **色深** - 推荐 32-bit RGBA，兼顾画质与透明支持

### 往 resource.rc 加资源

```rc
// String resources
STRINGTABLE
BEGIN
    IDS_APP_NAME,           L"极光输入法"
    IDS_APP_DESCRIPTION,    L"Windows TSF Chinese Input Method"
END

// Dialog resource (if needed)
IDD_ABOUT DIALOGEX 0, 0, 256, 200, WS_POPUP | WS_CAPTION
STYLE DS_SHELLFONT
CAPTION "About 极光输入法"
FONT 9, "MS Shell Dlg 2"
BEGIN
    DEFPUSHBUTTON   "OK", IDOK, 100, 180, 50, 14
    LTEXT           "Version 1.0", IDC_STATIC, 10, 10, 100, 14
END
```

### 资源编译器参数

CMakeLists.txt 里这样调用 RC.exe：

```bash
rc.exe /d UNICODE /d _UNICODE /fo resource.res resource.rc
```

**参数：**

- `/d UNICODE` - 启用 Unicode 支持
- `/d _UNICODE` - 定义 _UNICODE 宏
- `/fo` - 指定输出文件名（resource.res）

### 做高质量图标

1. **以 256x256 为主设计稿**
2. **导出多个尺寸：**
   - 256x256（主）
   - 128x128（高 DPI 回退）
   - 64x64（资源管理器视图）
   - 48x48（设置界面，中号）
   - 32x32（设置界面，小号；语言栏的双倍尺寸）
   - 16x16（语言栏实际尺寸）
3. **每个尺寸单独做抗锯齿**
4. **保留 alpha 通道的透明信息**
5. **用 ICO 格式** - 所有尺寸打进单个 .ico 文件

### version.rc.in 模板

版本信息通过 CMake `configure_file` 机制自动注入，不需要手工编辑：

```
# CMakeLists.txt 调用：
configure_file(res/version.rc.in ${CMAKE_BINARY_DIR}/version.rc @ONLY)
```

模板中的占位符（如 `@APP_VERSION_MAJOR@`、`@APP_VERSION_STR@`）由 CMake 替换为：

- `-DAPP_VERSION_MAJOR` / `MINOR` / `PATCH` / `BUILD` 参数值
- `-DAPP_VERSION_STR` 字符串（如 `"1.0.0"`）

## 构建与测试

### 验证图标显示

1. 构建 DLL：`cmake --build . --config Release`
2. 注册：`regsvr32 wind_tsf.dll`
3. 打开 Windows 设置 > 时间和语言 > 输入法
4. 确认极光输入法条目上的图标正确
5. 切换到该输入法
6. 确认语言栏按 16x16 显示图标
7. 在不同 DPI 缩放（150%、200%）下复测

### 验证资源无错

```bash
# After build, check for resource warnings
dumpbin /resources wind_tsf.dll | findstr "Icon"
```

### 验证 DLL 版本信息

```bash
wmic datafile where name="<path>\\wind_tsf.dll" get Version
# 或在 PowerShell 中：
(Get-Item "build\wind_tsf.dll").VersionInfo.FileVersion
```

## 依赖

### 内部

- `resource.rc` 通过 ICON 语句引用 `aurora_input.ico`

### 外部

- Windows Resource Compiler（RC.exe）——随 Windows SDK 提供
- 图标编辑工具（不参与构建，离机使用）：
  - GIMP
  - Paint.NET
  - Photoshop
  - Icon Editor（Visual Studio 内置）

## 全局约束

跨组件与全仓约定按需引用根 [`AGENTS.md`](../../AGENTS.md)（提交与格式化、日志规范、构建脚本），本节只补本目录特有的：

- **资源只经 `resource.rc` / `version.rc.in` 进入 DLL**：图标不做独立文件随安装包分发，DLL 里的 index 0 就是唯一入口；新增资源沿用同一份 `resource.rc`，别另起资源脚本。
- **版本号不手改**：产品版本唯一真源在构建脚本分发的 `APP_VERSION_*`（根 AGENTS.md「版本 / 发布」），模板里只有占位符。
