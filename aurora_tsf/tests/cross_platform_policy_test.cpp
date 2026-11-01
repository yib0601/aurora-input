// 跨端纯判据真值表对账（TSF 侧）。
//
// 跑法（纯 C++17，不含 Win32 头）：
//   g++ -std=c++17 -Wall -Wextra -I../include
//       -DAURORA_POLICY_JSON="<仓库>/aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json"
//       -o /tmp/cross_platform_policy_test cross_platform_policy_test.cpp
//   /tmp/cross_platform_policy_test
// （上面几行是一条命令，为避开注释续行的告警才折行抄写）
//
// 真值表是 TSF / macOS / Linux 三端**共用的同一份 JSON**（真源在 aurora-punct/testdata/）。
// 三方各读一遍、各自断言自己那套实现给出同样的答案——共享的是答案，不是实现。
//
// # 为什么需要这一层
//
// 三端各自都有单测（number_key_policy_test.cpp 等），但**测的是各自的理解**：三条实现同时
// 漂到同一个错误答案上，三方测试全绿而用户行为已错。源文件里那些「对位 macOS xxx 那边」
// 是注释，不是断言。本文件把「那边」变成会失败的测试。
//
// # 为什么手写一个极小 JSON 解析
//
// 不引入第三方 JSON 库：本仓的 C++ 侧一向零外部依赖（`aurora_tsf` 用 MSVC/静态 CRT，
// 加依赖会牵连 DLL 的自包含性），而这里只需要读一个结构固定的 JSON 数组。
// 解析器只认本文件用到的形态，**认不出就报错**——不猜。
//
// 变异检验：
//   - 把 ShouldEatNumberKey 的 `chineseMode && fullWidth` 改成 `||` ⇒ 中文半角空闲那条红
//   - 把 SmartPunctPrevChar 的 IsAsciiDigit 上界从 0x39 改成 0x40 ⇒ 真值表 ":" 那条红

#include "NumberKeyPolicy.h"
#include "SmartPunctPrevChar.h"

#include <cctype>
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#ifndef AURORA_POLICY_JSON
#error "编译时需 -DAURORA_POLICY_JSON=\"<仓库>/aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json\""
#endif

