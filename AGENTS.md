# AGENTS.md — WindInput 协作约定

> 给在本仓工作的 AI/人类协作者。Rust 核心在 `aurora_core/`（cargo workspace）。

## 仓库地图

| 位置 | 内容 | 文档 |
|---|---|---|
| `aurora_core/` | Rust 核心（cargo workspace：33 个 crate + `apps/{service,setting,repl,aurora-tools}`） | 本文档 + crate 级 `AGENTS.md` |
| `aurora_tsf/` | C++17 TSF DLL：Windows 输入法接口层，经 Named Pipe 与 Rust 服务通信 | [AGENTS.md](aurora_tsf/AGENTS.md) |
| `aurora_linux/` | Linux 前端：Fcitx5 addon + X11 候选窗/浮层 + GNOME Shell 扩展 | [AGENTS.md](aurora_linux/AGENTS.md) |
| `aurora_macos/` | macOS IMKit `.app`（与 `aurora_tsf/` 对位，开发中） | [AGENTS.md](aurora_macos/AGENTS.md) |
| `aurora_core/apps/setting` | 设置 UI（workspace 成员，经 JSON-RPC 与核心通信）；改设置界面去这里 | — |
| `data/`、`config/`、`assets/` | 随发行分发的出厂数据：词库、默认配置、图标 | `assets/README.md` |
| `scripts/` | 构建 / 部署 / 发版 / 性能探针脚本 | `CONTRIBUTING.md` |
| `.scratch/` | 本地 issue 与 spec 追踪（见文末「Agent skills」），不入库 | — |

**平级仓**（相对本仓根）：`../aurora-ui-rust` 已 checkout —— `auroraui` 已 vendored 进
`aurora_core/crates/auroraui`（`apps/setting` 的 path 依赖指向仓内那份），上游仓只用于查提交
历史与 CHANGELOG。`../wind-portable`（绿色版启动器）、`../WindInput-Go`（Go 旧版源码，只读参考）
**当前未 checkout**，构建脚本会自动跳过。

## Crate 与 app 索引

> workspace 共 33 个 crate + 4 个 app。复杂 crate 已配 crate 级 `AGENTS.md`，改动前先读对应文档；
> 新增/重构 crate 时参照同结构补文档。

| Crate | 职责 | crate 文档 |
|---|---|---|
| `aurora-coordinator` | 输入法“大脑”：按键路由、状态机、候选与模式切换的中央协调器 | [AGENTS.md](aurora_core/crates/aurora-coordinator/AGENTS.md) |
| `aurora-engine` | Schema 驱动的引擎工厂：拼音/码表/混输/英文四类引擎的构建、切换与候选分发 | [AGENTS.md](aurora_core/crates/aurora-engine/AGENTS.md) |
| `aurora-ui` | 所有浮层窗口（候选窗/工具栏/菜单/状态泡/Toast/Tooltip）的渲染与鼠标交互 | [AGENTS.md](aurora_core/crates/aurora-ui/AGENTS.md) |
| `aurora-ui-types` | 表现层协议（纯数据）：`UiCommand`/`UiEvent` 及载荷，协调器 ↔ 任意前端（桌面/macOS/Android）共用 | [AGENTS.md](aurora_core/crates/aurora-ui-types/AGENTS.md) |
| `aurora-cmdbar` | 命令直通车：短语解析 → AST 求值 → 动作执行（纯逻辑） | [AGENTS.md](aurora_core/crates/aurora-cmdbar/AGENTS.md) |
| `aurora-dict` | 多层复合词典引擎：DictLayer/CompositeDict 查询 + wdat mmap 二进制词库 | [AGENTS.md](aurora_core/crates/aurora-dict/AGENTS.md) |
| `aurora-store` | 用户数据门面：用户词与学到的数据（临时词 / 草稿 / 词频 / 联想历史）委托 `aurora-userdata`，真源都是 `<userdata>/data/<schema>/` 下的文本；短语 / Shadow / 覆盖表 / 补全 / 统计也在 `data/`（P5：复制用户目录即带走全部数据）；redb 只供迁移读；三个 Store 层也在这里 | [AGENTS.md](aurora_core/crates/aurora-store/AGENTS.md) |
| `aurora-userdata` | 用户数据引擎（快照 + 按设备日志 + 增量表）+ 运行时领域 API `Runtime`；经 `aurora-store` 门面接入（P1b） | [AGENTS.md](aurora_core/crates/aurora-userdata/AGENTS.md) |
| `aurora-wdict` | wdict（`.wdict.yaml`）交换格式读写与词条文本转义；原 `aurora-store::wdict`，`aurora-store` 原路径 re-export | [AGENTS.md](aurora_core/crates/aurora-wdict/AGENTS.md) |
| `aurora-rpc` | core ↔ 设置端 JSON-RPC IPC 双通道（ctrl 请求-响应 + events 广播） | [AGENTS.md](aurora_core/crates/aurora-rpc/AGENTS.md) |
| `aurora-config` | 配置系统：TOML 三层合并、字段注册表 SSOT、热键编译、变体探测、运行时状态 | [AGENTS.md](aurora_core/crates/aurora-config/AGENTS.md) |
| `aurora-theme` | 加载并求值 v3 主题，输出调色板 + 盒模型树供 aurora-ui 渲染 | [AGENTS.md](aurora_core/crates/aurora-theme/AGENTS.md) |
| `aurora-bridge` | Named Pipe 服务器 + Push 管道，桥接 Rust 服务与 C++ TSF DLL | [AGENTS.md](aurora_core/crates/aurora-bridge/AGENTS.md) |
| `aurora-ipc` | IPC 协议定义与编解码（TSF 二进制协议 + JSON-RPC） | — |
| `aurora-keys` | 键名/VK 映射、导航键分类（纯逻辑）+ 按键注入（平台层）；**VK 常量 SSOT** | — |
| `aurora-candidate` | 候选词数据类型、排序与过滤 | — |
| `aurora-phrase` | 短语系统：静态/动态模板展开 + cmdbar 双路径 | — |
| `aurora-transfer` | 导入导出/备份还原底座：Bundle（manifest + zip）聚合打包与 Merge 合并策略（编解码在 aurora-store）；方案包 `.wpkg` 与主题包 `.wtheme` 的格式定义都在这里 | — |
| `aurora-quick-input` | 快捷输入的内置候选来源（纯逻辑）：`quick_input.calc` 算式（含幂 `^`）/ `.date` 日期年月 / `.number` 数字金额；另定义 `.repeat`（重复上屏，由协调器实现）的成员 id。开关与优先级 = `mix_modes.members` 的有无与顺序 | — |
| `aurora-reverse` | 候选反查：五笔编码/拆字/拼音读音（悬停 tooltip） | — |
| `aurora-aux-code` | 辅助码过滤：拼音后追加字形辅助码，按字形裁减候选字词（**出厂关闭**，`schema.pinyin.aux_code.enabled`） | [AGENTS.md](aurora_core/crates/aurora-aux-code/AGENTS.md) |
| `aurora-punct` | 标点转换纯逻辑（中英标点/全半角/数字后智能） | — |
| `aurora-transform` | 文本变换：标点、全角、自动配对、简繁 | — |
| `aurora-assoc` | 联想候选的生成与合并（纯逻辑，可原生测试） | — |
| `aurora-softkeyboard` | 软键盘映射表：开启后接管主键区，按当前面的映射转换后上屏 | — |
| `aurora-host` | 核心 ↔ 宿主的接缝（纯数据 + trait） | — |
| `aurora-ports` | 端口定义：跨 crate 的 trait，只有声明没有实现（避免适配器反向依赖消费方） | — |
| `aurora-ids` | 跨层共享的标识符常量 | — |
| `aurora-utils` | 全仓共用的基础工具，**无业务语义**（文本读取规整、字节/字符串小处理） | — |
| `aurora-mobile` | 移动端形态的核心门面（headless Coordinator 的收口） | — |
| `aurora-tsf-win` | Windows TSF DLL 的 Rust 实现，逐模块替换 `aurora_tsf/` 的 C++ 版（两者不是并存关系，替换完即删对应 C++ 实现） | — |
| `aurora-webdata` | Web 设置数据 RPC：schema/dict/temp/freq/shadow/stats/theme/phrase/quick/commonChars 命名空间，方法名与前端 `contract.ts` 1:1 | [AGENTS.md](aurora_core/crates/aurora-webdata/AGENTS.md) |
| `auroraui` | 轻量跨平台桌面 GUI 框架（Windows：Win32+DirectWrite；macOS：Cocoa+CoreText，开发中）；上游 `aurora-ui-rust` 的 vendored 副本 | [AGENTS.md](aurora_core/crates/auroraui/AGENTS.md) |

