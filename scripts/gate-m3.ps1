# M3 gate. Exit 0 = pass.
# End-to-end core loop against a mock provider: capture -> enrich -> draft
# (to clipboard) -> reminder fires. Plus: clipboard monitoring default-off,
# three-dismissals rule, hard silence, no SMTP/mail egress, cold-start timing.
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

Write-Host "--- Rust unit tests (capture parsing, dismissal rule, silence, monitor default, reminders) ---"
Push-Location src-tauri
cmd /c "cargo test > NUL 2>&1"
Check "cargo test" ($LASTEXITCODE -eq 0)
Pop-Location

Write-Host "--- Static: no SMTP / mail-sending code path exists ---"
$srcHits = Get-ChildItem src, src-tauri\src -Recurse -File |
    Select-String -Pattern "smtp|lettre|sendmail|mail_send|mailgun|sendgrid" -List
Check "no SMTP/mail-API references in source" ($srcHits.Count -eq 0)
$lockHits = Select-String -Path src-tauri\Cargo.lock -Pattern '"lettre"|"smtp' -List
Check "no mail crates in dependency lock" ($null -eq $lockHits)

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

$dbPath = Join-Path $env:TEMP "ocellum-gate-m3.db"
Remove-Item $dbPath -ErrorAction SilentlyContinue
$env:OCELLUM_TEST = "1"
$env:OCELLUM_DB_PATH = $dbPath
$mock = Start-Process -FilePath (Join-Path $root "src-tauri\target\release\mock_llm.exe") -PassThru -WindowStyle Hidden
$launchStart = Get-Date
$proc = Start-Process -FilePath (Join-Path $root "src-tauri\target\release\ocellum.exe") -PassThru
try {
    $up = $false
    foreach ($i in 1..50) {
        Start-Sleep -Milliseconds 200
        try { $null = Send-Control "hwnd"; $up = $true; break } catch {}
    }
    Check "app launches" $up
    if ($up) {
        # Clipboard monitoring must be OFF on a first run (fresh DB).
        $monitor = (Send-Control "get-setting clipboard_monitor").value
        Check "clipboard monitoring off on first run" ($monitor -eq "0")

        $null = Send-Control "set-key anthropic OCELLUM-M3-KEY"
        $null = Send-Control "set-setting provider anthropic"
        $null = Send-Control "set-setting anthropic_url http://127.0.0.1:47700"
        $null = Send-Control "set-setting anthropic_model claude-opus-4-8"

        # --- The loop: text in -> lead -> enrich -> draft -> remind -> fires ---
        $lead = Send-Control "capture Jane Doe\nHead of Ops\njane.doe@acme.example\nAcme Ltd"
        Check "lead created from raw text (id $($lead.id))" ($lead.id -ge 1)

        $enrich = Send-Control "enrich $($lead.id)"
        Check "lead enriched via provider path" ($enrich.notes -eq "Hello from the mock")

        $draft = Send-Control "draft $($lead.id)"
        Check "draft produced" ($draft.text -eq "Hello from the mock")
        Start-Sleep -Milliseconds 300
        $clip = ""
        try { $clip = Get-Clipboard -Raw } catch {}
        Check "draft written to clipboard" ($clip.Trim() -eq "Hello from the mock")
        $draftSecs = [math]::Round(((Get-Date) - $launchStart).TotalSeconds, 1)
        Write-Host "  (cold start -> draft on clipboard: $draftSecs s)"
        Check "cold start to first draft under 3 minutes ($draftSecs s)" ($draftSecs -lt 180)

        $rows = Send-Control "lead-rows"
        Check "rows: lead/enrichment/draft persisted ($($rows.leads)/$($rows.enrichments)/$($rows.drafts))" `
            ($rows.leads -eq 1 -and $rows.enrichments -eq 1 -and $rows.drafts -eq 1)

        $rem = Send-Control "remind $($lead.id) 2"
        Check "reminder scheduled (id $($rem.reminder_id))" ($rem.reminder_id -ge 1)
        Start-Sleep -Seconds 6   # due in 2s, scanner ticks every 2s
        $state = (Send-Control "reminder-state $($rem.reminder_id)").state
        Check "reminder fired ($state)" ($state -eq "fired")
        $surfaces = Send-Control "surfaces"
        Check "reminder surfaced with evidence" ($surfaces.surfaces.reminder -ge 1)

        # --- No SMTP / mail egress: everything went to the mock only ---
        $hosts = (Send-Control "egress-hosts").hosts
        Check "all egress went to the mock host only ($($hosts -join ', '))" `
            (@($hosts).Count -eq 1 -and $hosts[0] -eq "127.0.0.1:47700")

        # --- Three dismissals disable a trigger type ---
        $s1 = (Send-Control "fire-surface clipboard_lead").shown
        foreach ($i in 1..3) { $null = Send-Control "dismiss clipboard_lead" }
        $s2 = (Send-Control "fire-surface clipboard_lead").shown
        $s3 = (Send-Control "fire-surface clipboard_lead").shown
        Check "trigger fires before dismissals, never after three" ($s1 -and -not $s2 -and -not $s3)

        # --- Hard silence suppresses all unsolicited surfaces ---
        $null = Send-Control "set-setting hard_silence 1"
        $silenced = (Send-Control "fire-surface decay").shown
        $null = Send-Control "set-setting hard_silence 0"
        $unsilenced = (Send-Control "fire-surface decay").shown
        Check "hard silence suppresses surfaces, lifting restores them" (-not $silenced -and $unsilenced)

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
if ($failures.Count -eq 0) { Write-Host "M3 GATE: PASS"; exit 0 }
Write-Host "M3 GATE: FAIL ($($failures.Count)): $($failures -join '; ')"
exit 1
