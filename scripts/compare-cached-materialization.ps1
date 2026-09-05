#requires -Version 7.0
param(
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [Parameter(Mandatory)][string]$BaselineInventory,
    [Parameter(Mandatory)][string]$CandidateInventory,
    [Parameter(Mandatory)][string]$Source,
    [Parameter(Mandatory)][string]$RepoCache,
    [Parameter(Mandatory)][string]$Target,
    [Parameter(Mandatory)][string]$RepairPath,
    [Parameter(Mandatory)][long]$Offset,
    [Parameter(Mandatory)][string]$ExpectedSha256,
    [Parameter(Mandatory)][long]$ExpectedFetchedBytes,
    [Parameter(Mandatory)][int]$ExpectedFiles,
    [Parameter(Mandatory)][string]$OutputDir,
    [ValidateRange(1,20)][int]$Trials = 3
)
$ErrorActionPreference = 'Stop'
$targetRoot = (Resolve-Path -LiteralPath $Target).Path.TrimEnd('\','/')
$outputRoot = [IO.Path]::GetFullPath($OutputDir)
$repairFile = [IO.Path]::GetFullPath((Join-Path $targetRoot $RepairPath))
if (!$repairFile.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $outputRoot.StartsWith($targetRoot + '\', [StringComparison]::OrdinalIgnoreCase) -or
    $outputRoot.Equals($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Repair must be inside target; artifacts must be outside target.'
}
if (Test-Path -LiteralPath $outputRoot) { throw 'Output directory must be new.' }
if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256) { throw 'Original hash mismatch.' }
$length = (Get-Item -LiteralPath $repairFile).Length
if ($Offset -lt 0 -or $Offset -ge $length) { throw 'Offset outside original file.' }
New-Item -ItemType Directory -Path $outputRoot | Out-Null
$backup = Join-Path $outputRoot 'original.bin'
Copy-Item -LiteralPath $repairFile -Destination $backup
if ((Get-FileHash -LiteralPath $backup).Hash -ne $ExpectedSha256) { throw 'Backup verification failed.' }
$binaries = @{ baseline = [IO.Path]::GetFullPath($Baseline); candidate = [IO.Path]::GetFullPath($Candidate) }
$inventories = @{ baseline = [IO.Path]::GetFullPath($BaselineInventory); candidate = [IO.Path]::GetFullPath($CandidateInventory) }
$rows = [Collections.Generic.List[object]]::new()
$pinnedRevision = $null
$identity = [ordered]@{ source = $Source; repo_cache = [IO.Path]::GetFullPath($RepoCache); target = $targetRoot
    baseline_sha256 = (Get-FileHash -LiteralPath $Baseline).Hash
    candidate_sha256 = (Get-FileHash -LiteralPath $Candidate).Hash
    expected_sha256 = $ExpectedSha256; repair_path = $RepairPath; offset = $Offset
    cache = 'warm OS cache; independently seeded inventories; cached release only'
    logical_processors = [Environment]::ProcessorCount; os = [Environment]::OSVersion.VersionString }
$identity | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputRoot 'identity.json')
try {
    foreach ($scenario in @('noop','repair')) {
        for ($trial = 1; $trial -le $Trials; $trial++) {
            $order = if ($trial % 2) { @('baseline','candidate') } else { @('candidate','baseline') }
            foreach ($label in $order) {
                if ($scenario -eq 'repair') {
                    $file = [IO.File]::Open($repairFile, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
                    try {
                        $file.Position = $Offset
                        $byte = $file.ReadByte()
                        $file.Position = $Offset
                        $file.WriteByte([byte]($byte -bxor 1))
                        $file.Flush($true)
                    } finally { $file.Dispose() }
                }
                $start = [Diagnostics.ProcessStartInfo]::new($binaries[$label])
                $start.UseShellExecute = $false
                $start.CreateNoWindow = $true
                $start.RedirectStandardOutput = $true
                $start.RedirectStandardError = $true
                foreach ($arg in @('sync',$Source,[IO.Path]::GetFullPath($RepoCache),$targetRoot,$inventories[$label])) { $start.ArgumentList.Add($arg) }
                $timer = [Diagnostics.Stopwatch]::StartNew()
                $process = [Diagnostics.Process]::Start($start)
                $stdout = $process.StandardOutput.ReadToEndAsync()
                $stderr = $process.StandardError.ReadToEndAsync()
                $peak = 0L
                while (!$process.WaitForExit(25)) {
                    $process.Refresh()
                    $peak = [Math]::Max($peak,$process.PeakWorkingSet64)
                }
                $timer.Stop()
                $text = $stdout.GetAwaiter().GetResult()
                $errors = $stderr.GetAwaiter().GetResult()
                $stem = Join-Path $outputRoot "$trial-$label-$scenario"
                $text | Set-Content -LiteralPath "$stem.stdout.json"
                $errors | Set-Content -LiteralPath "$stem.stderr.log"
                $exitCode = $process.ExitCode
                $cpu = $process.TotalProcessorTime.TotalSeconds
                $process.Dispose()
                if ($exitCode -ne 0) { throw "Failed $label $scenario; see $stem." }
                $report = $text | ConvertFrom-Json
                if ($null -eq $pinnedRevision) { $pinnedRevision = $report.revision }
                if ($report.revision -ne $pinnedRevision) { throw 'Cached release changed.' }
                $expectedKept = $ExpectedFiles - $(if ($scenario -eq 'repair') { 1 } else { 0 })
                $expectedWritten = if ($scenario -eq 'repair') { $length } else { 0 }
                $expectedFetched = if ($scenario -eq 'repair') { $ExpectedFetchedBytes } else { 0 }
                $result = $report.result
                if ($result.kept_files -ne $expectedKept -or $result.deleted_entries -ne 0 -or
                    $result.written_bytes -ne $expectedWritten -or $result.fetched_bytes -ne $expectedFetched -or
                    $result.reused_bytes -ne ($expectedWritten - $expectedFetched)) { throw 'Declared work differs.' }
                if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256 -or (Get-Item -LiteralPath $repairFile).Length -ne $length) { throw 'Restored bytes differ.' }
                $rows.Add([pscustomobject]@{ binary = $label; scenario = $scenario; trial = $trial
                    process_seconds = $timer.Elapsed.TotalSeconds; cpu_seconds = $cpu; peak_working_set_bytes = $peak
                    operation_ns = $report.operation_ns; setup_ns = $report.setup_ns; revision = $report.revision; outcome = $result })
                $rows | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $outputRoot 'runs.json')
                Write-Host "$label $scenario $trial : $([Math]::Round($timer.Elapsed.TotalSeconds,3))s"
            }
        }
    }
} finally {
    if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256) {
        Copy-Item -LiteralPath $backup -Destination $repairFile
        if ((Get-FileHash -LiteralPath $repairFile).Hash -ne $ExpectedSha256) { throw "Restore failed; backup at $backup" }
    }
}