| App | 职责 |
|---|---|
| `apps/service` | 主服务进程 `aurora_input`：起 Named Pipe 服务器，接收 TSF DLL 事件 |
| `apps/setting` | 设置 UI（见上表） |
| `apps/repl` | 候选 REPL：无需 TSF/UI，平台无关，可在 Linux 跑，用于快速验证 engine/dict/store |
| `apps/aurora-tools` | 代码生成工具集：`gen_protocol`（协议码对账）、`gen_pinyin`、`gen_unigram`、`gen_aux_code`、`gen_opencc`、`gen_emoji_*`、`gen_block_charsets` |

`—` = 暂无独立 `AGENTS.md`（多为纯逻辑/工具 crate，职责单一，看 `src/lib.rs` 顶部模块注释即可）。

## 平台分层策略——平台/系统依赖放哪

Android 复用（进程内直调 headless coordinator）落地后，「平台代码放哪」是立约级问题。
新写任何 `cfg(windows)` / `cfg(target_os = …)` / 平台 crate 依赖之前，按下表定归属：

| 你要加的东西 | 归属 | 先例 |
|---|---|---|
| 协调器 ↔ 前端之间流转的**数据类型** | `aurora-ui-types`（三条红线见其 AGENTS.md：仅纯数据、依赖过 android check、平台载荷走 target-specific + cfg 变体） | `UiCommand`；唯一平台载荷 `SetHostRender` |
| 引擎/词库/候选/配置等**纯逻辑** | 对应纯逻辑 crate，**零平台依赖**（引入前先问「Android 上编得过吗」） | aurora-engine / aurora-dict / aurora-candidate 等全员 |
| 协调器逻辑中途要**同步 pull** 的平台能力，且 cfg 兜底值是语义错误 | `HostServices` trait 注入面（coordinator/src/host_services.rs，收录判据三条见模块文档） | 剪贴板三方法 |
| 平台状态有**事件推送通路**可注入 | 用注入路，**不要**造第二个 pull 真相源 | 宿主进程名走焦点事件喂 `pid_names` |
| cfg 兜底值在缺失平台**可接受**的探测函数 | 同名函数三平台并列 cfg + 兜底，不进 trait | `theme_style.rs::system_prefers_dark`（三实现范例）、`aurora_keys::foreground::foreground_fullscreen_kind` |
| **整模块**平台专属的功能 | `lib.rs` 处整模块 cfg（聚类，勿散落函数内） | `direct_switch`（windows）、`handle_cmdbar_macos`、aurora-bridge 的 `*_unix`/`*_windows` 模块族 |
| **桌面渲染路径**（窗口/UI 线程/渲染注入） | aurora-coordinator 的 `desktop-ui` feature（default 含）；`--no-default-features` 即 headless/Android 形态 | 生产构造器 `new`、`UiManager` |

