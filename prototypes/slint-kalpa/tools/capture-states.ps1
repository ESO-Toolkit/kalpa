param(
  [string[]]$State = @("main", "discover-popular", "files", "settings-general"),
  [ValidateSet("low-memory", "standard")]
  [string]$Preset = "low-memory",
  [string]$OutputDir,
  [switch]$Build,
  [int]$WaitMilliseconds = 900
)

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$prototypeRoot = Resolve-Path (Join-Path $scriptDir "..")
$repoRoot = Resolve-Path (Join-Path $prototypeRoot "..\..")

if (-not $OutputDir) {
  $OutputDir = Join-Path $prototypeRoot "captures\verify"
}

$State = @($State | ForEach-Object { $_ -split "," } | Where-Object { $_ -ne "" })

if ($Build) {
  Push-Location $prototypeRoot
  try {
    & cargo build -j1
    if ($LASTEXITCODE -ne 0) {
      throw "cargo build failed with exit code $LASTEXITCODE"
    }
  } finally {
    Pop-Location
  }
}

$exe = Join-Path $prototypeRoot "target\debug\kalpa-slint-prototype.exe"
if (-not (Test-Path -LiteralPath $exe)) {
  throw "Prototype executable not found: $exe. Run cargo build first, or pass -Build."
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

if (-not ("KalpaCaptureNative" -as [type])) {
  $pinvoke = @"
using System;
using System.Runtime.InteropServices;

public static class KalpaCaptureNative {
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT {
    public int Left;
    public int Top;
    public int Right;
    public int Bottom;
  }

  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

  [DllImport("user32.dll")]
  public static extern bool SetProcessDpiAwarenessContext(IntPtr dpiContext);

  [DllImport("user32.dll")]
  public static extern bool SetProcessDPIAware();

  [DllImport("user32.dll")]
  public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

  [DllImport("user32.dll")]
  public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

  [DllImport("user32.dll")]
  public static extern bool IsWindowVisible(IntPtr hWnd);

  [DllImport("user32.dll")]
  public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

  [DllImport("user32.dll")]
  public static extern bool ShowWindow(IntPtr hWnd, int command);

  [DllImport("user32.dll")]
  public static extern bool SetWindowPos(
    IntPtr hWnd,
    IntPtr insertAfter,
    int x,
    int y,
    int cx,
    int cy,
    uint flags);

  [DllImport("dwmapi.dll")]
  public static extern int DwmFlush();
}
"@
  Add-Type $pinvoke
}

[KalpaCaptureNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
[KalpaCaptureNative]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing

function New-StateEnvironment {
  param([string]$Name)

  $env = @{
    KALPA_RENDER_PRESET = $Preset
    KALPA_REDUCED_MOTION = "1"
  }

  switch ($Name) {
    "main" { }
    "discover-popular" {
      $env.KALPA_VIEW = "discover"
      $env.KALPA_DISCOVER_TAB = "popular"
    }
    "discover-search" {
      $env.KALPA_VIEW = "discover"
      $env.KALPA_DISCOVER_TAB = "search"
      $env.KALPA_DISCOVER_QUERY = "combat"
    }
    "discover-category" {
      $env.KALPA_VIEW = "discover"
      $env.KALPA_DISCOVER_TAB = "categories"
    }
    "discover-url" {
      $env.KALPA_VIEW = "discover"
      $env.KALPA_DISCOVER_TAB = "url"
      $env.KALPA_DISCOVER_URL = "1360"
    }
    "files" {
      $env.KALPA_DETAIL_TAB = "files"
    }
    "files-editing" {
      $env.KALPA_DETAIL_TAB = "files"
      $env.KALPA_FILE_EDITOR_EDITABLE = "1"
      $env.KALPA_FILE_EDITOR_DIRTY = "1"
    }
    "settings-general" {
      $env.KALPA_SETTINGS_OPEN = "1"
      $env.KALPA_SETTINGS_TAB = "0"
    }
    "settings-appearance" {
      $env.KALPA_SETTINGS_OPEN = "1"
      $env.KALPA_SETTINGS_TAB = "1"
    }
    "settings-theme-editor" {
      $env.KALPA_SETTINGS_OPEN = "1"
      $env.KALPA_SETTINGS_TAB = "1"
      $env.KALPA_SETTINGS_EDITOR = "1"
    }
    "settings-tools" {
      $env.KALPA_SETTINGS_OPEN = "1"
      $env.KALPA_SETTINGS_TAB = "2"
    }
    "settings-data" {
      $env.KALPA_SETTINGS_OPEN = "1"
      $env.KALPA_SETTINGS_TAB = "3"
    }
    "theme-crimson" {
      $env.KALPA_THEME = "daedric-crimson"
    }
    "theme-frost" {
      $env.KALPA_THEME = "coldharbour-frost"
    }
    default {
      throw "Unknown capture state '$Name'."
    }
  }

  $env
}

function Get-LargestProcessWindow {
  param([int]$OwnerProcessId)

  $windows = New-Object System.Collections.Generic.List[object]
  $callback = [KalpaCaptureNative+EnumWindowsProc]{
    param($hWnd, $lParam)

    $windowProcessId = [uint32]0
    [KalpaCaptureNative]::GetWindowThreadProcessId($hWnd, [ref]$windowProcessId) | Out-Null

    if ($windowProcessId -eq [uint32]$OwnerProcessId -and [KalpaCaptureNative]::IsWindowVisible($hWnd)) {
      $rect = New-Object KalpaCaptureNative+RECT
      [KalpaCaptureNative]::GetWindowRect($hWnd, [ref]$rect) | Out-Null
      $width = $rect.Right - $rect.Left
      $height = $rect.Bottom - $rect.Top
      $windows.Add([pscustomobject]@{
        Hwnd = $hWnd
        Rect = $rect
        Width = $width
        Height = $height
        Area = $width * $height
      }) | Out-Null
    }

    return $true
  }

  [KalpaCaptureNative]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
  $windows |
    Where-Object { $_.Width -ge 400 -and $_.Height -ge 300 } |
    Sort-Object Area -Descending |
    Select-Object -First 1
}

function Get-HeaderSignature {
  param([string]$Path)

  $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
  try {
    $maxX = [Math]::Min(260, $bitmap.Width)
    $maxY = [Math]::Min(80, $bitmap.Height)
    $gold = 0
    $teal = 0

    for ($y = 0; $y -lt $maxY; $y += 2) {
      for ($x = 0; $x -lt $maxX; $x += 2) {
        $pixel = $bitmap.GetPixel($x, $y)
        if ($pixel.R -gt 120 -and $pixel.G -gt 90 -and $pixel.B -lt 90) {
          $gold++
        }
        if ($pixel.G -gt 100 -and $pixel.B -gt 120 -and $pixel.R -lt 90) {
          $teal++
        }
      }
    }

    [pscustomobject]@{
      GoldPixels = $gold
      TealPixels = $teal
      LooksLikeKalpa = ($gold -gt 8 -or $teal -gt 8)
    }
  } finally {
    $bitmap.Dispose()
  }
}

function Capture-State {
  param(
    [string]$Name,
    [hashtable]$Environment
  )

  $startInfo = [System.Diagnostics.ProcessStartInfo]::new($exe)
  $startInfo.WorkingDirectory = $prototypeRoot
  $startInfo.UseShellExecute = $false

  foreach ($key in $Environment.Keys) {
    $startInfo.Environment[$key] = [string]$Environment[$key]
  }

  $process = [System.Diagnostics.Process]::Start($startInfo)
  try {
    $window = $null
    $deadline = (Get-Date).AddSeconds(18)

    while ((Get-Date) -lt $deadline) {
      Start-Sleep -Milliseconds 350
      $process.Refresh()
      if ($process.HasExited) {
        throw "Prototype exited before capture for '$Name'."
      }

      $window = Get-LargestProcessWindow -OwnerProcessId $process.Id
      if ($null -ne $window) {
        break
      }
    }

    if ($null -eq $window) {
      throw "No full-size Slint window found for '$Name'."
    }

    $flags = 0x0001 -bor 0x0002 -bor 0x0010 -bor 0x0040
    [KalpaCaptureNative]::ShowWindow($window.Hwnd, 5) | Out-Null
    [KalpaCaptureNative]::SetWindowPos($window.Hwnd, [IntPtr](-1), 0, 0, 0, 0, $flags) | Out-Null
    Start-Sleep -Milliseconds $WaitMilliseconds
    [KalpaCaptureNative]::DwmFlush() | Out-Null

    $rect = New-Object KalpaCaptureNative+RECT
    [KalpaCaptureNative]::GetWindowRect($window.Hwnd, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top

    if ($width -lt 1200 -or $height -lt 700) {
      throw "Bad window rect for '$Name': ${width}x${height}."
    }

    $bitmap = [System.Drawing.Bitmap]::new($width, $height)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen(
      $rect.Left,
      $rect.Top,
      0,
      0,
      [System.Drawing.Size]::new($width, $height))

    $path = Join-Path $OutputDir "$Name.png"
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose()
    $bitmap.Dispose()

    [KalpaCaptureNative]::SetWindowPos($window.Hwnd, [IntPtr](-2), 0, 0, 0, 0, $flags) | Out-Null

    $signature = Get-HeaderSignature -Path $path
    $signatureOk = $signature.LooksLikeKalpa -or $Name.StartsWith("settings-")
    [pscustomobject]@{
      State = $Name
      Path = $path
      Left = $rect.Left
      Top = $rect.Top
      Width = $width
      Height = $height
      Preset = $Preset
      HeaderSignature = $signatureOk
    }
  } finally {
    if (-not $process.HasExited) {
      $process.Kill()
      $process.WaitForExit(3000) | Out-Null
    }
  }
}

$results = @()
foreach ($name in $State) {
  $results += Capture-State -Name $name -Environment (New-StateEnvironment -Name $name)
}

$results | Format-Table -AutoSize

$badSignature = $results | Where-Object { -not $_.HeaderSignature }
if ($badSignature) {
  Write-Warning "One or more captures did not show the expected Kalpa header signature. Inspect these manually before trusting them."
}
