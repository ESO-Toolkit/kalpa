$ErrorActionPreference = "Stop"

$node = Get-Command node -ErrorAction SilentlyContinue
if (-not $node) {
  Write-Error "Node.js is required to run the Slint screenshot diff harness. Install Node or run this from a shell where node is on PATH."
  exit 1
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$nodeScript = Join-Path $scriptDir "screenshot-diff.mjs"

& $node.Source $nodeScript @args
exit $LASTEXITCODE