编译门与防走样（命令即 `aurora_core/.cargo/config.toml` 的 alias，CI 强制）：
`cargo check-headless`（host 无 aurora-ui 形态）、`cargo check-android`（依赖闭包过
aarch64-linux-android；⚠ zstd-sys 等 C 依赖的 build script 需 NDK clang，本机无 NDK 时由
CI 承接）、CI 另有 `cargo tree` 断言 coordinator 无 aurora-ui 依赖边。

已知 Android target 坑：bionic 无 POSIX SHM（`shared_memory_posix` 已 cfg 排除）；
「check 不链接故无需 NDK」对 `-sys` crate **不成立**（build script 要编 C 源）。

## 虚拟键码（VK）—— 用常量，禁止裸十六进制

所有 Windows 虚拟键码统一在 `aurora_core/crates/aurora-keys/src/keymap.rs` 定义为
`pub const VK_*`（`VK_ESCAPE`/`VK_BACK`/`VK_SPACE`/`VK_RETURN`/`VK_PRIOR`/`VK_NEXT`/
`VK_UP`/`VK_DOWN`/`VK_A..VK_Z`/`VK_0..VK_9`/`VK_SEMICOLON` 等）。

- **禁止**在 `match data.key_code` / 比较中写裸 `0x1B`、`0x21` 之类字面量；用 `keymap::VK_*`。
- 触发键名（配置里的 `"backslash"`/`"semicolon"`）→ VK：`keymap::key_name_to_vk(_with_letters)`，
  单一真相源 `KEY_TABLE`，新增键只改一处。
- **⚠️ 触发键名一致性（设置清单 ↔ 本表）**：设置界面（`aurora_core/apps/setting`，见上方仓库
  列表）的 `src/assets/settings_manifest.toml` 里各触发键选项的 `value` 必须与本表
  `KEY_TABLE.names` 字符串**逐字相同**——两边无编译期/运行期校验，写错会静默失效（UI 显示
  "已选中"、保存不报错，内核 `key_name_to_vk` 返回 `None` 后被 `filter_map` 悄悄丢弃）。
  曾因设置端把方括号选项写成 `open_bracket`/`close_bracket`（本表实际是 `lbracket`/`rbracket`）
  导致临时英文/临时拼音/快捷输入的方括号触发键全部失效。改本表新增键或改名时，**必须同步
  grep 检查** `aurora_core/apps/setting/src/assets/settings_manifest.toml` 与
  `aurora_core/apps/setting/src/key_conflict.rs` 的 `key_symbol()` 有没有过时或不一致的字符串。
- **注意类型**：`KeyEventData.prev_char`、`CommitRequestData.trigger_key` 是 **u16**
  （UTF-16 码元 / 协议字段），与 VK(`u32`) 比较前需 `as u32` 转换；prev_char 是字符码点不是 VK，
  别套 VK 常量（用数值区间，如 `(0x30..=0x39).contains(&prev_char)` 判数字字符）。

## 候选导航键（翻页 / 高亮 / 二三候选）—— 配置驱动 + 统一逻辑

这些键**都可配置**，且必须走**统一**入口，禁止各 handler 各写一套硬编码判断：

- **单一真源是 `keys.session_actions`**（`BTreeMap<键名, 动词>`，值域 `aurora_config::SessionAction`：
  `page_prev`/`page_next`/`highlight_up`/`highlight_down`/`cancel`/`select_candidate:N`/…）。
  `keys.page_keys`、`keys.highlight_keys`、`keys.select_key_groups` **只作折算来源**——消费点读折算后的
  `session_actions`，不要再去读那三张老表。`page_keys` 的 page 组优先于 highlight 组。
- **分类谓词唯一**：`keymap::KeyBinds::classify`（`NavKeys = KeyBinds<NavAction>`，来自 aurora-keys）；
  `ConfigBundle::session_keys` 与之同源，别各写一套判定。
  - `include_printable=true`（码表型：普通/特殊/mix/临拼）：`-`/`=`/`[`/`]` 可作翻页；
  - `include_printable=false`（文本/表达式型：临英/快捷输入）：上述键作输入，不当导航。
  - overlay handler 用 `handle_candidate_nav`（按 `state.active` 自判 `include_printable`）。
- **解析层**：`aurora-coordinator/src/key_resolver.rs`（分层按键配置 → 消费点直接可用的单一视图）。
  方案级两张表（`[session_actions]` / `[key_actions]`）按键名逐条比对；⚠️ **方案层不要加 VK 预编译
  缓存**——`EngineManager` 已有 `key_actions_cache`/`session_actions_cache`，再叠一层就多出必须同步
  失效的镜像态，且 `schema_generation` 不能当失效判据（设置页改 `schema_overrides` 走
  `invalidate_schema`，不 bump 它 → 表现为「改了不生效、重启才生效」）。
- 二三候选键：`select_key_offset`（读 `keys.select_key_groups`，经 `hotkey::select_key_vks` 折算）。
  - **可打印键组**（`;'` / `,.`）走 keydown：各模式 handler 自己调 `select_key_offset`。
  - **修饰键组**（`lrshift` / `lrctrl`）走 **keyup**：纯修饰键的 keydown 不能吃（宿主要看得见
    修饰键），且在 keydown 上判定会让 `Ctrl+A` 的第一下 Ctrl 误选候选。故它们注册进
    `key_up`（action=`select_candidate`），由 TSF 的轻敲机制（<500ms + 中途无别的键）转发
    keyup，协调器入口的 `handle_select_key_up` → `select_page_candidate` 按模式派发。
    与切换键撞键时：**有候选选词、无候选切换**；越界吞键（修饰键没有字符，不套 overflow）。
  - 因此 `is_toggle_mode_keycode` 必须按 action 过滤——key_up 表里不只有切换键了。
