[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
function Assert-CleanCheckout {
    $changes = @(git status --porcelain --untracked-files=all)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read checkout status.' }
    if ($changes.Count) {
        throw ("Package only a clean committed checkout. Changed paths:" + [Environment]::NewLine + ($changes -join [Environment]::NewLine))
    }
}
Push-Location $repoRoot
try {
    Assert-CleanCheckout
    $commit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read commit.' }
    $build = (git rev-list --count HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read build number.' }
    cargo build --release --locked -j 2
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    Assert-CleanCheckout
    if ((git rev-parse HEAD).Trim() -ne $commit) { throw 'Checkout changed during packaging; no archive was produced.' }
    $exe = Join-Path $repoRoot 'target\release\tessel-rust.exe'
    $identity = (& $exe --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $identity.EndsWith("b$build ($($commit.Substring(0,7)))")) { throw 'Compiled build identity does not match the checkout.' }
    $check = Start-Process -FilePath $exe -ArgumentList '--self-test-pdf' -WindowStyle Hidden -Wait -PassThru
    if ($check.ExitCode -ne 0) { throw 'Release PDF worker acceptance failed.' }
    $officeCheck = Start-Process -FilePath $exe -ArgumentList '--self-test-office' -WindowStyle Hidden -Wait -PassThru
    if ($officeCheck.ExitCode -ne 0) { throw 'Release Office worker acceptance failed.' }
    # Every package uses a new staging directory; no recursive cleanup or overwrite.
    $packageRoot = Join-Path $repoRoot 'target\packages'
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    $stage = Join-Path $packageRoot ([guid]::NewGuid().ToString())
    New-Item -ItemType Directory -Path $stage | Out-Null
    Copy-Item -LiteralPath $exe -Destination $stage
    foreach ($name in @('README.md','MIGRATION.md','LICENSE','THIRD_PARTY_NOTICES.md')) { Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination $stage }
    $binaryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $stage 'tessel-rust.exe')).Hash.ToLowerInvariant()
    [ordered]@{ commit=$commit; build="b$build"; target='x86_64-pc-windows-msvc'; sha256=$binaryHash; configuration='release'; signed=$false } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $stage 'build.json')
    "$binaryHash  tessel-rust.exe" | Set-Content -Encoding ascii (Join-Path $stage 'SHA256SUMS.txt')
    $archive = Join-Path $packageRoot "tessel-rust-b$build-$($commit.Substring(0,7))-windows-x64.zip"
    if (Test-Path -LiteralPath $archive) { throw 'This package already exists; it was not overwritten.' }
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $archive -CompressionLevel Optimal
    $verify = Join-Path $packageRoot ([guid]::NewGuid().ToString())
    Expand-Archive -LiteralPath $archive -DestinationPath $verify
    $readback = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $verify 'tessel-rust.exe')).Hash.ToLowerInvariant()
    if ($readback -ne $binaryHash) { throw 'Package readback checksum mismatch.' }
    $archiveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash.ToLowerInvariant()
    "$archiveHash  $([IO.Path]::GetFileName($archive))" | Set-Content -Encoding ascii "$archive.sha256"
    Write-Output $archive
    Write-Output "SHA256 $archiveHash"
} finally { Pop-Location }
