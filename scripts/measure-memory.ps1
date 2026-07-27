<#
.SYNOPSIS
  Measure a Kalpa UI's memory the way README.md reports it.

.DESCRIPTION
  Reports the PRIVATE WORKING SET summed over a process tree — the same number
  Task Manager shows in its "Memory" column.

  Why private working set and not WorkingSet64: the WebView2 UI runs as ~7
  processes that share a large amount of mapped image and shader memory. Summing
  WorkingSet64 counts those shared pages once per process and inflates the
  webview roughly 3x (a measured 467 MB against a true 137 MB). WorkingSetPrivate
  counts each page against exactly one process, so a tree sum is meaningful.

  Never measure with KALPA_DEMO_DATA set — it loads a synthetic catalogue and
  inflates the native sidecar.

.EXAMPLE
  # Native sidecar, default renderer, open and minimized
  ./scripts/measure-memory.ps1 -Exe prototypes/slint-kalpa/target/release/kalpa-slint-prototype.exe -Minimize

.EXAMPLE
  # Same binary under the low-memory (software rendering) preset
  ./scripts/measure-memory.ps1 -Exe ... -Env @{ KALPA_RENDER_PRESET = 'low-memory' } -Minimize

.EXAMPLE
  # Already-running process tree (e.g. the installed webview build)
  ./scripts/measure-memory.ps1 -ProcessName Kalpa
#>
[CmdletBinding(DefaultParameterSetName = 'Launch')]
param(
    # Executable to launch and measure.
    [Parameter(Mandatory, ParameterSetName = 'Launch')]
    [string]$Exe,

    # Measure an already-running process tree by name (no .exe suffix).
    [Parameter(Mandatory, ParameterSetName = 'Attach')]
    [string]$ProcessName,

    # Extra environment variables for the launched process.
    [Parameter(ParameterSetName = 'Launch')]
    [hashtable]$Env = @{},

    # Seconds to let the process settle before sampling. The README numbers use
    # 100 s; startup allocations keep falling for roughly the first minute.
    [int]$SettleSeconds = 100,

    # Number of cold launches. The reported figure is the median.
    [int]$Runs = 3,

    # Also minimize the window, settle again, and report the minimized figure.
    [switch]$Minimize,

    # Seconds to settle after minimizing before sampling again.
    [int]$MinimizeSettleSeconds = 30
)

$ErrorActionPreference = 'Stop'

if ($env:KALPA_DEMO_DATA) {
    Write-Warning "KALPA_DEMO_DATA is set; it inflates the native sidecar. Clear it before trusting these numbers."
}

Add-Type -Namespace Win32 -Name Native -MemberDefinition @'
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
'@

# Sum WorkingSetPrivate across a root PID and all its descendants.
function Get-TreePrivateWorkingSetMB {
    param([int]$RootPid)

    # Build the pid -> parent map once; walking it is cheaper than repeated CIM
    # queries and gives a consistent snapshot.
    $all = Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId
    $childrenByParent = @{}
    foreach ($p in $all) {
        if (-not $childrenByParent.ContainsKey($p.ParentProcessId)) {
            $childrenByParent[$p.ParentProcessId] = New-Object System.Collections.Generic.List[int]
        }
        $childrenByParent[$p.ParentProcessId].Add($p.ProcessId)
    }

    $tree = New-Object System.Collections.Generic.List[int]
    $queue = New-Object System.Collections.Generic.Queue[int]
    $queue.Enqueue($RootPid)
    while ($queue.Count -gt 0) {
        $current = $queue.Dequeue()
        if ($tree.Contains($current)) { continue }
        $tree.Add($current)
        if ($childrenByParent.ContainsKey($current)) {
            foreach ($child in $childrenByParent[$current]) { $queue.Enqueue($child) }
        }
    }

    # WorkingSetPrivate lives on the perf-counter class, joined by IDProcess.
    # Win32_Process's WorkingSetSize is the *total* working set — wrong metric.
    $perf = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Property IDProcess, WorkingSetPrivate
    $bytes = 0
    $counted = 0
    foreach ($row in $perf) {
        if ($tree.Contains([int]$row.IDProcess)) {
            $bytes += [uint64]$row.WorkingSetPrivate
            $counted++
        }
    }

    [pscustomobject]@{
        MB        = [math]::Round($bytes / 1MB, 1)
        Processes = $counted
    }
}