- 新增模式/按键时**复用**以上，不要再写 `0x21|0x22 =>` 之类分支。

## 跨组件硬约定（违反即复现历史 bug）

跨 crate / 跨语言边界的立约级不变量集中在此；crate 内部细节归 crate 级 `AGENTS.md`。

- **协议面三层闸门，改线格式先过机器**：命令码由 `apps/aurora-tools` 的 `gen_protocol` 对账；
  payload 的**字节布局**由 `aurora_core/crates/aurora-ipc/testdata/wire_vectors.json` 对账
  （TSF / Linux / macOS 三端单测读同一份，`AURORA_WIRE_BLESS=1 cargo test -p aurora-ipc --test
  wire_vectors` 重录）；纯判据由 `aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json`
  对账（消费方 `aurora-tsf-win/tests/policy_vectors.rs`）。三张表各管一层，别以为跑了一张就够。
  ⚠ `static_assert(sizeof(...))` **拦不住字段互换**——TSF 的 `SendKeyEvent` 直接 memcpy
  结构体上送，两个 `uint16_t` 换位置后字节数不变、编译全过、DLL 照常加载，只有真值表会红。
- **C++ 吃键集必须 ⊆ Rust 出字集**：`wind_tsf` 在 `OnTestKeyDown` 就决定是否吃键，早于 IPC
  往返；Rust 侧事后回 PassThrough 已来不及。凡 C++ 吃掉而 Rust 最终不出字的键，在严格 TSF
  宿主上直接丢失（历史案例：全角模式丢键、密码框丢键；指纹＝「有些应用打不出、有些出半角」）。
  给 C++ 侧新增吃键条件前，先确认 Rust 侧在同条件下必定产出。
- **候选排序必须落到 weight**：协调器会按 weight 统一重排候选；引擎内部只调顺序、不改
  weight 的排序会被重排冲掉。顶码上屏取首选与候选窗展示必须共用同一排序函数。
- **用户短语数据的真源是 `<userdata>/user/phrases.dict.yaml`**（aurora-store，全局不分方案）：
  系统种子在 `data/system.phrases.toml`（只读，协调器经 `Store::set_system_phrases` 交进内存），
  文件里只记用户行与「对系统条目的修改」行（`overrides_system = 1`），读时叠加。旧 redb
  `PHRASES` 表**只供迁移读**，别再往上写。
- **自带数据文件一律经覆盖解析函数定位，禁止直接 `data_dir.join(...)`**：用户目录同名文件
  整体替代安装目录那份（`Config::resolve_data_file` / `resolve_schema_resource` /
  `EngineManager::resolve_schema_file` / `resolve_dict_file`）。绕过解析函数是这套机制历史上
  **全部**缺陷的唯一形态（`common_chars.txt`、`pinyin_map.txt`、`unigram_path` 均栽于此），
  且失败静默——找不到就退化，不报错。键级合并只有 `config.toml` 与 `compat.toml` 两处。
- **出厂预置的绑定/开关，落点必须是用户能清空的载体**：配置四层（默认 < data < data_custom
  < user）与方案两层（`.schema.toml` < `schema_overrides`）用的都是**逐键深合并**
  （`merge_value` / `merge_toml`）——这个算子只能新增/覆盖，**表达不了删除**。所以把出厂值
  写进 map 类字段（`keys.key_actions`、`punct.custom_mappings` 那类）的某个子键，用户在设置页
  删掉后每次 `load()` 都会被合并回来，**永远关不掉**：软键盘 `ctrl+shift+k` 写在 L2 的
  `keys.key_actions` 里就是这么在 v0.120.0 报障的；更早一例是五处 `trigger_keys` 折算进
  `key_actions`，靠一次性物化（`Config::materialize_key_actions`）才收场。三条出路，按优先级：
  1. **出厂值放标量/数组字段**（`keys.softkeyboard`、`keys.toggle_toolbar` 那类）：清空即禁用，
     上层整体覆盖，天然可关。**新增内置功能的开关键一律先考虑这条。**
  2. 形态上进不了专用字段（「一键一功能」的 `key_actions` 就是）时，那张表的值域**必须**含一个
     表示禁用的显式值（本仓是 `"none"`），且它的**每条通路**都要认——组合键、单键、修饰键三条
     少认一条，那一档就还是关不掉。
  3. 整表替换（`merge_toml` 对 `custom_mappings` 的特判）：只在「这张表是一整份、不与任何层
     逐行混」时才成立。跨层叠加的表（`key_actions`）不能这么改，那会让方案升级后作者新增的
     条目透不过来。
  ⇒ 加任何出厂预置前先答这一句：**用户在 UI 上做的删除落到哪个键？那个键能否压制出厂值所在
  的那个键？** 答不出「能」，就是上面这个 bug。
- **`Config` / `Schema` 跨 crate 的 (反)序列化只走 aurora-config 的非泛型入口**
  （`from_toml_value` / `to_toml_value` / `to_json_value`、`Schema::probe_and_patch`）：release
  不共享泛型，下游每写一处 `value.try_into::<Schema>()`、`serde_json::to_value(cfg)` 或
  `toml::from_str::<Schema>`，就在那个 crate 里把整棵类型树的 serde 代码再编一份（每份约
  0.2 MiB，曾累计 1.5 MiB）。编译器不报错，类型推断出来的 `.try_into()?` 也一样中招。
  同理，文本解析成 `toml::Value` / `toml::Table` 用 `text.parse()`（实例在 toml crate 里只有
  一份），别用 `toml::from_str`。

