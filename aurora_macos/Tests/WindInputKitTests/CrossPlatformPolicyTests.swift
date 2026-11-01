import XCTest
import Foundation
@testable import WindInputKit

/// 跨端纯判据真值表对账（macOS 侧）。
///
/// 真值表是 TSF / macOS / Linux 三端**共用的同一份 JSON**（真源在
/// `aurora_core/crates/aurora-punct/testdata/`）。三方各读一遍、各自断言自己那套实现给出
/// 同样的答案——共享的是答案，不是实现。
///
/// # 为什么需要这一层
///
/// 三端各自都有单测（`SmartPunctDigitTrackerTests.swift` 等），但**测的是各自的理解**：
/// 三条实现同时漂到同一个错误答案上，三方测试全绿而用户行为已错。源文件里那些
/// 「对位 Windows xxx 那边」是注释，不是断言。本文件把「那边」变成会失败的测试。
///
/// # 找表的方式
///
/// 顺 `#filePath` 向上找到同时含 `aurora_core/` 与 `aurora_tsf/` 的目录。找不到就**跳过并
/// 打印原因**（与 `toggle_bits_match_cpp.rs` 同款取舍：只检出 Swift 子树时不制造假红）。
/// CI 的 macOS job 检的是整仓，届时会真跑。
final class CrossPlatformPolicyTests: XCTestCase {

    /// 顺路径向上找仓库根。
    private static func repoRoot() -> URL? {
        var dir = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        for _ in 0..<12 {
            let core = dir.appendingPathComponent("aurora_core")
            let tsf = dir.appendingPathComponent("aurora_tsf")
            if FileManager.default.fileExists(atPath: core.path),
               FileManager.default.fileExists(atPath: tsf.path) {
                return dir
            }
            dir = dir.deletingLastPathComponent()
        }
        return nil
    }

    private static func table() throws -> [String: Any] {
        guard let root = repoRoot() else {
            throw XCTSkip("找不到仓库根，跳过跨端真值表对账")
        }
        let url = root
            .appendingPathComponent("aurora_core/crates/aurora-punct/testdata")
            .appendingPathComponent("cross_platform_policy_vectors.json")
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw XCTSkip("找不到真值表 \(url.path)")
        }
        let data = try Data(contentsOf: url)
        guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            XCTFail("真值表不是 JSON 对象")
            return [:]
        }
        return obj
    }

    /// 取出某个 case 的 vectors。
    private func vectors(_ table: [String: Any], _ id: String) throws -> [[String: Any]] {
        guard let cases = table["cases"] as? [[String: Any]] else {
            XCTFail("cases 必须是数组")
            return []
        }
        guard let c = cases.first(where: { $0["id"] as? String == id }) else {
            XCTFail("真值表里没有 case `\(id)`")
            return []
        }
        guard let rows = c["vectors"] as? [[String: Any]], !rows.isEmpty else {
            XCTFail("case `\(id)` 的 vectors 空着")
            return []
        }
        return rows
    }

    private func bool(_ row: [String: Any], _ key: String) -> Bool? {
        row[key] as? Bool
    }

    private func uint16(_ row: [String: Any], _ key: String) -> UInt16? {
        guard let n = row[key] as? Int, let v = UInt16(exactly: n) else { return nil }
        return v
    }

    // MARK: - 数字后智能标点

    /// `SmartPunctDigitTracker` 的记账语义：只有 ASCII 数字算「刚打了数字」。
    ///
    /// 真值表钉的是**「这个码元算不算数字」**这一层，与 tracker 内部判断逐条对照。
    /// 注意 prev_char 是 UTF-16 码元（协议字段就是 u16），不是字符码点。
    func testSmartPunctDigitSemanticsMatchTable() throws {
        let table = try Self.table()
        let rows = try vectors(table, "smart_punct_after_digit")
        XCTAssertGreaterThanOrEqual(rows.count, 8, "真值表条数不足")

        for row in rows {
            guard let prev = uint16(row, "prev_char"), let want = bool(row, "is_digit") else {
                XCTFail("向量字段缺失或类型不对: \(row)")
                continue
            }
            // 与 TSF `WindSmartPunct::IsAsciiDigit` / Linux `IsAsciiDigit` / Rust
            // `(0x30..=0x39).contains(&prev)` 是同一判据。
            let got = prev >= 0x30 && prev <= 0x39
            XCTAssertEqual(
                got, want,
                String(format: "prev_char=0x%04X：判据得 %@，真值表要 %@",
                       prev, String(got), String(want))
            )
        }
    }

    /// 用真值表的每一条喂 `SmartPunctDigitTracker` 的真实记账路径，确认「表 ⟹ 实现」也成立。
    ///
    /// tracker 的判据是「我们自己送进文档的东西」：`noteCommittedText()` / `noteKeyPassthrough()`
    /// 记账，`prevChar` 就是发给服务端的值。数字 → 报该码元；非数字 → 归 0（「不可用」）。
    func testSmartPunctTrackerAgreesWithTable() throws {
        let table = try Self.table()
        let rows = try vectors(table, "smart_punct_after_digit")

        for row in rows {
            guard let prev = uint16(row, "prev_char"), let want = bool(row, "is_digit") else {
                continue
            }
            let tracker = SmartPunctDigitTracker()
            // 走「经引擎上屏的文本」这条真实记账路径：记末位字符，ASCII 数字则记、
            // 否则清零（全角数字刻意不记——服务端只认 ASCII）。
            guard let scalar = UnicodeScalar(prev) else { continue }
            // 用 unicodeScalars 追加而非 Character(UnicodeScalar)：后者在不同 Swift 版本里
            // 的可用 initializer 不一致，而这个随后要拼进 JSON 之外的地方，越直白越安全。
            var committed = String()
            committed.unicodeScalars.append(scalar)
            tracker.noteCommittedText(committed)
            let reported = tracker.prevChar
            if want {
                XCTAssertEqual(
                    reported, prev,
                    String(format: "prev_char=0x%04X 是数字，tracker 应报它而不是 0x%04X",
                           prev, UInt32(reported))
                )
            } else if prev != 0 {
                // 非数字一律归 0 —— 这正是 macOS 侧的契约（见 SmartPunctDigitTracker.swift 头注释）。
                XCTAssertEqual(
                    reported, 0,
                    String(format: "prev_char=0x%04X 不是数字，tracker 应报 0 而不是它", prev)
                )
            }
        }
    }

    // MARK: - 数字键吃键

    /// 数字类键判据。macOS 没有 TSF 的前置闸门，但「该不该把这个数字键转发给服务端」
    /// 是同一判据（Linux addon 同理）。这里用与 `NumberKeyPolicy.h` 逐字相同的表达式对照。
    func testNumberKeyEatMatchesTable() throws {
        let table = try Self.table()
        let rows = try vectors(table, "number_key_eat")
        XCTAssertEqual(rows.count, 8, "数字键判据应覆盖 2×2×2 全部组合")

        for row in rows {
            guard let session = bool(row, "has_input_session"),
                  let chinese = bool(row, "chinese_mode"),
                  let full = bool(row, "full_width"),
                  let want = bool(row, "eat") else {
                XCTFail("向量字段缺失或类型不对: \(row)")
                continue
            }
            let got = session || (chinese && full)
            XCTAssertEqual(
                got, want,
                "session=\(session) chinese=\(chinese) full=\(full)：判据得 \(got)，真值表要 \(want)"
            )
        }
    }
}
