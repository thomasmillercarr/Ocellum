# M2 gate. Exit 0 = pass.
# Unit suites (one interface across providers, --disallowedTools lockdown,
# cost to the penny, spend cap, egress parity, claude-code budget shape) +
# real Claude Code call + real Credential Manager roundtrip + end-to-end
# streaming through the live UI against a local mock, with a no-key-on-disk
# scan.
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
function Send-Control([string]$cmd) {
    $client = New-Object System.Net.Sockets.TcpClient("127.0.0.1", 47613)
    $s = $client.GetStream()
    $w = New-Object System.IO.StreamWriter($s); $w.WriteLine($cmd); $w.Flush()
    $r = New-Object System.IO.StreamReader($s); $line = $r.ReadLine(); $client.Close()
    return $line | ConvertFrom-Json
}

Push-Location src-tauri
Write-Host "--- Rust unit tests (providers via one interface, lockdown args, cost, cap, egress parity, budget shapes) ---"
cmd /c "cargo test > NUL 2>&1"
Check "cargo test" ($LASTEXITCODE -eq 0)

Write-Host "--- Real integrations: Claude Code detected+auth+used; Credential Manager roundtrip ---"
cmd /c "cargo test -- --ignored > NUL 2>&1"
Check "cargo test --ignored (real claude CLI + real keychain)" ($LASTEXITCODE -eq 0)

Write-Host "--- Claude Code on PATH ---"
$claudePath = (where.exe claude 2>$null | Select-Object -First 1)
Check "claude binary detected on PATH ($claudePath)" (-not [string]::IsNullOrEmpty($claudePath))
Pop-Location

if (-not $SkipBuild) {
    Write-Host "--- Release build (app + mock_llm) ---"
    cmd /c "npx tauri build --no-bundle > NUL 2>&1"
    Check "release build" ($LASTEXITCODE -eq 0)
    Push-Location src-tauri
    cmd /c "cargo build --release --bin mock_llm > NUL 2>&1"
    Check "mock_llm build" ($LASTEXITCODE -eq 0)
    Pop-Location
}

# --- End-to-end: streaming chat in the live UI, keys in keychain only ---
$secret = "OCELLUM-GATE-SECRET-77aa"
$dbPath = Join-Path $env:TEMP "ocellum-gate-m2.db"
Remove-Item $dbPath -ErrorAction SilentlyContinue
$env:OCELLUM_TEST = "1"
$env:OCELLUM_DB_PATH = $dbPath
$t0 = Get-Date
$mock = Start-Process -FilePath (Join-Path $root "src-tauri\target\release\mock_llm.exe") -PassThru -WindowStyle Hidden
$proc = Start-Process -FilePath (Join-Path $root "src-tauri\target\release\ocellum.exe") -PassThru
try {
    $up = $false
    foreach ($i in 1..50) {
        Start-Sleep -Milliseconds 200
        try { $null = Send-Control "hwnd"; $up = $true; break } catch {}
    }
    Check "app launches" $up
    if ($up) {
        $null = Send-Control "set-key anthropic $secret"
        $null = Send-Control "set-setting provider anthropic"
        $null = Send-Control "set-setting anthropic_url http://127.0.0.1:47700"
        $null = Send-Control "set-setting anthropic_model claude-opus-4-8"

        # Fixed sleeps flake under load (builds, OneDrive sync) — poll for the
        # expected state instead; assertions are unchanged.
        $null = Send-Control "open-bubble"
        Start-Sleep -Milliseconds 800
        (New-Object -ComObject WScript.Shell).SendKeys("ping{ENTER}")
        $last = ""
        foreach ($i in 1..50) {
            Start-Sleep -Milliseconds 200
            $last = (Send-Control "chat-log").last
            if ($last -eq "Hello from the mock") { break }
        }
        Check "streaming conversation through live bubble ('$last')" ($last -eq "Hello from the mock")

        $egress = (Send-Control "egress-count").count
        Check "egress row per model call (1 call = $egress row)" ($egress -eq 1)

        # Second call -> second row (count parity, not a lucky constant).
        # Re-assert focus first: anything grabbing foreground between the two
        # messages would send the keystrokes elsewhere.
        $null = Send-Control "open-bubble"
        Start-Sleep -Milliseconds 400
        (New-Object -ComObject WScript.Shell).SendKeys("again{ENTER}")
        $egress2 = 0
        foreach ($i in 1..50) {
            Start-Sleep -Milliseconds 200
            $egress2 = (Send-Control "egress-count").count
            if ($egress2 -ge 2) { break }
        }
        Check "egress parity holds across calls (2 calls = $egress2 rows)" ($egress2 -eq 2)

        # --- No key material on disk ---
        Start-Sleep -Milliseconds 500
        $touched = @(Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue |
            Where-Object { $_.LastWriteTime -gt $t0 -and $_.FullName -notmatch '\\(node_modules|\.git)\\' })
        $touched += Get-Item $dbPath -ErrorAction SilentlyContinue
        foreach ($dir in @("$env:APPDATA\ocellum", "$env:LOCALAPPDATA\ocellum", "$env:APPDATA\app.ocellum.desktop", "$env:LOCALAPPDATA\app.ocellum.desktop")) {
            if (Test-Path $dir) { $touched += Get-ChildItem $dir -Recurse -File -ErrorAction SilentlyContinue }
        }
        $hits = @($touched | Select-String -Pattern $secret -List -ErrorAction SilentlyContinue)
        Check "API key appears in no file on disk (scanned $($touched.Count) files)" ($hits.Count -eq 0)
        if ($hits.Count -gt 0) { $hits | ForEach-Object { Write-Host "  LEAKED IN: $($_.Path)" } }

        $null = Send-Control "delete-key anthropic"
    }
} finally {
    foreach ($p in @($proc, $mock)) {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    }
    Remove-Item Env:\OCELLUM_TEST, Env:\OCELLUM_DB_PATH -ErrorAction SilentlyContinue
    Remove-Item $dbPath -ErrorAction SilentlyContinue
}

Write-Host ""
if ($failures.Count -eq 0) { Write-Host "M2 GATE: PASS"; exit 0 }
Write-Host "M2 GATE: FAIL ($($failures.Count)): $($failures -join '; ')"
exit 1