## 日志规范

### 级别策略

| 级别 | 用途 | 隐私要求 |
|---|---|---|
| `error` | 不可恢复错误，影响功能 | 无用户数据 |
| `warn` | 可恢复异常，值得关注 | 无用户数据 |
| `info` | **生产默认级别**，关键生命周期事件 | **严禁**包含用户输入、词库词条、候选词等隐私数据 |
| `debug` | 诊断细节，开发时手动开启 | 可含调试上下文，部署时不应开启 |
| `trace` | 极细粒度追踪 | 仅本地调试 |

`info` 是正式部署时的唯一文件输出级别，开发者需在 `config.toml` 手动配置才能开启更详细级别：

```toml
[debug]
log_level = "debug"   # 或 "trace"
```

### 日志文件

- 滚动策略：**每次服务启动滚动一次**（`log_rotate::rotate_on_startup`，上次运行整体搬入
  历史文件），另按大小兜底（默认 10 MB/文件）；历史文件默认保留 10 个（`debug.log_max_files`）
- 文件命名：`aurora_input.1.log`（本次运行）、`aurora_input.2.log`（上次）… `aurora_input.11.log`。
  **每份都带序号、数字越小越新**，与设置程序的 `aurora_setting.N.log` 同一规则——早先当前那份
  叫 `aurora_input.log`（无序号），用户打包发日志时认不出它才是最新的，发来的常常只有 `.1.log`。
  **序号在扩展名之前**，滚动后仍是 `.log`（编辑器可双击、按 `*.log` 可搜）；勿改回 `.log.N` 旧式，
  也别把当前那份改回无序号
- 时间戳为**本地时区**，格式与 `wind_tsf` 的 FileLogger 完全一致，两份日志按时间直接对齐排查；
  勿退回 tracing 默认的 UTC SystemTime timer
