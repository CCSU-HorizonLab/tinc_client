# Optional workaround for Tauri's short download timeout. Uses official URLs and the same pinned hashes.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$cacheRoot = Join-Path $projectRoot 'target\.tauri'
$downloadRoot = Join-Path $projectRoot '.tools\packaging'
New-Item -ItemType Directory -Path $cacheRoot,$downloadRoot -Force | Out-Null
$assets = @(
    @{ Name='nsis.zip'; Url='https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip'; Hash='EF7FF767E5CBD9EDD22ADD3A32C9B8F4500BB10D' },
    @{ Name='nsis_tauri_utils.dll'; Url='https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll'; Hash='75197FEE3C6A814FE035788D1C34EAD39349B860' }
)
foreach($asset in $assets) {
    $destination = Join-Path $downloadRoot $asset.Name
    if (-not (Test-Path -LiteralPath $destination) -or (Get-FileHash -LiteralPath $destination -Algorithm SHA1).Hash -ne $asset.Hash) {
        Invoke-WebRequest -Uri $asset.Url -OutFile $destination -UseBasicParsing -TimeoutSec 240
    }
    if ((Get-FileHash -LiteralPath $destination -Algorithm SHA1).Hash -ne $asset.Hash) { throw 'Official build tool hash mismatch.' }
}
Expand-Archive -LiteralPath (Join-Path $downloadRoot 'nsis.zip') -DestinationPath $downloadRoot -Force
$toolRoot = Join-Path $cacheRoot 'NSIS'
New-Item -ItemType Directory -Path $toolRoot -Force | Out-Null
Get-ChildItem -LiteralPath (Join-Path $downloadRoot 'nsis-3.11') -Force | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $toolRoot -Recurse -Force }
$pluginRoot = Join-Path $toolRoot 'Plugins\x86-unicode\additional'
New-Item -ItemType Directory -Path $pluginRoot -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $downloadRoot 'nsis_tauri_utils.dll') -Destination $pluginRoot -Force
Write-Output 'Official NSIS tools cached under target/.tauri; both hashes verified.'
