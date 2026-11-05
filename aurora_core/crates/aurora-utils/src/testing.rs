//! 测试专用临时目录托管（`feature = "testing"`，**只给测试代码用**）。
//!
//! # 为什么需要它
//!
//! 测试里常见这种辅助函数：
//!
//! ```ignore
//! fn data_dir(tag: &str) -> PathBuf {           // ← 只把路径返回出去
//!     let d = std::env::temp_dir().join(format!("wind_{tag}_{}", std::process::id()));
//!     std::fs::create_dir_all(&d).unwrap();
//!     d
//! }
//! ```
//!
//! 返回值被调用方持有，路径的生命周期比函数长。这时若在函数里放一个
//! `tempfile::TempDir`，它会在函数返回时 drop → 调用方拿到一个**已被删除**的路径，
//! 而编译器不会报错（只有测试会红）。反过来，保留 `std::env::temp_dir()` 就会漏
//! ——本机 `/tmp` 是 tmpfs，漏出来的 wdat/redb 直接吃内存，攒够就 ENOSPC。
//!
//! 本模块把目录**交给当前线程托管**：libtest 每个 `#[test]` 跑在自己的线程里，
//! 线程结束（含 panic unwind）时 RAII 删除。调用方签名不用改，也不用手工
//! `remove_dir_all` 收尾。
//!
//! # 用法
//!
//! ```ignore
//! fn data_dir(tag: &str) -> PathBuf {
//!     aurora_utils::testing::scratch(tag)   // 目录已建好，线程结束时自动删
//! }
//! ```
//!
//! # 边界
//!
//! - 目录的存活期是「本线程」，不是「本函数」。所以**不要**在 `thread::spawn` 出来的
//!   子线程里调用它、再把路径交回父线程用：子线程一结束目录就没了。
//! - 进程级单例夹具（`OnceLock<&'static Path>` 那类）用 [`process_scratch`]：生命周期是
//!   「进程」，由 atexit 在退出时删。
//! - 能在 `#[test]` 函数体内直接 `let tmp = tempfile::tempdir()` 的，优先用那种
//!   ——作用域一结束就删，比线程托管更紧。

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, Once};

/// 一个「随线程结束删除」的目录。
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    static OWNED: RefCell<Vec<Scratch>> = const { RefCell::new(Vec::new()) };
}

/// 建一个唯一临时目录并交给当前线程托管，返回它的路径。
///
/// 目录名由 `tag` + 进程号 + 自增序号组成（并行测试互不撞车）；位置仍是
/// `std::env::temp_dir()`，所以 `TMPDIR` 兜底照样生效。
pub fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "wind-test-{}-{}-{tag}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("创建测试临时目录失败");
    OWNED.with(|v| v.borrow_mut().push(Scratch(path.clone())));
    path
}

/// 把已经建好的目录也纳入本线程托管（不想改调用方、但目录是别处建的时用）。
pub fn adopt(path: PathBuf) -> PathBuf {
    OWNED.with(|v| v.borrow_mut().push(Scratch(path.clone())));
    path
}

// ---------------------------------------------------------------- 进程级

/// 进程级托管：目录活到**进程退出**，unix 上由 `atexit` 删除。
///
/// 给「进程级单例夹具」用——`static ROOT: OnceLock<PathBuf>` + `set_var(...)` 那一类，
/// 一个测试二进制里只建一次、被多个 `#[test]` 共用。这种夹具**不能**用 [`scratch`]：
/// 目录归最先跑进来的那个 test 线程，该线程一结束就被删，而 `OnceLock` 里缓存的路径
/// 还在 → 后面的测试拿到的是已删除的夹具根（编译不报错，测试成片红）。
///
/// # 平台差异
///
/// - unix：注册一次 `atexit`，进程正常退出时递归删除。**不新增依赖**——`atexit` 直接
///   声明 POSIX 符号，不走 libc crate。
/// - 非 unix：不删（`atexit` 在 Windows 上是 CRT 语义，与 rust 运行时混用不值得为测试
///   引入风险）。Windows 的临时目录在磁盘上，不占内存；Linux 才是 tmpfs 受害方。
pub fn process_scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "wind-proc-{}-{}-{tag}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).expect("创建测试临时目录失败");
    {
        let mut dirs = PROC_DIRS.lock().unwrap_or_else(|e| e.into_inner());
        dirs.push(path.clone());
    }
    register_exit_hook();
    path
}

static PROC_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static EXIT_HOOK: Once = Once::new();

extern "C" fn cleanup_at_exit() {
    // 只做 fs 删除：不 panic、不碰已释放内存（atexit 回调里 panic 会 abort）。
    let dirs = PROC_DIRS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

#[cfg(unix)]
fn register_exit_hook() {
    // safety: `atexit` 是 POSIX 标准符号（C ABI，签名 `int atexit(void (*)(void))`），
    // 链接期由 libc 提供；传入的回调是签名严格匹配的 extern "C" 函数。
    unsafe extern "C" {
        fn atexit(cb: extern "C" fn()) -> std::ffi::c_int;
    }
    EXIT_HOOK.call_once(|| unsafe {
        atexit(cleanup_at_exit);
    });
}

#[cfg(not(unix))]
fn register_exit_hook() {
    // 非 unix：不做 exit 钩子（见 `process_scratch` 的平台差异说明）。
}
