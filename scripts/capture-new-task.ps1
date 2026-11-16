param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
# Requires --features visual-test. No existing application, workspace or account is used.
$binaryPath = (Resolve-Path -LiteralPath $Executable).Path
$captureRoot = [IO.Path]::GetFullPath($OutputDirectory)
$runDirectory = Join-Path $captureRoot ([Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $runDirectory -Force
$dataDirectory = Join-Path $runDirectory 'data'
$pngPath = Join-Path $runDirectory 'new-task-fr.png'
$previousCapture = $env:TESSEL_SETTLED_SCREENSHOT
try {
    $env:TESSEL_SETTLED_SCREENSHOT = $pngPath
    $arguments = @('--new-task-fixture', '--data-dir', ('"' + $dataDirectory + '"'))
    $process = Start-Process -FilePath $binaryPath -ArgumentList $arguments -WorkingDirectory $runDirectory -WindowStyle Hidden -PassThru -RedirectStandardError (Join-Path $runDirectory 'stderr.txt') -RedirectStandardOutput (Join-Path $runDirectory 'stdout.txt')
    if (-not $process.WaitForExit(30000)) {
        # Bound this isolated fixture only; never terminate a user application.
        $process.Kill()
        throw 'Isolated task-form capture timed out.'
    }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $pngPath)) {
        throw "Task-form capture failed. Inspect $runDirectory"
    }
    [pscustomobject]@{ Image=$pngPath; Data=$dataDirectory; ExitCode=$process.ExitCode }
}
finally {
    $env:TESSEL_SETTLED_SCREENSHOT = $previousCapture
}
