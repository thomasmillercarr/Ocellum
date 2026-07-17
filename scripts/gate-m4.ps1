# M4 gate. Exit 0 = pass.
# Mood is a function of local data (§8.4): flat after 14 quiet days, positive
# delta on a fresh draft, no-brows characters degrade gracefully, and mood is
# derived from lead/interaction — never stored, never settable.
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

Write-Host "--- Rust unit tests (14-days-flat, fresh-draft delta, restless, neutral) ---"
Push-Location src-tauri
cmd /c "cargo test mood > NUL 2>&1"
Check "cargo test mood" ($LASTEXITCODE -eq 0)
Pop-Location

Write-Host "--- Frontend tests (brow mapping, behaviour modulation, no-brows degradation) ---"
cmd /c "npx vitest run src/mood.test.ts > NUL 2>&1"
Check "vitest mood.test.ts" ($LASTEXITCODE -eq 0)

Write-Host "--- Static: mood is derived, never stored or settable ---"
# The only 'mood' in the schema is the mood_event journal — no mood column on
# lead/interaction, no stored mood state.
$schemaMood = (Get-Content src-tauri\src\db.rs | Select-String -Pattern "mood" |
    Where-Object { $_.Line -notmatch "mood_event" })
Check "no mood column in the schema outside mood_event" ($null -eq $schemaMood)
# No code path sets a mood: no set_mood command, no UPDATE/DELETE on the journal.
$setters = Get-ChildItem src, src-tauri\src -Recurse -File -Include *.rs, *.ts |
    Select-String -Pattern "set_mood|set-mood" -List
Check "no set-mood command anywhere" ($null -eq $setters)
$mutations = Select-String -Path src-tauri\src\mood.rs -Pattern "UPDATE|DELETE" -List
Check "mood_event journal is append-only" ($null -eq $mutations)

if (-not $SkipBuild) {
    Write-Host "--- Release build ---"
    cmd /c "npx tauri build --no-bundle > NUL 2>&1"
    Check "release build" ($LASTEXITCODE -eq 0)
    Push-Location src-tauri
    cmd /c "cargo build --release --bin mock_llm > NUL 2>&1"
    Check "mock_llm build" ($LASTEXITCODE -eq 0)
    Pop-Location
}

$savedClipboard = ""
try { $savedClipboard = Get-Clipboard -Raw -ErrorAction SilentlyContinue } catch {}

$dbPath = Join-Path $env:TEMP "ocellum-gate-m4.db"
Remove-Item $dbPath -ErrorAction SilentlyContinue
$env:OCELLUM_TEST = "1"
$env:OCELLUM_DB_PATH = $dbPath
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
        # Fresh install: nothing to be flat about yet.
        $m0 = Send-Control "mood"
        Check "fresh DB mood is neutral ($($m0.mood))" ($m0.mood -eq "neutral" -and $m0.events -eq 0)

        $null = Send-Control "set-key anthropic OCELLUM-M4-KEY"
        $null = Send-Control "set-setting provider anthropic"
        $null = Send-Control "set-setting anthropic_url http://127.0.0.1:47700"
        $null = Send-Control "set-setting anthropic_model claude-opus-4-8"

        # A real draft through the app flips mood to bright and journals a delta.
        $lead = Send-Control "capture Jane Doe\njane.doe@acme.example\nAcme Ltd"
        $null = Send-Control "draft $($lead.id)"
        $m1 = Send-Control "mood"
        Check "fresh draft -> mood bright ($($m1.mood))" ($m1.mood -eq "bright")
        Check "fresh draft -> mood_event row written ($($m1.events))" ($m1.events -ge 1)

        $null = Send-Control "delete-key anthropic"
    }
} finally {
    foreach ($p in @($proc, $mock)) {
        if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force }
    }
    Remove-Item Env:\OCELLUM_TEST, Env:\OCELLUM_DB_PATH -ErrorAction SilentlyContinue
    Remove-Item $dbPath -ErrorAction SilentlyContinue
    if ($savedClipboard) { try { Set-Clipboard -Value $savedClipboard } catch {} }
}

Write-Host ""
if ($failures.Count -eq 0) { Write-Host "M4 GATE: PASS"; exit 0 }
Write-Host "M4 GATE: FAIL ($($failures.Count)): $($failures -join '; ')"
exit 1
