# M0 gate. Exit 0 = pass.
# Prereq: release exe built (npx tauri build --no-bundle).
# Drives the app via the OCELLUM_TEST control channel + real OS input APIs.
param(
    [switch]$SkipDisplayTest
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root "src-tauri\target\release\ocellum.exe"
if (-not (Test-Path $exe)) { Write-Host "FAIL: build $exe first (npx tauri build --no-bundle)"; exit 1 }

Add-Type -Namespace Gate -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
[DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(System.Drawing.Point p);
[DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
[DllImport("user32.dll", CharSet=CharSet.Auto)] public static extern int ChangeDisplaySettings(ref DEVMODE devMode, int flags);
[DllImport("user32.dll", CharSet=CharSet.Auto)] public static extern int ChangeDisplaySettingsNull(IntPtr devMode, int flags);
[DllImport("user32.dll", CharSet=CharSet.Auto)] public static extern bool EnumDisplaySettings(string deviceName, int modeNum, ref DEVMODE devMode);
[StructLayout(LayoutKind.Sequential, CharSet=CharSet.Auto)]
public struct DEVMODE {
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmDeviceName;
    public short dmSpecVersion, dmDriverVersion, dmSize, dmDriverExtra;
    public int dmFields, dmPositionX, dmPositionY, dmDisplayOrientation, dmDisplayFixedOutput;
    public short dmColor, dmDuplex, dmYResolution, dmTTOption, dmCollate;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst=32)] public string dmFormName;
    public short dmLogPixels;
    public int dmBitsPerPel, dmPelsWidth, dmPelsHeight, dmDisplayFlags, dmDisplayFrequency;
    public int dmICMMethod, dmICMIntent, dmMediaType, dmDitherType, dmReserved1, dmReserved2, dmPanningWidth, dmPanningHeight;
}
'@ -ReferencedAssemblies System.Drawing

function Send-Control([string]$cmd) {
    $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", 47613)
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $writer.WriteLine($cmd); $writer.Flush()
    $reader = New-Object System.IO.StreamReader($stream)
    $line = $reader.ReadLine()
    $client.Close()
    return $line | ConvertFrom-Json
}

$failures = @()
function Check([string]$name, [bool]$ok) {
    if ($ok) { Write-Host "PASS: $name" } else { Write-Host "FAIL: $name"; $script:failures += $name }
}

# --- Launch ---
$env:OCELLUM_TEST = "1"
$proc = Start-Process -FilePath $exe -PassThru
try {
    $up = $false
    foreach ($i in 1..50) {
        Start-Sleep -Milliseconds 200
        try { $null = Send-Control "hwnd"; $up = $true; break } catch {}
    }
    Check "app launches and control channel responds" $up
    if (-not $up) { exit 1 }
    Check "app process alive (tray/setup did not fail)" (-not $proc.HasExited)

    $hwnd = [IntPtr](Send-Control "hwnd").hwnd
    $rects = (Send-Control "rects").rects
    Check "frontend reported hit regions" ($rects.Count -ge 1)
    $pet = $rects[0]

    # --- Click-through outside hit region ---
    # A point inside the window but outside every hit rect: window top-left area.
    $outX = [int]($pet.x - 150); $outY = [int]($pet.y - 300)
    [Gate.Native]::SetCursorPos($outX, $outY) | Out-Null
    Start-Sleep -Milliseconds 250
    $ignoring = (Send-Control "ignore-state").ignoring
    Check "cursor outside hit region -> click-through enabled" ($ignoring -eq $true)
    $pt = New-Object System.Drawing.Point($outX, $outY)
    $under = [Gate.Native]::GetAncestor([Gate.Native]::WindowFromPoint($pt), 2)
    Check "OS hit-test outside hit region resolves to window below" ($under -ne $hwnd)

    # --- Click inside hit region reaches Ocellum ---
    $cx = [int]($pet.x + $pet.w / 2); $cy = [int]($pet.y + $pet.h / 2)
    [Gate.Native]::SetCursorPos($cx, $cy) | Out-Null
    Start-Sleep -Milliseconds 250
    $ignoring = (Send-Control "ignore-state").ignoring
    Check "cursor over pet -> window interactive" ($ignoring -eq $false)
    $pt2 = New-Object System.Drawing.Point($cx, $cy)
    $under2 = [Gate.Native]::GetAncestor([Gate.Native]::WindowFromPoint($pt2), 2)
    Check "OS hit-test over pet resolves to Ocellum" ($under2 -eq $hwnd)

    # --- Bubble open, keystrokes, close, focus return ---
    $null = Send-Control "open-bubble"
    Start-Sleep -Milliseconds 500
    Check "bubble opens" ((Send-Control "bubble-open").open -eq $true)
    Check "bubble open -> window interactive" ((Send-Control "ignore-state").ignoring -eq $false)
    $fg = [Gate.Native]::GetForegroundWindow()
    Check "bubble open -> Ocellum has focus" ($fg -eq $hwnd)
    (New-Object -ComObject WScript.Shell).SendKeys("hello gate")
    Start-Sleep -Milliseconds 500
    $val = (Send-Control "input-value").value
    Check "text input receives keystrokes ('$val')" ($val -eq "hello gate")
    $null = Send-Control "close-bubble"
    Start-Sleep -Milliseconds 500
    Check "bubble closes" ((Send-Control "bubble-open").open -eq $false)
    $fgAfter = [Gate.Native]::GetForegroundWindow()
    Check "focus returned to previous window" ($fgAfter -ne $hwnd)

    # --- Display resolution change survival ---
    if (-not $SkipDisplayTest) {
        $dm = New-Object Gate.Native+DEVMODE
        $dm.dmSize = [System.Runtime.InteropServices.Marshal]::SizeOf($dm)
        [Gate.Native]::EnumDisplaySettings($null, -1, [ref]$dm) | Out-Null
        $origW = $dm.dmPelsWidth; $origH = $dm.dmPelsHeight
        # Find a different supported resolution
        $alt = $null; $i = 0
        $probe = New-Object Gate.Native+DEVMODE
        $probe.dmSize = $dm.dmSize
        while ([Gate.Native]::EnumDisplaySettings($null, $i, [ref]$probe)) {
            if ($probe.dmPelsWidth -ne $origW -and $probe.dmPelsWidth -ge 1280 -and $probe.dmDisplayFrequency -eq $dm.dmDisplayFrequency -and $probe.dmBitsPerPel -eq $dm.dmBitsPerPel) { $alt = $probe; break }
            $i++
        }
        if ($null -eq $alt) {
            Write-Host "SKIP: no alternate resolution available for display test"
        } else {
            try {
                $r = [Gate.Native]::ChangeDisplaySettings([ref]$alt, 0)
                Start-Sleep -Seconds 2
                $alive = -not $proc.HasExited
                $respond = $false
                try { $null = Send-Control "hwnd"; $respond = $true } catch {}
                Check "survives resolution change (alive+responsive, CDS=$r)" ($alive -and $respond)
            } finally {
                $r2 = [Gate.Native]::ChangeDisplaySettings([ref]$dm, 0)
                Start-Sleep -Seconds 2
            }
            $respond2 = $false
            try { $null = Send-Control "hwnd"; $respond2 = $true } catch {}
            Check "survives resolution restore" ((-not $proc.HasExited) -and $respond2)
        }
    } else {
        Write-Host "SKIP: display test skipped by flag"
    }
} finally {
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Remove-Item Env:\OCELLUM_TEST -ErrorAction SilentlyContinue
}

Write-Host ""
if ($failures.Count -eq 0) { Write-Host "M0 GATE: PASS"; exit 0 }
Write-Host "M0 GATE: FAIL ($($failures.Count)): $($failures -join '; ')"
exit 1