function Get-MainWindowHandle {
    param([int]$RootPid)
    for ($i = 0; $i -lt 30; $i++) {
        $procs = Get-Process -Id $RootPid -ErrorAction SilentlyContinue
        if ($procs -and $procs.MainWindowHandle -ne 0) { return $procs.MainWindowHandle }
        Start-Sleep -Seconds 1
    }
    return [IntPtr]::Zero
}

function Measure-Once {
    param([int]$RunIndex)

    if ($PSCmdlet.ParameterSetName -eq 'Attach') {
        $proc = Get-Process -Name $ProcessName -ErrorAction Stop |
            Sort-Object StartTime | Select-Object -First 1
        $rootPid = $proc.Id
        $launched = $false
    } else {
        $resolved = (Resolve-Path $Exe).Path
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $resolved
        $psi.UseShellExecute = $false
        foreach ($key in $Env.Keys) { $psi.EnvironmentVariables[$key] = [string]$Env[$key] }
        $proc = [System.Diagnostics.Process]::Start($psi)
        $rootPid = $proc.Id
        $launched = $true
    }

    try {
        Write-Host ("  run {0}: pid {1}, settling {2}s..." -f $RunIndex, $rootPid, $SettleSeconds)
        Start-Sleep -Seconds $SettleSeconds

        $open = Get-TreePrivateWorkingSetMB -RootPid $rootPid
        $result = [ordered]@{ OpenMB = $open.MB; Processes = $open.Processes; MinimizedMB = $null }
        Write-Host ("    open:      {0,7:N1} MB  ({1} process(es))" -f $open.MB, $open.Processes)

        if ($Minimize) {
            $hwnd = Get-MainWindowHandle -RootPid $rootPid
            if ($hwnd -eq [IntPtr]::Zero) {
                Write-Warning "    no main window found; skipping the minimized sample"
            } else {
                [void][Win32.Native]::ShowWindow($hwnd, 6)  # SW_MINIMIZE
                Start-Sleep -Seconds 2
                if (-not [Win32.Native]::IsIconic($hwnd)) {
                    Write-Warning "    window did not report as minimized"
                }
                Start-Sleep -Seconds $MinimizeSettleSeconds
                $min = Get-TreePrivateWorkingSetMB -RootPid $rootPid
                $result.MinimizedMB = $min.MB
                Write-Host ("    minimized: {0,7:N1} MB" -f $min.MB)
            }
        }

        [pscustomobject]$result
    } finally {
        if ($launched) {
            Get-Process -Id $rootPid -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 2
        }
    }
}

function Get-Median {
    param([double[]]$Values)
    if (-not $Values -or $Values.Count -eq 0) { return $null }
    $sorted = $Values | Sort-Object
    $mid = [int][math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2 -eq 1) { return $sorted[$mid] }
    return [math]::Round((($sorted[$mid - 1] + $sorted[$mid]) / 2), 1)
}

$effectiveRuns = if ($PSCmdlet.ParameterSetName -eq 'Attach') { 1 } else { $Runs }
$samples = @()
for ($i = 1; $i -le $effectiveRuns; $i++) { $samples += Measure-Once -RunIndex $i }

Write-Host ""
Write-Host "=== median over $effectiveRuns run(s), private working set ==="
$openMedian = Get-Median -Values ($samples | ForEach-Object { $_.OpenMB })
Write-Host ("open:      {0,7:N1} MB" -f $openMedian)
$minValues = $samples | Where-Object { $null -ne $_.MinimizedMB } | ForEach-Object { $_.MinimizedMB }
if ($minValues) {
    Write-Host ("minimized: {0,7:N1} MB" -f (Get-Median -Values $minValues))
}