namespace
{

int g_failures = 0;

void Fail(const char* fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    std::printf("  FAIL  ");
    std::vprintf(fmt, ap);
    std::printf("\n");
    va_end(ap);
    ++g_failures;
}

// ─────────────────────────────────────────────────────────────
// 极简 JSON 读取：只覆盖真值表用到的形态
// ─────────────────────────────────────────────────────────────

/// 取某个 `"id": "<id>"` 所在 case 的 `"vectors": [...]` 原文切片。
/// 找不到返回空串；调用方负责报错。
std::string sliceVectors(const std::string& src, const std::string& id)
{
    const std::string idKey = "\"id\": \"" + id + "\"";
    size_t at = src.find(idKey);
    if (at == std::string::npos) return {};
    size_t vec = src.find("\"vectors\"", at);
    if (vec == std::string::npos) return {};
    size_t open = src.find('[', vec);
    if (open == std::string::npos) return {};
    // 括号配平，切出整个数组
    int depth = 0;
    for (size_t i = open; i < src.size(); ++i)
    {
        if (src[i] == '[') ++depth;
        else if (src[i] == ']')
        {
            if (--depth == 0) return src.substr(open, i - open + 1);
        }
    }
    return {};
}

/// 把切出的数组按顶层 `{...}` 切成一条条记录。
std::vector<std::string> splitObjects(const std::string& arr)
{
    std::vector<std::string> out;
    int depth = 0;
    size_t begin = std::string::npos;
    for (size_t i = 0; i < arr.size(); ++i)
    {
        if (arr[i] == '{')
        {
            if (depth++ == 0) begin = i;
        }
        else if (arr[i] == '}')
        {
            if (--depth == 0 && begin != std::string::npos)
            {
                out.push_back(arr.substr(begin, i - begin + 1));
                begin = std::string::npos;
            }
        }
    }
    return out;
}

/// 读布尔字段。缺字段直接报错（而不是默认 false）——静默默认会把「字段改名」变成假绿。
bool getBool(const std::string& obj, const std::string& key, const char* ctx)
{
    size_t at = obj.find("\"" + key + "\"");
    if (at == std::string::npos)
    {
        Fail("%s 缺字段 \"%s\"", ctx, key.c_str());
        return false;
    }
    size_t colon = obj.find(':', at);
    if (colon == std::string::npos) return false;
    size_t p = colon + 1;
    while (p < obj.size() && std::isspace(static_cast<unsigned char>(obj[p]))) ++p;
    return obj.compare(p, 4, "true") == 0;
}

/// 读整数字段。
long getInt(const std::string& obj, const std::string& key, const char* ctx, bool& ok)
{
    ok = false;
    size_t at = obj.find("\"" + key + "\"");
    if (at == std::string::npos)
    {
        Fail("%s 缺字段 \"%s\"", ctx, key.c_str());
        return 0;
    }
    size_t colon = obj.find(':', at);
    if (colon == std::string::npos) return 0;
    const char* s = obj.c_str() + colon + 1;
    char* end = nullptr;
    long v = std::strtol(s, &end, 10);
    if (end == s) return 0;
    ok = true;
    return v;
}

std::string readFile(const char* path)
{
    std::ifstream in(path, std::ios::binary);
    if (!in) return {};
    std::stringstream ss;
    ss << in.rdbuf();
    return ss.str();
}

// ─────────────────────────────────────────────────────────────
// 各组 case
// ─────────────────────────────────────────────────────────────

/// 数字类键吃键判据：2×2×2 全组合，与 NumberKeyPolicy.h 逐条对照。
void TestNumberKeyEat(const std::string& json)
{
    const std::string arr = sliceVectors(json, "number_key_eat");
    if (arr.empty())
    {
        Fail("真值表里没有 case number_key_eat（或格式变了）");
        return;
    }
    auto rows = splitObjects(arr);
    if (rows.size() != 8)
    {
        Fail("number_key_eat 应有 8 条向量，实得 %zu", rows.size());
        return;
    }
    for (const auto& row : rows)
    {
        bool ok = false;
        const bool session = getBool(row, "has_input_session", "number_key_eat");
        const bool chinese = getBool(row, "chinese_mode", "number_key_eat");
        const bool full = getBool(row, "full_width", "number_key_eat");
        const bool want = getBool(row, "eat", "number_key_eat");
        const long raw = getInt(row, "has_input_session", "number_key_eat", ok);
        (void)raw;
        const bool got = wind::numberkey::ShouldEatNumberKey(session, chinese, full);
        if (got != want)
        {
            Fail("ShouldEatNumberKey(session=%d chinese=%d full=%d) = %d，真值表要 %d",
                 int(session), int(chinese), int(full), int(got), int(want));
        }
    }
}

/// 数字后智能标点：prevChar 记账的「算不算 ASCII 数字」语义。
void TestSmartPunctDigit(const std::string& json)
{
    const std::string arr = sliceVectors(json, "smart_punct_after_digit");
    if (arr.empty())
    {
        Fail("真值表里没有 case smart_punct_after_digit（或格式变了）");
        return;
    }
    auto rows = splitObjects(arr);
    if (rows.size() < 8)
    {
        Fail("smart_punct_after_digit 至少 8 条向量，实得 %zu", rows.size());
        return;
    }
    for (const auto& row : rows)
    {
        bool ok = false;
        const long prev = getInt(row, "prev_char", "smart_punct_after_digit", ok);
        if (!ok) continue;
        const bool want = getBool(row, "is_digit", "smart_punct_after_digit");
        const bool got = WindSmartPunct::IsAsciiDigit(static_cast<uint16_t>(prev));
        if (got != want)
        {
            Fail("IsAsciiDigit(prev_char=0x%04lX) = %d，真值表要 %d", prev, int(got), int(want));
        }
    }
}

// ─────────────────────────────────────────────────────────────
// 结构自检：真值表本身没被改坏
// ─────────────────────────────────────────────────────────────

void TestTableIsWellFormed(const std::string& json)
{
    if (json.empty())
    {
        Fail("打不开真值表 %s", AURORA_POLICY_JSON);
        return;
    }
    for (const char* id : {"smart_punct_after_digit", "number_key_eat"})
    {
        if (sliceVectors(json, id).empty())
        {
            Fail("真值表缺 case %s", id);
        }
    }
}

} // namespace

int main()
{
    const std::string json = readFile(AURORA_POLICY_JSON);

    TestTableIsWellFormed(json);
    TestNumberKeyEat(json);
    TestSmartPunctDigit(json);

    if (g_failures == 0)
    {
        std::printf("cross_platform_policy_test (TSF): all passed\n");
        return 0;
    }
    std::printf("cross_platform_policy_test (TSF): %d failure(s)\n", g_failures);
    return 1;
}
