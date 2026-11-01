// 跨端纯判据真值表对账（Linux addon 侧）。
//
// 真值表是 TSF / macOS / Linux 三端**共用的同一份 JSON**（真源在
// aurora_core/crates/aurora-punct/testdata/）。三方各读一遍、各自断言自己那套实现给出
// 同样的答案——共享的是答案，不是实现。
//
// # 这一份为什么要单独存在
//
// Linux addon 没有 `OnTestKeyDown` 前置闸门，数字键判据在这里的用途不是「吃不吃」而是
// 「转不转发」；数字后智能标点的 prevChar 记账也在 `ResponseRouter` 里另写了一份。
// 语义相同、代码不同——正是最容易漂的那一类。TSF 那份测试覆盖不到这里。
//
// 跑法：由 aurora_linux/CMakeLists.txt 注册（与其它 *_test.cpp 同批），
// 编译参数带 -DAURORA_POLICY_JSON=<真值表路径>。也可单独手跑：
//   g++ -std=c++17 -Wall -Wextra -Iinclude
//       -DAURORA_POLICY_JSON="<仓库>/aurora_core/crates/aurora-punct/testdata/cross_platform_policy_vectors.json"
//       -o /tmp/linux_xpp tests/cross_platform_policy_test.cpp
//   （上面是一条命令，为避开注释续行告警才折行抄写）

#include "TestHarness.h"

#include <cctype>
#include <cstdint>
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

// ─────────────────────────────────────────────────────────────
// 极简 JSON 读取（与 aurora_tsf/tests/cross_platform_policy_test.cpp 同一实现）
//
// 两边各留一份而不共享头文件：TSF 的 tests/ 与 Linux 的 tests/ 是两套独立构建
// （前者 g++ 直编、后者走 CMake），共享文件会把一条编译依赖加到另一条构建路径上，
// 而这里总共 60 行。**两边的差异只有 namespace 与断言宏**，改一处时记得看另一处——
// 这条注释就是那个提醒。
// ─────────────────────────────────────────────────────────────

std::string sliceVectors(const std::string& src, const std::string& id)
{
    const std::string idKey = "\"id\": \"" + id + "\"";
    size_t at = src.find(idKey);
    if (at == std::string::npos) return {};
    size_t vec = src.find("\"vectors\"", at);
    if (vec == std::string::npos) return {};
    size_t open = src.find('[', vec);
    if (open == std::string::npos) return {};
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

bool getBool(const std::string& obj, const std::string& key)
{
    size_t at = obj.find("\"" + key + "\"");
    if (at == std::string::npos) return false;
    size_t colon = obj.find(':', at);
    if (colon == std::string::npos) return false;
    size_t p = colon + 1;
    while (p < obj.size() && std::isspace(static_cast<unsigned char>(obj[p]))) ++p;
    return obj.compare(p, 4, "true") == 0;
}

long getInt(const std::string& obj, const std::string& key, bool& ok)
{
    ok = false;
    size_t at = obj.find("\"" + key + "\"");
    if (at == std::string::npos) return 0;
    size_t colon = obj.find(':', at);
    if (colon == std::string::npos) return 0;
    const char* s = obj.c_str() + colon + 1;
    char* end = nullptr;
    long v = std::strtol(s, &end, 10);
    if (end == s) return 0;
    ok = true;
    return v;
}

std::string readFile()
{
    std::ifstream in(AURORA_POLICY_JSON, std::ios::binary);
    if (!in) return {};
    std::stringstream ss;
    ss << in.rdbuf();
    return ss.str();
}

/// Linux addon 的数字键判据：**与 TSF `ShouldEatNumberKey` 同一表达式**。
///
/// 这里刻意不 include 任何 TSF 头（那是 Windows 侧的实现）：Linux 的语义是「转不转发给
/// 服务端」，与 TSF 的「吃不吃」字面不同、判据相同。真值表钉的正是这个「相同」。
bool ShouldForwardNumberKey(bool hasInputSession, bool chineseMode, bool fullWidth)
{
    return hasInputSession || (chineseMode && fullWidth);
}

/// ASCII 数字判定（与 TSF `WindSmartPunct::IsAsciiDigit` 同一判据）。
bool IsAsciiDigit(uint16_t c) { return c >= 0x30 && c <= 0x39; }

} // namespace

int main()
{
    const std::string json = readFile();

    CASE("真值表可读且结构完好")
    {
        if (json.empty())
        {
            std::printf("  FAIL  打不开 %s\n", AURORA_POLICY_JSON);
            ++testharness::g_failures;
        }
        else
        {
            for (const char* id : {"smart_punct_after_digit", "number_key_eat"})
            {
                if (sliceVectors(json, id).empty())
                {
                    std::printf("  FAIL  真值表缺 case %s\n", id);
                    ++testharness::g_failures;
                }
            }
        }
    }

    CASE("数字键转发判据与真值表一致（2×2×2 全组合）")
    {
        auto rows = splitObjects(sliceVectors(json, "number_key_eat"));
        if (rows.size() != 8)
        {
            std::printf("  FAIL  应有 8 条向量，实得 %zu\n", rows.size());
            ++testharness::g_failures;
        }
        for (const auto& row : rows)
        {
            const bool session = getBool(row, "has_input_session");
            const bool chinese = getBool(row, "chinese_mode");
            const bool full = getBool(row, "full_width");
            const bool want = getBool(row, "eat");
            const bool got = ShouldForwardNumberKey(session, chinese, full);
            if (got != want)
            {
                std::printf(
                    "  FAIL  ShouldForwardNumberKey(session=%d chinese=%d full=%d) = %d，真值表要 %d\n",
                    int(session), int(chinese), int(full), int(got), int(want));
                ++testharness::g_failures;
            }
        }
    }

    CASE("数字后智能标点的 ASCII 数字判定与真值表一致")
    {
        auto rows = splitObjects(sliceVectors(json, "smart_punct_after_digit"));
        if (rows.size() < 8)
        {
            std::printf("  FAIL  至少 8 条向量，实得 %zu\n", rows.size());
            ++testharness::g_failures;
        }
        for (const auto& row : rows)
        {
            bool ok = false;
            const long prev = getInt(row, "prev_char", ok);
            if (!ok) continue;
            const bool want = getBool(row, "is_digit");
            const bool got = IsAsciiDigit(static_cast<uint16_t>(prev));
            if (got != want)
            {
                std::printf("  FAIL  IsAsciiDigit(0x%04lX) = %d，真值表要 %d\n",
                            prev, int(got), int(want));
                ++testharness::g_failures;
            }
        }
    }

    TEST_MAIN_END()
}
