<!-- Parent: ../../../AGENTS.md -->

# aurora-wdict

> wdict（`.wdict.yaml`）交换格式读写与词条文本转义；依赖图底层，不依赖 `aurora-store`/`aurora-dict`。

## 职责

wdict（`.wdict.yaml`）交换格式的读写（phrases / words / shadow / freq 各段），以及词条文本域的转义（`escape_text_field` / `unescape_text_field`，命令栏语法条目只保护分隔符）。原是 `aurora-store::wdict`；P1b 抽成独立 crate，让 `aurora-dict` 不必为几个文本函数依赖用户库（`aurora-store → aurora-userdata → aurora-dict` 才能单向成立）。`aurora-store` 用 `pub use aurora_wdict as wdict;` 保留原路径，旧调用方不改。

## 关键文件

| 文件 | 说明 |
|------|------|
| `src/lib.rs` | 全部格式代码与测试（原 `aurora-store/src/wdict.rs` 原样搬入） |

## 硬约定

- **不要依赖 `aurora-store` / `aurora-dict`**：本 crate 在依赖图底层，只依赖 `aurora-utils` 与 `aurora-cmdbar`。往这里加依赖前先看会不会回绕成环。
- 转义规则与命令栏判据的取舍说明见文件头注释；文本域与编码域用的是两对不同的函数。

## 构建与测试

- `cargo test -p aurora-wdict`（纯 Rust，任意 host 可跑）。

## 依赖

### 内部

- `aurora-utils` — `normalize_input`
- `aurora-cmdbar` — `is_cmdbar_grammar` / `contains_marker_call`

## 全局约束

按需引用根 `AGENTS.md`（提交与格式化规范以它为准）。本 crate 特有：

- 位于依赖图底层：新增依赖前先确认不会与 `aurora-store` / `aurora-dict` 形成环。
