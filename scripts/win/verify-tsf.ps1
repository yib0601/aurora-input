<#
.SYNOPSIS
    验证 Rust 版 TSF DLL 能否在 Windows 上被系统加载、注册、激活。

.DESCRIPTION
    三层验证，逐层加严。任一层失败即停止并报告——不要往下走，
    下层失败的真实原因往往就是上层没通过。

    第 0 层  文件与依赖      DLL 存在、位数对、无缺失的运行库依赖
    第 1 层  可加载          LoadLibrary 成功（未注册也能测）
    第 2 层  可导出/可实例化  DllGetClassObject 拿到工厂并 CreateInstance
    第 3 层  可注册/可激活    regsvr32 成功、系统输入法列表里能看到

.PARAMETER DllPath
    待测 DLL 路径。默认在当前仓库的构建产物里找。

.PARAMETER SkipRegister
    跳过第 3 层（不写注册表）。用于反复调试前两层。

.EXAMPLE
    .\scripts\win\verify-tsf.ps1
    .\scripts\win\verify-tsf.ps1 -DllPath C:\build\aurora_tsf_win.dll
#>
[CmdletBinding()]
param(
    [string]$DllPath,
    [switch]$SkipRegister
)

$ErrorActionPreference = 'Stop'
$script:failures = 0

function Step($n, $msg) { Write-Host "`n[$n] $msg" -ForegroundColor Cyan }
function Pass($msg)     { Write-Host "  PASS  $msg" -ForegroundColor Green }
function Fail($msg)     { Write-Host "  FAIL  $msg" -ForegroundColor Red; $script:failures++ }
function Info($msg)     { Write-Host "  ..    $msg" -ForegroundColor DarkGray }

# ── 定位 DLL ────────────────────────────────────────────────────────────
if (-not $DllPath) {
    $candidates = @(
        "$PSScriptRoot\..\..\aurora_core\target\x86_64-pc-windows-msvc\release\aurora_tsf_win.dll"
        "$PSScriptRoot\..\..\build\aurora_tsf_win.dll"
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { $DllPath = (Resolve-Path $c).Path; break }
    }
}
if (-not $DllPath -or -not (Test-Path $DllPath)) {
    Write-Host "找不到 DLL。用 -DllPath 指定。" -ForegroundColor Red
    exit 2
}

Write-Host "`n=== TSF DLL 验证 ===" -ForegroundColor White
Write-Host "DLL: $DllPath"
Write-Host "OS : $([System.Environment]::OSVersion.VersionString)"
Write-Host "PS : $($PSVersionTable.PSVersion)"

# ── 第 0 层：文件与依赖 ─────────────────────────────────────────────────
Step "0/3" "文件与依赖"

$dll = Get-Item $DllPath
Info "大小: $([math]::Round($dll.Length/1KB)) KB"

# 位数必须与当前进程一致（TSF 宿主加载同位数 DLL）
try {
    $bytes = [System.IO.File]::ReadAllBytes($DllPath)[0..1]
    $machine = [BitConverter]::ToUInt16($bytes, 0)
    if ($machine -eq 0x8664) { $arch = 'x64' }
    elseif ($machine -eq 0x014c) { $arch = 'x86' }
    else { $arch = "unknown($machine)" }
    $procArch = if ([IntPtr]::Size -eq 8) { 'x64' } else { 'x86' }
    if ($arch -eq $procArch) {
        Pass "位数匹配：DLL=$arch 进程=$procArch"
    } else {
        Fail "位数不匹配：DLL=$arch 进程=$procArch（TSF 要求一致）"
    }
} catch {
    Fail "读取 PE 头失败: $_"
}

