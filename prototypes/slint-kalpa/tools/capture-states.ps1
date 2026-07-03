param(
  [string[]]$State = @("main", "discover-popular", "files", "settings-general"),
  [ValidateSet("low-memory", "standard")]
  [string]$Preset = "low-memory",
  [string]$OutputDir,
  [switch]$Build,
  [int]$WaitMilliseconds = 900,
  [int]$WindowWidth = 1920,
  [int]$WindowHeight = 1080,
  [switch]$FitPrimaryWorkArea,
  [switch]$ScreenFallback,
  [switch]$Foreground,
  [switch]$KeepOpen,
  [switch]$LeaveOpen,
  [switch]$NoCapture,
  [int]$KeepOpenSeconds = 0
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

if ($ScreenFallback -and -not $Foreground) {
  throw "-ScreenFallback captures the visible desktop and requires -Foreground. It is disabled by default so verification cannot grab whatever you are using."
}

if (-not ("KalpaCaptureNative" -as [type])) {
  $pinvoke = @"
using System;
using System.Runtime.InteropServices;
using System.Text;

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

  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern int GetWindowTextLength(IntPtr hWnd);

  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern int GetClassName(IntPtr hWnd, StringBuilder text, int maxCount);

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

  [DllImport("user32.dll")]
  public static extern bool BringWindowToTop(IntPtr hWnd);

  [DllImport("user32.dll")]
  public static extern bool SetForegroundWindow(IntPtr hWnd);

  [DllImport("user32.dll")]
  public static extern bool UpdateWindow(IntPtr hWnd);

  [DllImport("user32.dll")]
  public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint flags);

  [DllImport("dwmapi.dll")]
  public static extern int DwmFlush();
}
"@
  Add-Type $pinvoke
}

[KalpaCaptureNative]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
[KalpaCaptureNative]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

