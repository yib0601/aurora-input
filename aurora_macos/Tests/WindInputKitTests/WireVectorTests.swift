import XCTest
import Foundation
@testable import WindInputKit

/// 线格式真值表对账（macOS 侧）。
///
/// 真值表是 TSF / macOS / Linux 三端**共用的同一份 JSON**（真源在
/// `aurora_core/crates/aurora-ipc/testdata/`，由 Rust 那份编码器重录）。三方各读一遍、
/// 各自断言自己那套实现给出同样的字节——共享的是答案，不是实现。
///
/// # 为什么需要这一层
///
/// `BinaryCodec.swift` 是 Rust 编解码器的第三份实现。命令码已有机器对账
/// （`ProtocolTypes.swift` 的常量 + `gen_protocol`），但 **payload 的字节布局没有**：
/// `flags:u32 + textLen:u32 + compLen:u32` 这种头，任一字段错位都表现为「上屏一串错字」
/// 或「preedit 残留」，两端都不报错、日志里一个字都没有。`BinaryCodecTests.swift`
/// 测的是**本端自己的理解**——三条实现同时漂到同一个错误答案上，三方测试全绿而用户已错。
///
/// # 找表的方式
///
/// 顺 `#filePath` 向上找同时含 `aurora_core/` 与 `aurora_tsf/` 的目录。找不到就**跳过并
/// 打印原因**（与 `CrossPlatformPolicyTests` / `toggle_bits_match_cpp.rs` 同款取舍：
/// 只检出 Swift 子树时不制造假红）。
final class WireVectorTests: XCTestCase {

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

    private struct Vector {
        let name: String
        let cmd: UInt16
        let producer: String
        let bytes: Data
    }