# 检查运行库依赖。
# 关键判据：不能依赖 VCRUNTIME140 / MSVCP140（那说明没走 /MT 静态 CRT）。
# 全新装的 Windows 没有 dumpbin，故自带一个纯字符串扫描的兜底——
# PE 的导入表里 DLL 名是 ASCII 明文，直接扫即可，不需要完整解析。
$dllBytes = [System.IO.File]::ReadAllBytes($DllPath)
$dllText = [System.Text.Encoding]::ASCII.GetString($dllBytes)
$found = @()
foreach ($pat in @('VCRUNTIME140','MSVCP140','MSVCR1','ucrtbase','api-ms-win-crt')) {
    if ($dllText.Contains($pat)) { $found += $pat }
}
if ($found.Count -gt 0) {
    Fail "依赖了 VC++ / UCRT 运行库（未静态链接 CRT）: $($found -join ', ')"
    Info "目标机没装运行库时 DLL 加载失败。构建时加 RUSTFLAGS='-C target-feature=+crt-static'"
} else {
    Pass "无 VC++ / UCRT 运行库依赖（静态 CRT 生效）"
}

# 列出它依赖的系统 DLL（供人工核对）
$imported = [regex]::Matches($dllText, '[A-Za-z0-9_.-]+\.dll') |
    ForEach-Object { $_.Value.ToLower() } | Sort-Object -Unique
Info "导入的系统库: $($imported -join ', ')"

# ── 第 1 层：可加载 ─────────────────────────────────────────────────────
Step "1/3" "可加载（LoadLibrary / FreeLibrary）"

$sig = @'
using System;
using System.Runtime.InteropServices;
public static class N {
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    public static extern IntPtr LoadLibraryW(string p);
    [DllImport("kernel32.dll", SetLastError=true)]
    public static extern bool FreeLibrary(IntPtr h);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Ansi)]
    public static extern IntPtr GetProcAddress(IntPtr h, string n);
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    public static extern IntPtr GetModuleHandleW(string p);
}
'@
if (-not ('N' -as [type])) { Add-Type -TypeDefinition $sig }

$h = [N]::LoadLibraryW($DllPath)
if ($h -eq [IntPtr]::Zero) {
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    Fail "LoadLibrary 失败，Win32 错误码 $err —— $([ComponentModel.Win32Exception]::new($err).Message)"
    Write-Host "`n后续层全部跳过。" -ForegroundColor Yellow
    exit 1
}
Pass "LoadLibrary 成功 (handle=$h)"

# ── 第 2 层：可导出 / 可实例化 ──────────────────────────────────────────
Step "2/3" "导出函数与 COM 实例化"

foreach ($fn in @('DllGetClassObject','DllCanUnloadNow','DllRegisterServer','DllUnregisterServer')) {
    $p = [N]::GetProcAddress($h, $fn)
    if ($p -eq [IntPtr]::Zero) { Fail "缺导出 $fn" } else { Pass "导出 $fn 存在" }
}

# 真正走一遍 DllGetClassObject → CreateInstance → QueryInterface
$clsid = [Guid]'99C2EE30-5C57-45A2-9C63-FB54B34FD90A'
$com = @'
using System;
using System.Runtime.InteropServices;
public static class C {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)]
    public static extern IntPtr GetModuleHandleW(string p);
    [DllImport("kernel32.dll", CharSet=CharSet.Ansi)]
    public static extern IntPtr GetProcAddress(IntPtr h, string n);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int DllGetClassObjectFn(ref Guid rclsid, ref Guid riid, out IntPtr ppv);

    // IClassFactory vtable 顺序: QueryInterface(0) AddRef(1) Release(2) CreateInstance(3) LockServer(4)
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int CreateInstanceFn(IntPtr self, IntPtr outer, ref Guid riid, out IntPtr ppv);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate uint ReleaseFn(IntPtr self);

    public static string Run(IntPtr hMod, Guid clsid) {
        var p = GetProcAddress(hMod, "DllGetClassObject");
        if (p == IntPtr.Zero) return "缺 DllGetClassObject";
        var fn = Marshal.GetDelegateForFunctionPointer<DllGetClassObjectFn>(p);

        Guid iidClassFactory = new Guid("00000001-0000-0000-C000-000000000046");
        int hr = fn(ref clsid, ref iidClassFactory, out IntPtr factory);
        if (hr != 0) return $"DllGetClassObject 失败 hr=0x{hr:X8}";
        if (factory == IntPtr.Zero) return "DllGetClassObject 返回成功但指针为空";

        try {
            IntPtr vtbl = Marshal.ReadIntPtr(factory);
            IntPtr createPtr = Marshal.ReadIntPtr(vtbl, 3 * IntPtr.Size);
            var create = Marshal.GetDelegateForFunctionPointer<CreateInstanceFn>(createPtr);

            // IID_IUnknown
            Guid iidUnknown = new Guid("00000000-0000-0000-C000-000000000046");
            hr = create(factory, IntPtr.Zero, ref iidUnknown, out IntPtr obj);
            if (hr != 0) return $"CreateInstance 失败 hr=0x{hr:X8}";
            if (obj == IntPtr.Zero) return "CreateInstance 返回成功但对象为空";

            // 释放对象
            IntPtr objVtbl = Marshal.ReadIntPtr(obj);
            IntPtr relPtr = Marshal.ReadIntPtr(objVtbl, 2 * IntPtr.Size);
            var rel = Marshal.GetDelegateForFunctionPointer<ReleaseFn>(relPtr);
            rel(obj);

            return null; // null = 成功
        } finally {
            IntPtr fVtbl = Marshal.ReadIntPtr(factory);
            IntPtr fRelPtr = Marshal.ReadIntPtr(fVtbl, 2 * IntPtr.Size);
            var fRel = Marshal.GetDelegateForFunctionPointer<ReleaseFn>(fRelPtr);
            fRel(factory);
        }
    }
}
'@
if (-not ('C' -as [type])) { Add-Type -TypeDefinition $com }