function New-StateEnvironment {
  param([string]$Name)

  $env = @{
    KALPA_RENDER_PRESET = $Preset
    KALPA_REDUCED_MOTION = "1"
  }

  foreach ($key in @("KALPA_ADDONS_PATH", "KALPA_NATIVE_STATE_DIR", "KALPA_THEME_FILE", "KALPA_THEME_JSON")) {
    $value = [System.Environment]::GetEnvironmentVariable($key)
    if (-not [string]::IsNullOrWhiteSpace($value)) {
      $env[$key] = $value
    }
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
    "packhub-browse" {
      $env.KALPA_PACK_HUB_OPEN = "1"
      $env.KALPA_PACK_HUB_VIEW = "browse"
    }
    "packhub-create1" {
      $env.KALPA_PACK_HUB_OPEN = "1"
      $env.KALPA_PACK_HUB_VIEW = "create-details"
    }
    "packhub-create2" {
      $env.KALPA_PACK_HUB_OPEN = "1"
      $env.KALPA_PACK_HUB_VIEW = "create-addons"
    }
    "packhub-install" {
      $env.KALPA_PACK_HUB_OPEN = "1"
      $env.KALPA_PACK_HUB_VIEW = "install-detail"
    }
    "uploader-manual" {
      $env.KALPA_UPLOADER_OPEN = "1"
      $env.KALPA_UPLOADER_VIEW = "manual"
    }
    "uploader-uploading" {
      $env.KALPA_UPLOADER_OPEN = "1"
      $env.KALPA_UPLOADER_VIEW = "uploading"
    }
    "uploader-live" {
      $env.KALPA_UPLOADER_OPEN = "1"
      $env.KALPA_UPLOADER_VIEW = "live"
    }
    "uploader-live-running" {
      $env.KALPA_UPLOADER_OPEN = "1"
      $env.KALPA_UPLOADER_VIEW = "live-running"
    }
    "svm-overview" {
      $env.KALPA_SVM_OPEN = "1"
      $env.KALPA_SVM_VIEW = "overview"
    }
    "svm-cleanup" {
      $env.KALPA_SVM_OPEN = "1"
      $env.KALPA_SVM_VIEW = "cleanup"
    }
    "svm-copy" {
      $env.KALPA_SVM_OPEN = "1"
      $env.KALPA_SVM_VIEW = "copy-profile"
    }
    "svm-editor" {
      $env.KALPA_SVM_OPEN = "1"
      $env.KALPA_SVM_VIEW = "editor"
    }
    "backup-restore-main" {
      $env.KALPA_BACKUP_RESTORE_OPEN = "1"
      $env.KALPA_BACKUP_RESTORE_VIEW = "main"
    }
    "backup-restore-label" {
      $env.KALPA_BACKUP_RESTORE_OPEN = "1"
      $env.KALPA_BACKUP_RESTORE_VIEW = "custom-label"
    }
    "backup-restore-confirm" {
      $env.KALPA_BACKUP_RESTORE_OPEN = "1"
      $env.KALPA_BACKUP_RESTORE_VIEW = "restore-confirm"
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

    if ($windowProcessId -eq [uint32]$OwnerProcessId) {
      $rect = New-Object KalpaCaptureNative+RECT
      [KalpaCaptureNative]::GetWindowRect($hWnd, [ref]$rect) | Out-Null
      $width = $rect.Right - $rect.Left
      $height = $rect.Bottom - $rect.Top
      $title = New-Object System.Text.StringBuilder 256
      [KalpaCaptureNative]::GetWindowText($hWnd, $title, $title.Capacity) | Out-Null
      $className = New-Object System.Text.StringBuilder 256
      [KalpaCaptureNative]::GetClassName($hWnd, $className, $className.Capacity) | Out-Null
      $visible = [KalpaCaptureNative]::IsWindowVisible($hWnd)
      $score = $width * $height
      if ($title.ToString() -eq "Kalpa") {
        $score += 1000000000000
      }
      if ($className.ToString() -eq "Window Class") {
        $score += 100000000000
      }
      if ($visible) {
        $score += 10000000000
      }
      $windows.Add([pscustomobject]@{
        Hwnd = $hWnd
        Rect = $rect
        Width = $width
        Height = $height
        Area = $width * $height
        Title = $title.ToString()
        ClassName = $className.ToString()
        Visible = $visible
        Score = $score
      }) | Out-Null
    }

    return $true
  }

  [KalpaCaptureNative]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
  $windows |
    Where-Object { $_.Width -ge 400 -and $_.Height -ge 300 } |
    Sort-Object Score -Descending |
    Select-Object -First 1
}

function Save-WindowCapture {
  param(
    [IntPtr]$Hwnd,
    [KalpaCaptureNative+RECT]$Rect,
    [string]$Path,
    [switch]$UsePrintWindow
  )

  $width = $Rect.Right - $Rect.Left
  $height = $Rect.Bottom - $Rect.Top
  $bitmap = [System.Drawing.Bitmap]::new($width, $height)
  $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
  $method = "screen"

  try {
    if ($UsePrintWindow) {
      $hdc = $graphics.GetHdc()
      try {
        if ([KalpaCaptureNative]::PrintWindow($Hwnd, $hdc, 2)) {
          $method = "printwindow"
        }
      } finally {
        $graphics.ReleaseHdc($hdc)
      }
    }

    if ($method -eq "screen") {
      $graphics.CopyFromScreen(
        $Rect.Left,
        $Rect.Top,
        0,
        0,
        [System.Drawing.Size]::new($width, $height))
    }

    $bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    return $method
  } finally {
    $graphics.Dispose()
    $bitmap.Dispose()
  }
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

function Get-StateSignature {
  param(
    [string]$Path,
    [string]$Name
  )

  $needsModalSignature = $Name.StartsWith("svm-") -or $Name.StartsWith("backup-restore-")
  if (-not $needsModalSignature) {
    return [pscustomobject]@{
      LooksLikeState = $true
      GoldPixels = 0
      PanelPixels = 0
    }
  }

  $bitmap = [System.Drawing.Bitmap]::FromFile($Path)
  try {
    $xStart = [int]($bitmap.Width * 0.18)
    $xEnd = [int]($bitmap.Width * 0.72)
    $yStart = [int]($bitmap.Height * 0.06)
    $yEnd = [int]($bitmap.Height * 0.20)
    $gold = 0
    $panel = 0

    for ($y = $yStart; $y -lt $yEnd; $y += 2) {
      for ($x = $xStart; $x -lt $xEnd; $x += 2) {
        $pixel = $bitmap.GetPixel($x, $y)
        if ($pixel.R -gt 150 -and $pixel.G -gt 95 -and $pixel.G -lt 190 -and $pixel.B -lt 95) {
          $gold++
        }
        if ($pixel.R -ge 24 -and $pixel.R -le 48 -and $pixel.G -ge 28 -and $pixel.G -le 54 -and $pixel.B -ge 34 -and $pixel.B -le 62) {
          $panel++
        }
      }
    }

    [pscustomobject]@{
      LooksLikeState = ($gold -gt 20 -and $panel -gt 400)
      GoldPixels = $gold
      PanelPixels = $panel
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

    $workArea = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $targetLeft = $workArea.Left + 40
    $targetTop = $workArea.Top + 40
    $targetWidth = $WindowWidth
    $targetHeight = $WindowHeight
    if ($FitPrimaryWorkArea) {
      $targetWidth = $workArea.Width - 80
      $targetHeight = $workArea.Height - 80
    }
    $targetWidth = [Math]::Min([Math]::Max($targetWidth, 1200), $workArea.Width - 80)
    $targetHeight = [Math]::Min([Math]::Max($targetHeight, 700), $workArea.Height - 80)
    $showFlags = 0x0004 -bor 0x0040
    $restoreFlags = 0x0001 -bor 0x0002 -bor 0x0040
    [KalpaCaptureNative]::ShowWindow($window.Hwnd, 9) | Out-Null
    $insertAfter = [IntPtr]::Zero
    if ($Foreground) {
      $showFlags = 0x0040
      $insertAfter = [IntPtr](-1)
    }
    [KalpaCaptureNative]::SetWindowPos($window.Hwnd, $insertAfter, $targetLeft, $targetTop, $targetWidth, $targetHeight, $showFlags) | Out-Null
    if ($Foreground) {
      [KalpaCaptureNative]::BringWindowToTop($window.Hwnd) | Out-Null
      [KalpaCaptureNative]::SetForegroundWindow($window.Hwnd) | Out-Null
    }
    [KalpaCaptureNative]::UpdateWindow($window.Hwnd) | Out-Null
    Start-Sleep -Milliseconds $WaitMilliseconds
    [KalpaCaptureNative]::DwmFlush() | Out-Null

    $rect = New-Object KalpaCaptureNative+RECT
    [KalpaCaptureNative]::GetWindowRect($window.Hwnd, [ref]$rect) | Out-Null
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top

    if ($width -lt 1200 -or $height -lt 700) {
      throw "Bad window rect for '$Name': ${width}x${height}."
    }

    $path = Join-Path $OutputDir "$Name.png"
    $captureMethod = if ($NoCapture) { "none" } else { "printwindow" }
    $signatureOk = $true
    if (-not $NoCapture) {
      $captureMethod = Save-WindowCapture -Hwnd $window.Hwnd -Rect $rect -Path $path -UsePrintWindow

      $signature = Get-HeaderSignature -Path $path
      $stateSignature = Get-StateSignature -Path $path -Name $Name
      $needsModalSignature = $Name.StartsWith("svm-") -or $Name.StartsWith("backup-restore-")
      if ($needsModalSignature) {
        $signatureOk = $stateSignature.LooksLikeState
      } else {
        $signatureOk = $signature.LooksLikeKalpa -or $Name.StartsWith("settings-") -or $Name.StartsWith("packhub-") -or $Name.StartsWith("uploader-")
      }

      if (-not $signatureOk -and $ScreenFallback) {
        $captureMethod = Save-WindowCapture -Hwnd $window.Hwnd -Rect $rect -Path $path
        $signature = Get-HeaderSignature -Path $path
        $stateSignature = Get-StateSignature -Path $path -Name $Name
        if ($needsModalSignature) {
          $signatureOk = $stateSignature.LooksLikeState
        } else {
          $signatureOk = $signature.LooksLikeKalpa -or $Name.StartsWith("settings-") -or $Name.StartsWith("packhub-") -or $Name.StartsWith("uploader-")
        }
      }

      [KalpaCaptureNative]::SetWindowPos($window.Hwnd, [IntPtr](-2), 0, 0, 0, 0, $restoreFlags) | Out-Null

      if ($needsModalSignature -and -not $signatureOk) {
        throw "Capture for '$Name' did not match the expected modal signature. Inspect $path before trusting the screenshot."
      }
    }

    [pscustomobject]@{
      State = $Name
      Path = $path
      Left = $rect.Left
      Top = $rect.Top
      Width = $width
      Height = $height
      Preset = $Preset
      WindowTitle = $window.Title
      WindowClass = $window.ClassName
      WindowVisible = $window.Visible
      CaptureMethod = $captureMethod
      HeaderSignature = $signatureOk
      ProcessId = $process.Id
    }
  } finally {
    if ($LeaveOpen -and -not $process.HasExited) {
      Write-Host "Leaving prototype process $($process.Id) open. Close the Kalpa window when finished."
    } elseif ($KeepOpen -and -not $process.HasExited) {
      if ($KeepOpenSeconds -gt 0) {
        Start-Sleep -Seconds $KeepOpenSeconds
      } else {
        Write-Host "Keeping prototype process $($process.Id) open. Close the Kalpa window when finished."
        $process.WaitForExit()
      }
    }

    if (-not $KeepOpen -and -not $LeaveOpen -and -not $process.HasExited) {
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