    private static func loadVectors() throws -> [Vector] {
        guard let root = repoRoot() else {
            throw XCTSkip("找不到仓库根，跳过线格式真值表对账")
        }
        let url = root
            .appendingPathComponent("aurora_core/crates/aurora-ipc/testdata")
            .appendingPathComponent("wire_vectors.json")
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw XCTSkip("找不到真值表 \(url.path)")
        }
        let data = try Data(contentsOf: url)
        guard
            let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
            let cases = obj["cases"] as? [[String: Any]]
        else {
            XCTFail("真值表不是预期的 JSON 结构")
            return []
        }
        return cases.compactMap { c in
            guard
                let name = c["name"] as? String,
                let cmd = c["cmd"] as? Int,
                let producer = c["producer"] as? String,
                let hex = c["hex"] as? String
            else {
                XCTFail("真值表条目缺字段：\(c)")
                return nil
            }
            var bytes = Data(capacity: hex.count / 2)
            var i = hex.startIndex
            while i < hex.endIndex {
                let j = hex.index(i, offsetBy: 2)
                guard let v = UInt8(hex[i..<j], radix: 16) else {
                    XCTFail("`\(name)` 的 hex 不是合法十六进制")
                    return nil
                }
                bytes.append(v)
                i = j
            }
            return Vector(name: name, cmd: UInt16(cmd), producer: producer, bytes: bytes)
        }
    }

    private func vector(_ all: [Vector], _ name: String) throws -> Vector {
        guard let v = all.first(where: { $0.name == name }) else {
            XCTFail("真值表里没有 `\(name)`")
            throw XCTSkip("缺条目")
        }
        return v
    }

    /// 每条真值表的 8 字节头都要能被本端 `decodeHeader` 接受，且长度自洽。
    func testHeadersMatchVectors() throws {
        let all = try Self.loadVectors()
        XCTAssertFalse(all.isEmpty, "真值表为空")
        for v in all {
            XCTAssertGreaterThanOrEqual(v.bytes.count, 8, "`\(v.name)` 比一个头还短")
            let header = try BinaryCodec.decodeHeader(v.bytes)
            XCTAssertEqual(header.cmd, v.cmd, "`\(v.name)` 的命令码不符")
            XCTAssertEqual(
                Int(header.length),
                v.bytes.count - 8,
                "`\(v.name)` 的 header.length 与实际载荷长度不符"
            )
        }
    }

    /// 上行：本端编码器的产出必须与真值表逐字节相同。
    func testKeyEventFrameMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "key_event_all_fields_distinct")

        let p = KeyEventPayload(
            keyCode: 0x41,
            scanCode: 0x1E,
            modifiers: 0x0005,
            eventType: .down,
            toggles: 0x03,
            eventSeq: 0x1234,
            prevChar: 0x4E2D
        )
        XCTAssertEqual(
            BinaryCodec.encodeKeyEventFrame(p),
            v.bytes,
            "KEY_EVENT 上行帧与真值表不一致——C++/Rust 那边读的就是这串字节"
        )
    }

    /// 下行：本端解码器读出来的内容必须与真值表描述的一致。
    func testCommitTextDecodeMatchesVectors() throws {
        let all = try Self.loadVectors()

        for (name, wantText, wantComp, wantFlags) in [
            ("commit_text_plain", "hi", "", UInt32(0x04)),
            ("commit_text_with_new_composition", "hi", "hao", UInt32(0x07)),
            ("commit_text_replacing_held", ",", "", UInt32(0x0C)),
        ] {
            let v = try vector(all, name)
            let payload = v.bytes.dropFirst(8)
            let p = try BinaryCodec.decodeCommitTextPayload(Data(payload))
            XCTAssertEqual(p.text, wantText, "`\(name)` 的正文不符")
            XCTAssertEqual(p.newComposition, wantComp, "`\(name)` 的新组合文本不符")
            XCTAssertEqual(p.flags, wantFlags, "`\(name)` 的 flags 不符")
        }
    }

    func testUpdateCompositionDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "update_composition_ascii")
        let p = try BinaryCodec.decodeUpdateCompositionPayload(Data(v.bytes.dropFirst(8)))
        XCTAssertEqual(p.caretPos, 2)
        XCTAssertEqual(p.text, "ni")
    }

    func testCommitTextWithCursorDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "commit_text_with_cursor")
        let p = try BinaryCodec.decodeCommitTextWithCursorPayload(Data(v.bytes.dropFirst(8)))
        XCTAssertEqual(p.text, "hi!")
        XCTAssertEqual(p.cursorOffset, 2)
    }

    func testMoveCursorDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "move_cursor_count")
        let p = try BinaryCodec.decodeMoveCursorPayload(Data(v.bytes.dropFirst(8)))
        // Swift 侧字段仍叫 `direction`（历史名），Rust/C++ 已改叫 count：
        // 旧版当 direction（恒 1）读，新版当「向右移动的格数」读；两版读的正是这 4 字节。
        XCTAssertEqual(p.direction, 3)
    }

    func testCandidateRectsDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "candidate_rects_two")
        let rects = try BinaryCodec.decodeCandidateRectsPayload(Data(v.bytes.dropFirst(8)))
        XCTAssertEqual(rects.count, 2)
        guard rects.count == 2 else { return }
        XCTAssertEqual(rects[0].index, 0)
        XCTAssertEqual(rects[0].x, 0)
        XCTAssertEqual(rects[0].y, 10)
        XCTAssertEqual(rects[0].w, 20)
        XCTAssertEqual(rects[0].h, 24)
        XCTAssertEqual(rects[1].index, 1)
        XCTAssertEqual(rects[1].x, 10)
        XCTAssertEqual(rects[1].y, 46)
    }

    func testModePushDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "mode_push_all_flags")
        let payload = v.bytes.dropFirst(8)
        XCTAssertEqual(payload.count, 4, "MODE_PUSH 恒为 4 字节 flags")
        guard payload.count == 4 else { return }
        // 三个位各自独立：中英 / 全角 / 中文标点。
        // 漏标点位会让中文标点态误用英文集合（`, . '` 直接透传成半角）。
        let p = [UInt8](payload)
        let flags = UInt32(p[0]) | (UInt32(p[1]) << 8) | (UInt32(p[2]) << 16) | (UInt32(p[3]) << 24)
        XCTAssertEqual(flags & 0x0001, 0x0001, "chineseMode 位")
        XCTAssertEqual(flags & 0x0002, 0x0002, "fullWidth 位")
        XCTAssertEqual(flags & 0x0004, 0x0004, "chinesePunct 位")
    }

    func testExtEnvelopeDecodeMatchesVector() throws {
        let all = try Self.loadVectors()
        let v = try vector(all, "ext_envelope")
        guard let env = BinaryCodec.decodeExt(Data(v.bytes.dropFirst(8))) else {
            XCTFail("EXT 信封解不回来")
            return
        }
        XCTAssertEqual(env.kind, "notify")
        XCTAssertEqual(env.body, Data("hi".utf8))
    }
}