try {
    $err = [C]::Run($h, $clsid)
    if ($err) { Fail "COM 实例化: $err" } else { Pass "DllGetClassObject → CreateInstance → Release 全通" }
} catch {
    Fail "COM 实例化抛异常: $_"
}

[N]::FreeLibrary($h) | Out-Null
Info "已 FreeLibrary"

# ── 第 3 层：可注册 / 可激活 ────────────────────────────────────────────
if ($SkipRegister) {
    Step "3/3" "注册与激活（已跳过 -SkipRegister）"
} else {
    Step "3/3" "注册到系统"

    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $isAdmin) {
        Info "非管理员，跳过 regsvr32（注册 HKCR 需要提权）"
    } else {
        $proc = Start-Process regsvr32.exe -ArgumentList "/s", "`"$DllPath`"" -Wait -PassThru
        if ($proc.ExitCode -eq 0) {
            Pass "regsvr32 /s 成功"
        } else {
            Fail "regsvr32 退出码 $($proc.ExitCode)"
        }

        # 核对注册表三项
        $clsidKey = "Registry::HKEY_CLASSES_ROOT\CLSID\{99C2EE30-5C57-45A2-9C63-FB54B34FD90A}"
        if (Test-Path $clsidKey) { Pass "HKCR\CLSID 键已写入" } else { Fail "HKCR\CLSID 键缺失" }

        $inproc = "$clsidKey\InprocServer32"
        if (Test-Path $inproc) {
            $v = (Get-ItemProperty $inproc).'(default)'
            if ($v -eq $DllPath) { Pass "InprocServer32 指向本 DLL" }
            else { Fail "InprocServer32 指向 $v（期望 $DllPath）" }
            $tm = (Get-ItemProperty $inproc).ThreadingModel
            if ($tm -eq 'Apartment') { Pass "ThreadingModel = Apartment" }
            else { Fail "ThreadingModel = $tm（期望 Apartment）" }
        } else { Fail "InprocServer32 子键缺失" }

        # TSF 输入法列表（Profile 注册的落点）
        $tip = "Registry::HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\CTF\TIP\{99C2EE30-5C57-45A2-9C63-FB54B34FD90A}"
        if (Test-Path $tip) { Pass "CTF\TIP 键已写入（输入法已登记）" }
        else { Info "CTF\TIP 未找到——Profile 可能注册到 HKCU，或需要重新登录才生效" }
    }
}

# ── 结果 ────────────────────────────────────────────────────────────────
Write-Host ""
if ($script:failures -eq 0) {
    Write-Host "=== 全部通过 ===" -ForegroundColor Green
    exit 0
} else {
    Write-Host "=== $($script:failures) 项失败 ===" -ForegroundColor Red
    exit 1
}