- 路径（变体感知）：
  - 正常安装 release：`%LOCALAPPDATA%\AuroraInput\logs\`
  - 正常安装 dev：`%LOCALAPPDATA%\AuroraInputDev\logs\`
  - 便携模式：`<exe目录>\localdata\logs\`（以 exe 同目录存在 `portable_mode` 文件为标记；
    旧名 `wind_portable_mode` 仅保留读取兼容，新写入一律用新名）
- 可通过 `RUST_LOG` 环境变量覆盖级别（优先级最高，仅用于开发排查）

### 写日志准则

- `info!` 只记录系统事件（启动/关闭/加载/错误），**不得**记录用户键入的字符、候选词、词库内容
- `debug!` / `trace!` 可含诊断数据，但部署包中不应默认开启

## 提交与格式化（细则见 `CONTRIBUTING.md`「提交规范」）

本仓常有多个会话并行工作，因此：

- **只用显式路径**：`git add <具体文件>`，禁止 `git add -A` / `git add .` / `git commit -a`；提交前 `git status` 确认暂存区只含自己改的文件。
- **提交信息**：`type(scope): 摘要` + 中文正文；不带 `Co-Authored-By` / `Generated with` / `Constraint:` / `Confidence:` / `Tested:` 等 AI trailer；不带真实姓名、邮箱、内网地址、主机名与含用户名的本机路径（引用文件一律仓库相对路径）。
- **格式化**：改 Rust 文件后，验证通过前必须 `cargo fmt`（在 `aurora_core/` 下）；**逻辑改动与 fmt 改动分开提交**。开始新一轮前先 `git status` + `cargo fmt`，清掉上一轮遗留的纯格式漂移。

## 构建 / 测试

### 当前开发机：Fedora 44（本机现状）

本机是 **Fedora 44 Workstation + GNOME 50.3 + Wayland**，跑 Linux 全新的 `scripts/dev.sh`。

| 能做什么 | 命令 |
|---|---|
| host 单测 / 检查（**按 crate 范围**：改哪个测哪个） | `cargo test -p <crate>`；编译门用 `cargo check --workspace --all-targets`（在 `aurora_core/` 下） |
| 候选逻辑速验 | `./scripts/dev.sh r`（repl） |
| 设置程序交叉检查与 wine 测试 | `./scripts/dev.sh sk` / `st` / `sg`（见下节） |
| 与设置端产物对账 | `cargo test -p aurora-rpc --test wind_setting_assets`（见下） |

⚠️ **本机不要跑全量 `cargo test --workspace`**：113 个 test binary 并行编译 + 运行会把 load 打到
190 上下（实测整机卡死、用户正常使用被打断）。全量测试是 CI 的事；本机验证只跑改动涉及的 crate
（`cargo test -p <crate>`），需要跨 crate 的编译门时用 `cargo check --workspace --all-targets`
（分钟级以内，不跑测试进程）。

| 做不了 / 未配置 | 原因 |
|---|---|
| 任何写 `build[_dev]/`、`dist/` 的构建类命令 | **本机未创建 `scripts/build.local`**（模板见 `scripts/build.local.example`），构建类命令会**硬失败**而不是回落本机 |
| `dev.sh k` / `l`（cargo-xwin 交叉检查） | 需 clang-19 / lld-19 / llvm-19（MSVC STL 要 clang≥19），**本机未装** |
| `cargo check-android` | 需 `rustup target add aarch64-linux-android`，**本机未装该 target** |
| 发版 | 见下方「版本 / 发布」；另外 ⚠️ **本 checkout 缺 `docs/VERSION`**，「版本 / 发布」里有说明 |

已装工具：`cargo-xwin`、`wine-11.20 (Staging)`、`gh`、`podman`、`docker`。

### Windows 本机（MSVC，`scripts\dev.ps1` / `dev.bat`）

- host 即 Windows 目标，无交叉编译限制：`cargo check` / `cargo test` 可直接跑全 workspace
  （含 `aurora-coordinator`）。脚本快捷键：`k`=check、`l`=clippy、`t`=test、`f`=fmt、
  `ci`=fmt+clippy+test。
- 全构建：`1`（release → `build/`）/ `d1`（dev → `build_dev/`）；单模块 `m1..m4`（tsf/核心/
  setting/portable，前缀 `d` 为 dev）。系统安装：`p1`/`pd1`；安装包：`8`/`d8` → `dist\*-Setup.exe`。
- 部署目标默认 `C:\Program Files\WindInput[Dev]`，可在 `scripts\deploy.local.ps1` 覆盖。
- **照常用 `dev.ps1`**：`dm1` / `d1` / `t` / `ci` 等构建检查类命令自动转发，产物落回 `build[_dev]\`。
- **细粒度 cargo**：`.\scripts\rc.ps1 test -p aurora-coordinator`（直接敲 `cargo` 是在本机跑）。
- **任意命令**：`.\scripts\remote-build.ps1 -Raw "<命令>"`，在编译机的 `aurora_input\` 下执行。
- **临时回落本机**：`$env:AURORA_NO_REMOTE = "1"`（编译机关机 / 不在内网 / 要做 A-B 对照）。

⚠️ **编译机必须是 Windows + 原生 MSVC**：clang / cargo-xwin 交叉编译出的 `wind_tsf.dll` 在带
安全加固的宿主（企业微信 / TIM / UU 浏览器）里 COM 激活失败，已 A/B 实测锁定在工具链的代码
生成层（两份 DLL 的 PE 安全元数据逐位相同，换 cargo-xwin 用法或版本均无效）——因此发布链
从 ubuntu 交叉编译改回 windows-latest。Linux 编译机与 sccache-dist（其 build server 官方只
支持 Linux）都因此出局。

#### 同一台编译机同时只跑一个构建

看到 `[锁] 编译机正被另一个构建占用 (...), 等待...` 是正常的，它会自动排队并在对方结束后继续。
这道锁不是为了效率而是**正确性**：`Do-Full` 开头会清空 `build[_dev]/`，两个构建并发时后者
会把前者的产物整个删掉，而前者浑然不觉地走到打包——产出一个空安装包并报告「打包完成」，
全程零报错（实测：本该 20 MB 的包出了 2.3 MB）。锁按槽位隔离，不同 worktree 互不排队。
只读的 `-Raw` 查询可用 `-NoLock` 跳过排队；凡是会写 `target/` 或 `build/` 的命令都不要加。

⚠️ 并发还会让**任何性能测量失效**——同一条命令实测跑出过 83.8 / 96.5 / 103.4 / 135.9 s。
调优前先确认没有别人在用这台机器。

**Ctrl+C 中断构建后锁卡死怎么办**：陈旧判据是「锁文件创建时刻起 30 分钟未变」，Ctrl+C/断线
中断时锁不一定能立刻释放（本地进程如何退出不受脚本控制）。不想等 30 分钟：
`.\scripts\dev.ps1 runlock` 或 `.\scripts\remote-build.ps1 -Unlock`——只删锁文件，不碰源码/产物。
⚠️ 用前先确认没有其他会话正在用这台机器（见上一段），否则会打断真实在跑的构建。
（曾做过一版远程心跳自动续期/判陈旧的方案，机制验证有效，但对单人使用这台编译机而言过于
复杂，已撤回改回手动 `-Unlock`；如果以后这台机器变成多人共用、手动清锁不够用，可再按该方向补。）

#### 远程侧的构建是并行的

`remote-build.ps1` 会注入 `AURORA_PARALLEL_BUILD=1`，让全构建的 core / tsf / setting / portable
四步并行（它们各写各的产物，且 setting 与 portable 用各自仓库的 `target/`）。**本机直接跑
`dev.ps1` 时默认关闭**——12 核机器上同时跑四个构建会把机器压死。想在本机试可自行设该环境变量。

实测并行度约 1.9x 而非 4x：三个 cargo 进程会在**全局 package cache 锁**上排队
（输出里的 `Blocking waiting for file lock on package cache`），这是 cargo 的固有行为。

#### worktree 必须各占一个槽位

多个 worktree 共用一台编译机时，若都同步到同一个远程目录会**无声地互相覆盖**——后一次解压
盖掉前一次的源码，产物属于哪个分支全看谁最后跑完。`remote-build.ps1` 因此按 **worktree 目录名
自动派生槽位**（主树派生不出槽位，行为与从前一致），也可用环境变量覆盖：

```powershell
$env:AURORA_REMOTE_SLOT = "fx"     # → C:\build-fx\WindInput
$env:AURORA_REMOTE_SLOT = "off"    # 关掉槽位，与主树共用目录（串台风险自负）
```

⚠️ 槽位换的是**父目录**，不是主仓的目录名。设置程序已并入本仓（`aurora_core/apps/setting`），
`auroraui` 也已 vendored 进 `aurora_core/crates/auroraui`，两者都不再跨目录取 crate；仓外
path 依赖只剩可选的伴生仓（`../wind-portable`、`../wind-installer`）。改主仓目录名时这类
伴生仓会去旧路径取 crate，**取到的是别处代码、编译照样成功、错得毫无提示**。

⚠️ 每个槽位各带一份 `target\`（几十 GB），用完清掉：`remote-build.ps1 -Command <x> -Clean`。

#### 同步语义（三条容易误判的）

- 同步的是**工作树快照**（tar 打包文件系统），不是 git 提交。本地改了没提交的文件照样会过去。
- 反过来，**多会话并发改同一棵树时会抓拍到别人的半成品**。远程构建报的错若指向不在你改动集里
  的文件、且 `git status` 显示它干净，那就是撞上了中间态：别去改那份代码，也别反复重试（结果
  随机），等它收敛。开跑前可用 `rc.ps1 check --workspace --all-targets` 当门——不做 codegen，
  半分钟出结果。⚠️ `check -p <单个 crate>` **不能**当这个门：给枚举加变体时，报错的是所有
  `match` 它的下游 crate，`-p` 只覆盖其中一个。
- 同步是**镜像**：解压后会清掉编译机上多余的文件。这道清理不是洁癖——cargo 把 `tests/`、
  `benches/`、`examples/` 下的每个 `.rs` **自动发现**为独立编译目标，**不需要任何 `mod` 引用**，
  于是本机早已删除的测试文件会带着对已删字段的引用一起炸，而报错指向一个 `git` 和工作区里
  都找不到的文件。清理范围严格等于同步范围（同一份排除清单），`target/`、`build[_dev]/`、
  `dist/`、`.cache/` 一律不碰；只删文件不删目录。逃生口 `-NoPrune`，彻底重来 `-Clean`。

### Linux（MSVC / cargo-xwin，`scripts/dev.sh`）

⚠️ **本机不产出交付物**：凡会写进 `build[_dev]/` 或 `dist/` 的命令都要转发到 Windows 编译机
（配置 `scripts/build.local`，模板 `scripts/build.local.example`）。理由在工具链代码生成层——
clang / cargo-xwin 交叉编出的 `wind_tsf.dll` 在带安全加固的宿主里 **COM 激活失败**，原生 MSVC
编的没这个问题。⇒ **发版与部署的产物一律走编译机**，本机交叉只用于「编不编得过 / 逻辑对不对」。
`AURORA_BUILD_LOCAL=1` 可强行本机出二进制，那是用来看链接过不过的，**不能部署也不能发版**。

- 编译检查：`dev.sh k` / `l`（cargo-xwin，目标 `x86_64-pc-windows-msvc`；本机缺 clang-19 时跑不起来）。
- host 单测：`cargo test -p <crate>`（Linux 原生跑，`windows` crate 是 `cfg(windows)` 依赖不参与编译）。
  ⚠️ `dev.sh t` / `cargo test --workspace` 是**全量**（113 个 test binary 并行），本机跑会把 load
  打到 190 上下，只给 CI 用；注意 `build_dev/data` 的假绿判据（见下方「注意」）。
- 候选逻辑速验：`dev.sh r`（repl）。
- 部署 / 远程诊断：`dev.sh pd1`、`rproc`、`shot` 等（推的是编译机产出的产物）。

#### 设置程序（`apps/setting`）：交叉 + wine（`sk` / `st` / `sg`）

设置程序编不出**原生 Linux** 产物：它依赖 `auroraui`，而 `auroraui` 对非 Windows/macOS 目标
直接 `compile_error!`（rfd 的后端 feature 只是最先爆的那个，修好它照样过不去）。但**交叉到
Windows 目标一切正常**，测试二进制交给 wine 执行即可。三者都跑 `aurora_core/apps/setting`
（`dev.sh` 的 `SETTING_DIR`）：

| 命令 | 做什么 | 要 wine |
|---|---|---|
| `dev.sh sk` | 交叉检查（`--all-targets`，含测试代码） | 否 |
| `dev.sh st` | 设置程序全量测试（实测 1691 passed / 0 failed） | 是 |
| `dev.sh sg` | 重生成检入产物（capability 快照 / `mockdata/config.json`） | 是 |

装 wine（Fedora）：`sudo dnf install wine`（首次跑会建 `~/.wine` prefix，之后秒级；启动时那句
`wine32 is missing` 对 x64 目标无碍）。本机已是 `wine-11.20 (Staging)`。`sg` 取代了以前
「tar 两个仓 → scp 编译机 → 独立槽位 → 跑完删 2.6G target」那条路。

⚠️ `sg` 跑完 `git diff` 里可能只剩 `appVersion` 一行，那是**正常的**：它跟着版本号走，与配置
契约无关（邻仓守护测试刻意不比它）。别把它跟自己的改动一起提交。

#### 本机 wine 冒烟：核心 exe 的 CLI

交叉编出的 `aurora_input.exe` 在 wine 下跑得起来，CLI 子命令可用（实测 `--help` 与
`config get <key>` 都正常）。适合快速验证「配置读写 / 数据层加载」这类不碰宿主的逻辑，
省掉一轮推靶机。

⛔ **不能**拿它验证 TSF：`wind_tsf.dll` 要真实 TSF 宿主加载 + COM 激活，而交叉编出来的那份
恰恰就卡在 COM 激活（本节开头那条）。输入链路的验证仍然只能上靶机。
⛔ 这份产物同样**不可发版、不可部署**，理由同上。

#### 改了 core 的配置注册表，先跑这条

```bash
cargo test -p aurora-rpc --test wind_setting_assets
```

在 core 这边对账设置程序的三份检入产物（capability 快照逐键比、`mockdata` 比键集合、设置清单
覆盖单向断言），**不编译设置程序、也不需要 wine**，秒级。找不到设置目录时整族跳过并打印原因
（`AURORA_SETTING_DIR` 可指定位置）。它只管「core 这边看得见的事实」；值的策展偏离、
控件类型、选项文案仍由 `st` 那套把关，两者不重叠。

### macOS 本机（`scripts/mac/dev.sh`）

- host 即目标：`cargo check/test --workspace` 全 workspace 可直接跑（含 `aurora-coordinator`
  ——`windows` crate 是 `cfg(windows)` 依赖，非 Windows 上不参与编译）。脚本快捷键与
  `dev.ps1` 对齐：`k`/`l`/`t`/`f`/`fmt-check`/`ci`/`clean`/`gd`/`r`。
- 模块映射：Win 的 `m1`(TSF DLL) ↔ mac 的 `m1`(`WindInput.app`，Swift/IMKit)；
  Win 的 `m2`(核心 exe) ↔ mac 的 `m2`(Rust 服务)。全构建 `1`/`d1`，系统安装 `p1`/`pd1`，
  安装包 `8`/`d8` → `.pkg`。
- `.app` 侧单测：`cd wind_macos && swift test`。
- 与 Windows 的功能差距（宿主层）登记在 `aurora_macos/AGENTS.md`「与 Windows 的功能差距」。

### 靶机按键延迟探针（`scripts/perf/key-latency.sh`）

在靶机上直连服务 bridge 管道逐键发 KEY_EVENT 计往返耗时（Esc 撤销，不上屏不进词频），
可造 trim / purge 冷场景。子命令 `codes`（从日志取编码清单）/ `run`（跑各轮）/ `report`（汇总），
用法与注意事项（会闪候选窗、别在有人打字时跑）见脚本头部注释。

### 注意

- 部分集成测试依赖 `build_dev/` 下的数据（junction/词库）；**数据缺失时测试族静默跳过，
  0.0x 秒全绿＝假绿**，在 worktree 里跑测试尤其要核对耗时是否合理。

## 版本 / 发布

- 产品版本**唯一真源 = `docs/VERSION`**。构建脚本读取后分发到 5 类产物
  （`aurora_input.exe` / `wind_tsf.dll` / `aurora_setting.exe` / `aurora_portable.exe` / 安装包），
  跨仓经环境变量 `AURORA_APP_VERSION` 注入（不经脚本独立构建时各仓自行回退）。
  发版只改 `docs/VERSION` 一处，**不要**手改各仓 `Cargo.toml` 的 `version`。
  - ⚠️ **本 checkout 没有 `docs/VERSION`**（`scripts/dev.sh` 会回落成 `?`）。它由 CI 在构建时
    写入（`.github/workflows/release.yml` 读 tag 后 `printf > docs/VERSION`），本地开发也需要
    一份才能出正确版本号的产物——发版前先确认它存在。
- 发版入口：Linux 走 `scripts/release.sh`（直接跑进交互菜单；子命令 `check` → `push`
  → `wait` → `sign-draft`，或用 `auto-sign` 等 CI 跑完自动接签名上传），
  Windows 走 `scripts/release.ps1`。签名只能在 Windows 编译机上做（云签名客户端与
  `signtool` 只有 Windows 有），故 Linux 侧把流程劈成「本机用 `gh` 收发、编译机只签名」
  两段。
- Linux 包（`.deb`，amd64 + arm64）由 `release.yml` 并行调 `linux-build.yml` 构建，与 Windows/macOS **同为硬依赖**（任一平台失败都不建草稿）；
  想先单独发 Linux：手动触发 `release-linux.yml`（可填 tag 挂到该 tag 的 Release）。
  本机出包 `scripts/linux/package-deb.sh`，核对 `scripts/linux/verify-deb.sh`；
  另有 RPM 路线 `scripts/linux/package-rpm.sh` + `verify-rpm.sh` + `smoke-rpm.sh`（Fedora 侧）。
- 包管理器（winget `WindInput.WindInput` / Scoop `windinput-np`，在自有桶 huanfeng/scoop-bucket，Nonportable 待收录）只是调用标准安装包，
  清单开关绑定 wind-installer 的提权、退出码与 ARP 键名；清单在 `scripts/pkg-managers/`。
- CI（release.yml）为 tag-first：以 tag 覆盖 `docs/VERSION` 再构建；仓库里的 `docs/VERSION`
  是开发占位。**切勿添加 `tag == docs/VERSION` 一致性校验**——会破坏手动触发的 `-dev` 占位流程。
- 草稿 Release 的正文由 `scripts/gen-release-notes.sh` 生成：
  基础信息 + 人工填写区 + 折叠的提交记录。人工填写区由 `<!-- user-facing:start/end -->`
  圈定，**两个下游按此标记取内容**——文档仓 `scripts/sync_release_notes.py`（官网更新记录）
  与 `aurora_core/apps/setting/src/update/notes.rs`（应用内升级提示）。占位文本必须恰好是 `暂未填写`
  （Rust 侧按全等判定），前面加 `>` 之类修饰会让占位符被当成正文弹给用户。

## Agent skills

> 供 Matt Pocock 系列工程技能（`to-tickets` / `triage` / `to-spec` / `qa` / `wayfinder` 等）读取的仓级约定。

### Issue tracker

本地 markdown：issue 与 spec 存于 `.scratch/<feature>/`（一 feature 一目录，`issues/NN-<slug>.md` 一票一文件）。

### Triage labels

五个规范角色标签，标签名与角色名一致（`needs-triage` / `needs-info` / `ready-for-agent` / `ready-for-human` / `wontfix`），本地追踪器下写作 issue 文件顶部的 `Status:` 行。

### Domain docs

单上下文：根 `CONTEXT.md` + `docs/adr/`（均按需惰性生成，缺失时静默跳过）；本仓以 `AGENTS.md` 与各 crate 的 `AGENTS.md` 作为现状架构文档。
