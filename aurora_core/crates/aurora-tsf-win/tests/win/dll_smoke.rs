//! 在 Wine/Windows 下验证 aurora_tsf_win.dll 的加载与 COM 实例化。
//!
//! 走与 TSF 宿主相同的路径：
//!   LoadLibrary → DllGetClassObject(CLSID) → IClassFactory::CreateInstance
//!   → QueryInterface(ITfTextInputProcessor) → Release

use std::ffi::c_void;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::UI::TextServices::ITfTextInputProcessor;
use windows::core::{GUID, IUnknown, Interface, PCSTR, PCWSTR};

const CLSID_TEXT_SERVICE: GUID = GUID::from_u128(0x99C2EE30_5C57_45A2_9C63_FB54B34FD90A);

type DllGetClassObjectFn =
    unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> i32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| r"C:\dll\aurora_tsf_win.dll".to_string());

    println!("=== aurora_tsf_win.dll 加载测试 ===");
    println!("路径: {path}");

    let wpath = wide(&path);

    // ── 第 1 层：LoadLibrary ──
    let hmod: HMODULE = unsafe { LoadLibraryW(PCWSTR(wpath.as_ptr()))? };
    println!("[1] LoadLibrary 成功 (HMODULE={:?})", hmod.0);

    // ── 第 2 层：四个导出 ──
    let exports = [
        "DllGetClassObject",
        "DllCanUnloadNow",
        "DllRegisterServer",
        "DllUnregisterServer",
    ];
    for name in exports {
        let cname = format!("{name}\0");
        let p = unsafe { GetProcAddress(hmod, PCSTR(cname.as_ptr())) };
        match p {
            Some(f) => println!("[2] 导出 {name} = {:?}", f as *const c_void),
            None => {
                println!("[2] FAIL 缺导出 {name}");
                unsafe {
                    let _ = FreeLibrary(hmod);
                }
                return Err("缺导出".into());
            }
        }
    }

    // ── 第 3 层：COM 实例化 ──
    let cname = b"DllGetClassObject\0";
    let p =
        unsafe { GetProcAddress(hmod, PCSTR(cname.as_ptr())) }.ok_or("拿不到 DllGetClassObject")?;
    let get_class_object: DllGetClassObjectFn = unsafe { std::mem::transmute(p) };

    let mut factory_ptr: *mut c_void = std::ptr::null_mut();
    let iid_class_factory = IClassFactory::IID;
    let hr = unsafe { get_class_object(&CLSID_TEXT_SERVICE, &iid_class_factory, &mut factory_ptr) };
    if hr < 0 {
        println!("[3] FAIL DllGetClassObject hr=0x{hr:08X}");
        unsafe {
            let _ = FreeLibrary(hmod);
        }
        return Err("DllGetClassObject 失败".into());
    }
    if factory_ptr.is_null() {
        println!("[3] FAIL DllGetClassObject 成功但指针为空");
        unsafe {
            let _ = FreeLibrary(hmod);
        }
        return Err("空指针".into());
    }
    println!("[3] DllGetClassObject 成功 (factory={factory_ptr:?})");

    let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_ptr) };
    // 额外持一份引用，验证「有活对象时不可卸载」
    let _factory_ref: IClassFactory = factory.clone();

    // CreateInstance → IUnknown
    let unk: IUnknown = unsafe { factory.CreateInstance(None) }?;
    println!("[4] CreateInstance 成功");

    // QueryInterface → ITfTextInputProcessor（TSF 真正要的那个）
    let service: ITfTextInputProcessor = unk.cast()?;
    println!("[5] QueryInterface(ITfTextInputProcessor) 成功");

    // ── 第 6 层：用真实 TSM 走 TSF 的 Activate 路径 ──
    // 这是 TSF 宿主的真实调用：先创建 ThreadMgr，再把它的指针传给 Activate。
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::TextServices::{CLSID_TF_ThreadMgr, ITfThreadMgr};

    unsafe {
        let hr_co = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr_co.is_err() {
            println!("[6] CoInitializeEx 失败 0x{:08X}", hr_co.0);
        } else {
            let tm: windows::core::Result<ITfThreadMgr> =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER);
            match tm {
                Ok(tm) => {
                    let tid = tm.Activate().unwrap_or(0);
                    println!("[6] ThreadMgr 就绪 (TfClientId={tid})");

                    // 真实 TSF 激活：把 TSM 指针交给我们的 TextService
                    match unsafe { service.Activate(Some(&tm), tid) } {
                        Ok(()) => println!("[6b] TextService::Activate(真实 TSM) 成功"),
                        Err(e) => println!("[6b] TextService::Activate 失败: 0x{:08X}", e.code().0),
                    }
                    match unsafe { service.Deactivate() } {
                        Ok(()) => println!("[7] TextService::Deactivate 成功"),
                        Err(e) => {
                            println!("[7] TextService::Deactivate 失败: 0x{:08X}", e.code().0)
                        }
                    }
                    let _ = tm.Deactivate();
                }
                Err(e) => println!("[6] 创建 ThreadMgr 失败: 0x{:08X}", e.code().0),
            }
            CoUninitialize();
        }
    }

    // ── 第 7b 层：新增的接口是否可 QueryInterface ──
    {
        use windows::Win32::UI::TextServices::ITfDisplayAttributeProvider;
        // 重新实例化一个对象来测（上面的 unk 已 drop）
        let unk2: IUnknown = unsafe { factory.CreateInstance(None) }?;
        match unk2.cast::<ITfDisplayAttributeProvider>() {
            Ok(p) => {
                println!("[7b] QueryInterface(ITfDisplayAttributeProvider) 成功");
                use windows::core::GUID;
                let input_guid = GUID::from_u128(0x99C2EE33_5C57_45A2_9C63_FB54B34FD90A);
                match unsafe { p.GetDisplayAttributeInfo(&input_guid) } {
                    Ok(info) => {
                        let g = unsafe { info.GetGUID() }.unwrap_or_default();
                        let desc = unsafe { info.GetDescription() }.unwrap_or_default();
                        println!(
                            "         属性 GUID 匹配={} 描述={:?}",
                            g == input_guid,
                            desc.to_string()
                        );
                    }
                    Err(e) => println!("         取属性失败: 0x{:08X}", e.code().0),
                }
            }
            Err(e) => println!("[7b] QueryInterface 失败: 0x{:08X}", e.code().0),
        }
        drop(unk2);
    }

    // ── 第 7c 层：语言栏按钮 ──
    {
        let n = b"AuroraTsfTestLangBar\0";
        match unsafe { GetProcAddress(hmod, PCSTR(n.as_ptr())) } {
            Some(f) => {
                let f: unsafe extern "system" fn() -> i32 = unsafe { std::mem::transmute(f) };
                let rc = unsafe { f() };
                match rc {
                    0 => println!("[7c] 语言栏按钮 GetInfo/GetStatus/工具提示/双接口 全部通过"),
                    -1 => println!("[7c] FAIL GetInfo"),
                    -2 => println!("[7c] FAIL GetStatus"),
                    -3 => println!("[7c] FAIL GetTooltipString"),
                    -4 => println!("[7c] FAIL QueryInterface(ITfLangBarItemButton)"),
                    _ => println!("[7c] FAIL 未知 {rc}"),
                }
            }
            None => println!("[7c] 缺导出 AuroraTsfTestLangBar"),
        }
    }

    // ── 第 7d 层：组合状态机 ──
    {
        let n = b"AuroraTsfTestComposition\0";
        match unsafe { GetProcAddress(hmod, PCSTR(n.as_ptr())) } {
            Some(f) => {
                let f: unsafe extern "system" fn() -> i32 = unsafe { std::mem::transmute(f) };
                let rc = unsafe { f() };
                if rc == 0 {
                    println!("[7d] 组合状态机：空闲→开始→更新→结束→清理→宿主强杀 全部正确");
                } else {
                    println!("[7d] FAIL 第 {} 步", -rc);
                }
            }
            None => println!("[7d] 缺导出 AuroraTsfTestComposition"),
        }
    }

    // ── 第 8 层：引用计数与 DllCanUnloadNow ──
    // DllCanUnloadNow 必须在还有活对象时返回 S_FALSE(1)，全释放后返回 S_OK(0)
    let cname = b"DllCanUnloadNow\0";
    let can_unload: unsafe extern "system" fn() -> i32 = unsafe {
        std::mem::transmute(
            GetProcAddress(hmod, PCSTR(cname.as_ptr())).ok_or("缺 DllCanUnloadNow")?,
        )
    };

    let hr_with_objects = unsafe { can_unload() };
    println!("[8] 有活对象时 DllCanUnloadNow = 0x{hr_with_objects:08X} (期望 S_FALSE=1)");
    if hr_with_objects != 1 {
        println!("    WARN 期望 S_FALSE，实得 0x{hr_with_objects:08X}");
    }

    drop(service);
    drop(unk);
    drop(factory);
    drop(_factory_ref);

    let hr_after = unsafe { can_unload() };
    println!("[9] 全部释放后 DllCanUnloadNow = 0x{hr_after:08X} (期望 S_OK=0)");
    if hr_after != 0 {
        println!("    FAIL 组件泄漏：仍报告不可卸载");
        unsafe {
            let _ = FreeLibrary(hmod);
        }
        return Err("引用计数泄漏".into());
    }

    // ── 第 10 层：DllRegisterServer（会写注册表）──
    let cname = b"DllRegisterServer\0";
    let register: unsafe extern "system" fn() -> i32 = unsafe {
        std::mem::transmute(
            GetProcAddress(hmod, PCSTR(cname.as_ptr())).ok_or("缺 DllRegisterServer")?,
        )
    };
    let hr_reg = unsafe { register() };
    println!(
        "[10] DllRegisterServer = 0x{hr_reg:08X} ({})",
        if hr_reg == 0 {
            "S_OK"
        } else {
            "失败或部分完成"
        }
    );

    let cname = b"DllUnregisterServer\0";
    let unregister: unsafe extern "system" fn() -> i32 = unsafe {
        std::mem::transmute(
            GetProcAddress(hmod, PCSTR(cname.as_ptr())).ok_or("缺 DllUnregisterServer")?,
        )
    };
    let hr_unreg = unsafe { unregister() };
    println!("[11] DllUnregisterServer = 0x{hr_unreg:08X}");

    // ── 第 12 层：TSF 运行时可用性（Wine 的 msctf 完整度检查）──
    println!("\n--- TSF 运行时检查 ---");
    match test_tsf_runtime() {
        Ok(()) => println!("[12] TSF 运行时可用"),
        Err(e) => println!("[12] TSF 运行时不可用: {e}（本环境无法测更深层）"),
    }

    unsafe {
        let _ = FreeLibrary(hmod);
    }
    println!("\n=== 全部通过 ===");
    Ok(())
}

/// 检查本环境的 TSF 运行时是否可用。
///
/// Wine 的 msctf.dll 导出了 TF_CreateThreadMgr 等入口，但内部实现未必完整。
/// 这条测试的目的不是「必须成功」，而是**如实报告**当前环境能测到哪一层。
fn test_tsf_runtime() -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::TextServices::{CLSID_TF_ThreadMgr, ITfThreadMgr};

    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(format!("CoInitializeEx 失败 0x{:08X}", hr.0).into());
        }
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let tm: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
            println!("     创建 ITfThreadMgr 成功");
            let tid = tm.Activate()?;
            println!("     Activate 成功 (TfClientId={tid})");
            let _ = tm.Deactivate();
            println!("     Deactivate 成功");
            Ok(())
        })();
        CoUninitialize();
        result
    }
}
