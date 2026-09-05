#requires -Version 7.0
<#
Compare release binaries against a real registered profile. This deliberately
corrupts one caller-selected, previously verified payload byte and repairs it.
Run full profile validation first; supply that file's original SHA-256 and the
manifest piece length. Backups, binaries and raw results stay outside the target.
#>
param(
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [Parameter(Mandatory)][string]$ConfigDir,
    [Parameter(Mandatory)][string]$ProfileId,
    [Parameter(Mandatory)][string]$Target,
    [Parameter(Mandatory)][string]$RepairPath,
    [Parameter(Mandatory)][long]$Offset,
    [Parameter(Mandatory)][string]$ExpectedSha256,
    [Parameter(Mandatory)][long]$ExpectedFetchedBytes,
    [Parameter(Mandatory)][string]$OutputDir,
    [ValidateRange(1, 20)][int]$Trials = 3
)
$ErrorActionPreference = 'Stop'
$targetRoot = (Resolve-Path -LiteralPath $Target).Path.TrimEnd('\', '/')
$outputRoot = [IO.Path]::GetFullPath($OutputDir)
$repairFile = [IO.Path]::GetFullPath((Join-Path $targetRoot $RepairPath))
function Within([string]$Path, [string]$Root) {
    $Path.Equals($Root, [StringComparison]::OrdinalIgnoreCase) -or
        $Path.StartsWith($Root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)
}
if (!(Within $repairFile $targetRoot) -or (Within $outputRoot $targetRoot)) {
    throw 'Repair must be inside target; results and backup must be outside target.'
}
$profilesFile = Join-Path $ConfigDir 'profiles.json'
$profilesHash = (Get-FileHash -LiteralPath $profilesFile -Algorithm SHA256).Hash
$registered = @((Get-Content -Raw -LiteralPath $profilesFile | ConvertFrom-Json).profiles | Where-Object id -eq $ProfileId)
if ($registered.Count -ne 1 -or ![IO.Path]::GetFullPath($registered[0].destination).TrimEnd('\', '/').Equals($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Profile destination does not match the selected test target.'
}
$settingsFile = Join-Path $ConfigDir 'settings.json'
$settingsHash = if (Test-Path -LiteralPath $settingsFile) { (Get-FileHash -LiteralPath $settingsFile).Hash } else { $null }
if ((Get-FileHash -LiteralPath $repairFile -Algorithm SHA256).Hash -ne $ExpectedSha256) {
    throw 'Selected repair file does not match the verified original hash.'
}
$originalLength = (Get-Item -LiteralPath $repairFile).Length
if ($Offset -lt 0 -or $Offset -ge $originalLength -or $ExpectedFetchedBytes -le 0) { throw 'Invalid payload offset or piece length.' }
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$backup = Join-Path $outputRoot 'original.bin'
Copy-Item -LiteralPath $repairFile -Destination $backup
if ((Get-FileHash -LiteralPath $backup).Hash -ne $ExpectedSha256) { throw 'Backup verification failed.' }
$binaries = @{}
foreach ($label in @('baseline', 'candidate')) {
    $source = if ($label -eq 'baseline') { $Baseline } else { $Candidate }
    $destination = Join-Path $outputRoot "$label.exe"
    Copy-Item -LiteralPath $source -Destination $destination
    $binaries[$label] = $destination
}
$rows = [Collections.Generic.List[object]]::new()
$identity = [ordered]@{
    schema_version = 1; profile = $ProfileId; source = $registered[0].source; target = $targetRoot; repair_path = $RepairPath
    original_sha256 = $ExpectedSha256; original_bytes = $originalLength; offset = $Offset
    expected_fetched_bytes = $ExpectedFetchedBytes; trials = $Trials
    profiles_sha256 = $profilesHash; os = [Environment]::OSVersion.VersionString
    logical_processors = [Environment]::ProcessorCount; powershell = $PSVersionTable.PSVersion.ToString()
    baseline_sha256 = (Get-FileHash -LiteralPath $binaries.baseline).Hash
    candidate_sha256 = (Get-FileHash -LiteralPath $binaries.candidate).Hash
    transport = 'registered profile source'; cache_state = 'warm OS cache; durable inventory retained'
}
$identity | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputRoot 'identity.json')
function Measure-Sync([string]$Label, [int]$Trial, [string]$Scenario) {
    $start = [Diagnostics.ProcessStartInfo]::new($binaries[$Label])
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($arg in @('sync', $ProfileId, '--no-progress')) { $start.ArgumentList.Add($arg) }
    $start.Environment['FLEET_CONFIG_DIR'] = [IO.Path]::GetFullPath($ConfigDir)
    $start.Environment.Remove('FLEET_SIMULATE_SYNC') | Out-Null
    $start.Environment['RUST_LOG'] = 'flux=info'
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $process = [Diagnostics.Process]::Start($start)
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    $peak = 0L
    while (!$process.WaitForExit(25)) {
        $process.Refresh()
        $peak = [Math]::Max($peak, $process.PeakWorkingSet64)
    }
    $clock.Stop()
    $text = $stdout.GetAwaiter().GetResult()
    $errors = $stderr.GetAwaiter().GetResult()
    $stem = Join-Path $outputRoot "$Trial-$Label-$Scenario"
    $text | Set-Content -LiteralPath "$stem.stdout.log"
    $errors | Set-Content -LiteralPath "$stem.stderr.log"
    $exitCode = $process.ExitCode
    $process.Dispose()
    if ($exitCode -ne 0 -or $text -notmatch 'local_health: Clean' -or $text -notmatch 'repo_freshness: UpToDate') { throw "Failed $Label $Scenario; see $stem logs." }
    $success = [regex]::Match($text, 'operation="run_success"[^\r\n]+')
    if (!$success.Success) { throw 'Missing actual Flux outcome; cannot score a run.' }
    $row = [ordered]@{ trial = $Trial; binary = $Label; scenario = $Scenario; wall_seconds = $clock.Elapsed.TotalSeconds; peak_working_set_bytes = $peak }
    foreach ($counter in @('kept_files', 'reused_bytes', 'fetched_bytes', 'written_bytes', 'deleted_entries')) {
        $match = [regex]::Match($success.Value, "$counter=(\d+)")
        if (!$match.Success) { throw "Missing $counter" }
        $row[$counter] = [long]$match.Groups[1].Value
    }
    if ($row.deleted_entries -ne 0) { throw 'Unexpected deletion during same-goal comparison.' }
    if ($rows.Count -gt 0) {
        $expectedKept = $rows[0].kept_files - $(if ($Scenario -eq 'repair') { 1 } else { 0 })
        if ($row.kept_files -ne $expectedKept) { throw 'Target file count differs between comparison runs.' }
    }
    if ($Scenario -eq 'noop') {
        if ($row.fetched_bytes -ne 0 -or $row.written_bytes -ne 0 -or $row.reused_bytes -ne 0) { throw 'No-op performed data work.' }
    } elseif ($row.fetched_bytes -ne $ExpectedFetchedBytes -or $row.written_bytes -ne $originalLength -or $row.reused_bytes -ne ($originalLength - $ExpectedFetchedBytes)) {
        throw 'Repair did not fetch exactly the missing piece and reuse the intact remainder.'
    }
    if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256 -or (Get-Item -LiteralPath $repairFile).Length -ne $originalLength) { throw 'Restored bytes differ from the original.' }
    $rows.Add([pscustomobject]$row)
    $rows | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputRoot 'runs.json')
    Write-Host "$Label $Scenario trial $Trial : $([Math]::Round($clock.Elapsed.TotalSeconds, 3))s"
}
try {
    for ($trial = 1; $trial -le $Trials; $trial++) {
        $order = if ($trial % 2) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
        foreach ($label in $order) {
            Measure-Sync $label $trial 'noop'
            $stream = [IO.File]::Open($repairFile, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
            try {
                $stream.Position = $Offset
                $byte = $stream.ReadByte()
                $stream.Position = $Offset
                $stream.WriteByte([byte]($byte -bxor 1))
                $stream.Flush($true)
            } finally { $stream.Dispose() }
            if ((Get-FileHash -LiteralPath $repairFile).Hash -eq $ExpectedSha256) { throw 'Corruption was not applied.' }
            Measure-Sync $label $trial 'repair'
        }
    }
} finally {
    if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256) {
        Copy-Item -LiteralPath $backup -Destination $repairFile
        if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256) { throw "Restore failed; verified backup at $backup" }
    }
    if ((Get-FileHash -LiteralPath $profilesFile).Hash -ne $profilesHash) { throw 'Profile configuration changed during comparison.' }
    $finalSettingsHash = if (Test-Path -LiteralPath $settingsFile) { (Get-FileHash -LiteralPath $settingsFile).Hash } else { $null }
    if ($finalSettingsHash -ne $settingsHash) { throw 'Settings changed during comparison.' }
}
