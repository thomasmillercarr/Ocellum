# M1 gate. Exit 0 = pass.
# Unit suites (behaviour distributions, contract validation, renderer
# compositing) + a live pixel check that the placeholder character renders.
param(
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

$failures = @()
function Check([string]$name, [bool]$ok) {
    if ($ok) { Write-Host "PASS: $name" } else { Write-Host "FAIL: $name"; $script:failures += $name }
}

Write-Host "--- TS unit tests (blink machine, Poisson distribution, decorrelation, roll transform, contract, renderer) ---"
cmd /c "npx vitest run > NUL 2>&1"
Check "vitest suite (21 tests)" ($LASTEXITCODE -eq 0)

Write-Host "--- Rust unit tests (character dir loader) ---"
Push-Location src-tauri
cmd /c "cargo test > NUL 2>&1"
Check "cargo test" ($LASTEXITCODE -eq 0)
Pop-Location

if (-not $SkipBuild) {
    Write-Host "--- Release build ---"
    cmd /c "npx tauri build --no-bundle > NUL 2>&1"
    Check "release build" ($LASTEXITCODE -eq 0)
}

# --- Live render check: the placeholder ball must actually appear on screen ---
Add-Type -AssemblyName System.Drawing
function Send-Control([string]$cmd) {
    $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", 47613)
    $s = $client.GetStream()
    $w = New-Object System.IO.StreamWriter($s); $w.WriteLine($cmd); $w.Flush()
    $r = New-Object System.IO.StreamReader($s); $line = $r.ReadLine(); $client.Close()
    return $line | ConvertFrom-Json
}

$env:OCELLUM_TEST = "1"
$proc = Start-Process -FilePath (Join-Path $root "src-tauri\target\release\ocellum.exe") -PassThru
try {
    $up = $false
    foreach ($i in 1..50) {
        Start-Sleep -Milliseconds 200
        try { $null = Send-Control "hwnd"; $up = $true; break } catch {}
    }
    Check "app launches" $up
    if ($up) {
        Start-Sleep -Milliseconds 1500  # let SVG layers decode + first frames render
        $rects = (Send-Control "rects").rects
        Check "hit regions reported (frontend JS alive)" ($rects.Count -ge 1)
        $pet = $rects[0]
        $cx = [int]($pet.x + $pet.w / 2); $cy = [int]($pet.y + $pet.h / 2)
        $bmp = New-Object System.Drawing.Bitmap(40, 40)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $g.CopyFromScreen($cx - 20, $cy - 20, 0, 0, $bmp.Size)
        $g.Dispose()
        # The ball body is a blue gradient (#7ec8e3 → #215a75): look for
        # clearly blue-dominant pixels in the pet centre.
        $blue = 0
        foreach ($x in 0..39) {
            foreach ($y in 0..39) {
                $p = $bmp.GetPixel($x, $y)
                if ($p.B -gt 100 -and $p.B -gt ($p.R + 30) -and $p.G -lt $p.B) { $blue++ }
            }
        }
        $bmp.Dispose()
        Check "placeholder character composited on screen ($blue/1600 blue px)" ($blue -gt 400)
    }
} finally {
    if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force }
    Remove-Item Env:\OCELLUM_TEST -ErrorAction SilentlyContinue
}

Write-Host ""
if ($failures.Count -eq 0) { Write-Host "M1 GATE: PASS"; exit 0 }
Write-Host "M1 GATE: FAIL ($($failures.Count)): $($failures -join '; ')"
exit 1
