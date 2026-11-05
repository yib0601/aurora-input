# 贡献指南

感谢您对极光输入法 (WindInput) 项目的关注！我们欢迎所有形式的贡献，包括 Bug 报告、功能建议和代码提交。

## 签署 CLA（必须）

**所有贡献者在首次提交 Pull Request 前必须签署贡献者许可协议 (CLA)。**

这是为了确保项目的许可证管理和知识产权的一致性。流程如下：

1. 提交您的 Pull Request
2. CLA Assistant 机器人会自动在 PR 中发起签署请求
3. 在 PR 评论中回复：`I have read the CLA Document and I hereby sign the CLA`
4. 签署完成后，CLA 检查将自动通过

未签署 CLA 的 PR 将无法合并。完整协议内容请参阅 [CLA.md](CLA.md)。

## Bug 报告

请通过 [GitHub Issues](../../issues) 的 **Bug 反馈** 模板提交，并尽量包含以下信息：

- 操作系统与版本（如 Windows 11 24H2，或 macOS 15）
- 出现问题的应用程序名称
- 重现步骤、预期行为与实际行为
- 相关日志文件
  - Windows 安装版：`%LOCALAPPDATA%\AuroraInput\logs\`
  - Windows 便携版：`<程序目录>\localdata\logs\`
  - macOS：`~/Library/Application Support/WindInput/logs/`

## 功能建议

欢迎通过 [GitHub Issues](../../issues) 的 **功能建议** 模板提交。请描述您希望实现的功能、使用场景，如有参考实现请提供链接。

## 代码贡献

### 开发环境

通用要求：

- Rust stable 工具链（rustup 安装，含 `rustfmt` 与 `clippy` 组件）

Windows 额外需要：

- Visual Studio 2022（安装时勾选「使用 C++ 的桌面开发」组件，用于 MSVC 与 CMake）
- PowerShell 7+

macOS 额外需要：

- macOS 12+ 与 Xcode 15+（Swift 5.9 工具链）

### 构建

| 平台 | 命令 | 说明 |
|------|------|------|
| Windows | `.\scripts\dev.ps1` | 交互式菜单；`.\scripts\dev.ps1 1` 为 Release 全构建 |
| Windows | `.\scripts\dev.ps1 l` | clippy 检查（本地不跑完整门禁，见下方「CI」） |
| macOS | `scripts/mac/dev.sh` | macOS 构建/调试菜单 |
| Linux | `scripts/dev.sh` | 交叉编译 Windows 产物（cargo-xwin） |

构建脚本会自动下载第三方词库数据（缓存于 `.cache/`）并生成完整数据目录。

> 配套的设置程序（[wind-setting](https://github.com/huanfeng/wind-setting)）、
> 便携启动器（[wind-portable](https://github.com/huanfeng/wind-portable)）与安装器
> （[wind-installer](https://github.com/huanfeng/wind-installer)）为独立仓库，
> 构建脚本在其不存在时会自动跳过，不影响核心部分的构建。

### 提交规范

本项目使用 [Conventional Commits](https://www.conventionalcommits.org/zh-hans/) 规范：

```
<类型>(<范围>): <描述>

[可选的正文]
```

类型包括：

| 类型 | 说明 |
|------|------|
| `feat` | 新功能 |
| `fix` | Bug 修复 |
| `docs` | 文档变更 |
| `refactor` | 代码重构（不改变行为） |
| `perf` | 性能优化 |
| `test` | 测试相关 |
| `chore` | 构建/工具变更 |
| `style` | 格式化（如 `cargo fmt`） |

范围示例：`engine`、`coordinator`、`dict`、`config`、`ui`、`theme`、`rpc`、`tsf`、`macos`、`build`

#### 多会话共仓时的提交纪律

本仓常有多个会话/贡献者并行改动，因此：

- **只用显式路径**：`git add <具体文件>`；**禁止 `git add -A` / `git add .` / `git commit -a`**
  ——那会把别人未提交的文件一起卷入。提交前 `git status` 确认暂存区只含自己改的文件。
- **不带 AI 元信息**：不加 `Co-Authored-By`、`Generated with`，也不加 `Constraint:` /
  `Confidence:` / `Tested:` 等 trailer。
- **不带个人信息**：提交信息与入库的注释/文档不写真实姓名、邮箱、内网地址与主机名，以及含
  用户名或本机目录布局的路径（如 `/home/<用户名>/…`、`C:\Users\<用户名>\…`）。引用文件写
  仓库相对路径。
- **逻辑修改与格式化分开提交**：`cargo fmt` 的结果单独一个 `style(fmt): …` commit，不混进
  逻辑改动。开始新一轮工作前先 `git status` + `cargo fmt`，确认没有上一轮遗留的纯格式漂移。
- `cargo fmt` 对整个 workspace 生效，被连带格式化的其他 crate 一并纳入同一个 fmt 提交。

### 代码风格

- **Rust**: 必须使用 `cargo fmt` 格式化（CI 会检查）；逻辑修改与 fmt 修改分开提交
- **C++**: 遵循项目现有代码风格（参考 `aurora_tsf/src/`）
- **Swift**: 遵循项目 macOS 端现有风格（参考 `aurora_macos/Sources/`）

### Pull Request 流程

1. Fork 本仓库并从 `main` 分支创建您的分支
2. 完成修改后本地按改动范围验证：`cargo test -p <crate>`（必要时 `dev.sh l` / `dev.ps1 l`）。
   完整门禁（fmt --check + clippy -D warnings + 全 workspace 测试）由 GitHub Actions
   在 PR 上执行，本地不再重复跑
3. 修改了某 crate 的对外接口/导出常量/文件结构时，同步更新对应的 `AGENTS.md`
4. 按 PR 模板填写变更说明、测试情况与检查清单
5. 提交 PR 并等待 CLA 检查和代码审查
6. 根据审查意见修改后，PR 将被合并

## 项目结构

| 目录 | 说明 |
|------|------|
| `aurora_core/` | Rust 核心服务（cargo workspace，33 个 crate + 4 个 app） |
| `aurora_tsf/` | Windows TSF 接口层（C++/CMake） |
| `aurora_linux/` | Linux 前端（Fcitx5 addon + X11 候选窗，C++/CMake） |
| `aurora_macos/` | macOS IMKit 输入法客户端（Swift/SwiftPM） |
| `data/` | 入库的方案、码表、主题等数据源文件 |
| `scripts/` | 构建与部署脚本 |

各 crate 的职责索引与协作约定见根目录 [AGENTS.md](AGENTS.md)。

## 许可证

本项目采用 [GPL-3.0-or-later](LICENSE) 许可证。提交贡献即表示您同意按照 [CLA](CLA.md) 的条款授权您的贡献。词库相关的第三方资源许可证请参阅 [NOTICE.md](NOTICE.md)。

此外，若您将本项目分支后公开发布，请更换项目名称与 logo 并注明为非官方分支。
该项不影响您对代码的任何权利，详见 [项目名称与标识使用约定](BRANDING.md)。
