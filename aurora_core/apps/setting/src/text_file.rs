//! 导入用的文本文件读取。导入导出与配置文件一律只认 UTF-8（可带 BOM），
//! 读不出时给一句用户能照着做的话，而不是 std 的英文报错。

use std::path::Path;

/// 界面上的固定提示，各导入导出入口共用一份措辞。
pub(crate) const UTF8_ONLY: &str = "文件须为 UTF-8 编码";

/// 读整个文本文件；失败时返回给用户看的完整一句话。内容原样返回（BOM 留给各格式自己处理）。
pub(crate) fn read_utf8(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读文件失败: {e}"))?;
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(
            "文件是 UTF-16 编码，只支持 UTF-8。请用记事本等编辑器另存为「UTF-8」后再导入".into(),
        );
    }
    String::from_utf8(bytes).map_err(|_| {
        "文件不是 UTF-8 编码（可能是 GBK/ANSI），只支持 UTF-8。\
         请用记事本等编辑器另存为「UTF-8」后再导入"
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 返回 `TempDir` 一并交给调用方持有：它一旦 drop 就删掉整个临时目录。
    fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        (tmp, p)
    }

    #[test]
    fn utf8_with_or_without_bom_reads_verbatim() {
        let (_t1, p) = write("ok.txt", "中文\n".as_bytes());
        assert_eq!(read_utf8(&p).unwrap(), "中文\n");
        let (_t2, p) = write("bom.txt", "\u{FEFF}中文".as_bytes());
        assert_eq!(read_utf8(&p).unwrap(), "\u{FEFF}中文");
    }

    #[test]
    fn gbk_and_utf16_name_the_encoding() {
        // 「中文」的 GBK 字节
        let (_t3, p) = write("gbk.txt", b"\xd6\xd0\xce\xc4");
        let gbk = read_utf8(&p).unwrap_err();
        assert!(
            gbk.contains("不是 UTF-8") && gbk.contains("另存为"),
            "{gbk}"
        );
        let (_t4, p) = write("u16.txt", b"\xff\xfe-N\x87e");
        let u16 = read_utf8(&p).unwrap_err();
        assert!(u16.contains("UTF-16"), "{u16}");
    }

    #[test]
    fn missing_file_is_a_read_error_not_an_encoding_one() {
        let e = read_utf8(Path::new("/nonexistent/ws_text_file.txt")).unwrap_err();
        assert!(e.starts_with("读文件失败") && !e.contains("UTF"), "{e}");
    }
}
