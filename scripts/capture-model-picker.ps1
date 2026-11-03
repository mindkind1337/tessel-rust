param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$OutputDirectory,
    [switch]$Compact
)
$ErrorActionPreference = 'Stop'
# Requires a binary built with --features visual-test. Every run gets isolated
# synthetic app data. It never drives the desktop or an existing Tessel window.
$binaryPath = (Resolve-Path -LiteralPath $Executable).Path
$captureRoot = [IO.Path]::GetFullPath($OutputDirectory)
$runDirectory = Join-Path $captureRoot ([Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $runDirectory -Force
$dataDirectory = Join-Path $runDirectory 'data'
$pngPath = Join-Path $runDirectory 'models-fr.png'
$errorPath = Join-Path $runDirectory 'stderr.txt'
$outputPath = Join-Path $runDirectory 'stdout.txt'
$previousCapture = $env:TESSEL_SETTLED_SCREENSHOT
$process = $null
try {
    $env:TESSEL_SETTLED_SCREENSHOT = $pngPath
    $arguments = @('--models-fixture', '--data-dir', ('"' + $dataDirectory + '"'))
    if ($Compact) { $arguments += '--compact-fixture' }
    $process = Start-Process -FilePath $binaryPath -ArgumentList $arguments -WorkingDirectory $runDirectory -WindowStyle Hidden -PassThru -RedirectStandardError $errorPath -RedirectStandardOutput $outputPath
    if (-not $process.WaitForExit(30000)) {
        # Only terminate the isolated process this script just created.
        $process.Kill()
        throw 'Isolated model-picker capture timed out.'
    }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $pngPath)) {
        throw "Model-picker capture failed. Inspect $errorPath"
    }
    [pscustomobject]@{ Image=$pngPath; Data=$dataDirectory; ExitCode=$process.ExitCode }
}
finally {
    $env:TESSEL_SETTLED_SCREENSHOT = $previousCapture
}
