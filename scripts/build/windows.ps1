param(
    [string]$TincDirectory = "",
    [string]$TapInstaller = ""
)

# Build a self-contained Windows installer: desktop UI, privileged Agent,
# approved Tinc runtime, and the signed TAP-Windows driver installer.
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'This packaging entry point requires Windows.' }
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
. (Join-Path $projectRoot 'scripts\dev\env.ps1')
$expectedTincSha256 = '746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C'
$expectedTapSha256 = '1C44E77AB148DDFB174DE8041100EFC38C4F23500183FAFF88F9BADAC5782E3C'
$tapDownloadUrl = 'https://build.openvpn.net/downloads/releases/tap-windows-9.24.7-I601-Win10.exe'
$securityModule = Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1'
if (-not (Test-Path -LiteralPath $securityModule -PathType Leaf)) {
    throw 'The built-in Microsoft.PowerShell.Security module is unavailable.'
}
Import-Module -Name $securityModule -ErrorAction Stop

function Assert-Sha256 {
    param([string]$Path, [string]$Expected, [string]$Label)
    $actual = Get-Sha256 -Path $Path
    if ($actual -ne $Expected.ToUpperInvariant()) {
        throw "$Label SHA256 mismatch. Expected $Expected, got $actual"
    }
}

function Get-Sha256 {
    param([string]$Path)
    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '')
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

Push-Location -LiteralPath $projectRoot
try {
    & npm.cmd run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }

    # Read Cargo's actual executable path rather than assuming target/release.
    $cargoOutput = & cargo build --locked --release -p vpn-agent -p tinc-desktop --message-format=json-render-diagnostics
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    $artifacts = @($cargoOutput | ForEach-Object { ConvertFrom-Json -InputObject $_ })
    $desktop = $artifacts | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'tinc-desktop' -and $_.executable } | Select-Object -Last 1
    $agent = $artifacts | Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'vpn-agent' -and $_.executable } | Select-Object -Last 1
    if (-not $desktop) { throw 'Cargo did not report a desktop executable.' }
    if (-not $agent) { throw 'Cargo did not report an Agent executable.' }

    if ([string]::IsNullOrWhiteSpace($TincDirectory)) {
        if (-not [string]::IsNullOrWhiteSpace($env:TINC_CLIENT_TINC_DIRECTORY)) {
            $TincDirectory = $env:TINC_CLIENT_TINC_DIRECTORY
        } else {
            $TincDirectory = Join-Path (Split-Path -Parent $projectRoot) 'tinc_cli_gui\windows\Tinc'
        }
    }
    $tincBinary = Join-Path $TincDirectory 'tincd.exe'
    $tincLicense = Join-Path $TincDirectory 'COPYING.txt'
    if (-not (Test-Path -LiteralPath $tincBinary -PathType Leaf)) { throw "Approved Tinc binary not found: $tincBinary" }
    if (-not (Test-Path -LiteralPath $tincLicense -PathType Leaf)) { throw "Tinc license not found: $tincLicense" }
    Assert-Sha256 -Path $tincBinary -Expected $expectedTincSha256 -Label 'Tinc'
    $tincVersion = & $tincBinary --version 2>&1
    if ($LASTEXITCODE -ne 0 -or (($tincVersion | Out-String) -notmatch '(?im)^tinc version 1\.0\.36(?:\s|$)')) {
        throw 'Only the approved Tinc 1.0.36 runtime can be bundled.'
    }

    $buildRoot = Join-Path $projectRoot '.runtime\build'
    $dependencyRoot = Join-Path $projectRoot '.runtime\build-deps'
    New-Item -ItemType Directory -Path $buildRoot -Force | Out-Null
    New-Item -ItemType Directory -Path $dependencyRoot -Force | Out-Null
    if ([string]::IsNullOrWhiteSpace($TapInstaller)) {
        if (-not [string]::IsNullOrWhiteSpace($env:TINC_CLIENT_TAP_INSTALLER)) {
            $TapInstaller = $env:TINC_CLIENT_TAP_INSTALLER
        } else {
            $TapInstaller = Join-Path $dependencyRoot 'tap-windows-9.24.7-I601-Win10.exe'
        }
    }
    if (-not (Test-Path -LiteralPath $TapInstaller -PathType Leaf)) {
        Write-Output "Downloading the pinned TAP-Windows installer from $tapDownloadUrl"
        Invoke-WebRequest -Uri $tapDownloadUrl -OutFile $TapInstaller -UseBasicParsing
    }
    Assert-Sha256 -Path $TapInstaller -Expected $expectedTapSha256 -Label 'TAP-Windows installer'
    $tapSignature = Microsoft.PowerShell.Security\Get-AuthenticodeSignature -FilePath $TapInstaller
    if ($tapSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
        $tapSignature.SignerCertificate.Subject -notmatch 'O=OpenVPN Inc\.') {
        throw "TAP-Windows Authenticode verification failed: $($tapSignature.Status), $($tapSignature.SignerCertificate.Subject)"
    }

    $payloadRoot = Join-Path $buildRoot 'windows-payload'
    $expectedPayloadRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot '.runtime\build\windows-payload')).TrimEnd('\')
    $resolvedPayloadRoot = [IO.Path]::GetFullPath($payloadRoot).TrimEnd('\')
    if (-not $resolvedPayloadRoot.Equals($expectedPayloadRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to recreate an unexpected payload path: $resolvedPayloadRoot"
    }
    if (Test-Path -LiteralPath $payloadRoot) {
        Remove-Item -LiteralPath $payloadRoot -Recurse -Force
    }
    New-Item -ItemType Directory -Path (Join-Path $payloadRoot 'tinc') -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $payloadRoot 'tap') -Force | Out-Null
    Copy-Item -LiteralPath $agent.executable -Destination (Join-Path $payloadRoot 'vpn-agent.exe')
    Copy-Item -LiteralPath $tincBinary -Destination (Join-Path $payloadRoot 'tinc\tincd.exe')
    Copy-Item -LiteralPath $tincLicense -Destination (Join-Path $payloadRoot 'tinc\COPYING.txt')
    Copy-Item -LiteralPath $TapInstaller -Destination (Join-Path $payloadRoot 'tap\tap-windows-9.24.7-I601-Win10.exe')

    $manifestPath = Join-Path $payloadRoot 'payload-manifest.json'
    $manifest = [ordered]@{
        schemaVersion = 1
        agentSha256 = Get-Sha256 -Path $agent.executable
        tincSha256 = $expectedTincSha256
        tincLicenseSha256 = Get-Sha256 -Path $tincLicense
        tapSha256 = $expectedTapSha256
    } | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($manifestPath, $manifest, (New-Object Text.UTF8Encoding($false)))

    & (Join-Path $projectRoot 'packaging\windows\install-client.ps1') -PayloadRoot $payloadRoot -VerifyOnly
    if ($LASTEXITCODE -ne 0) { throw 'Offline payload verification failed.' }

    # GNU builds dynamically import this DLL. MSVC builds normally link it statically.
    # tauri-build copies it beside the executable, but NSIS does not add it automatically.
    $loaderPath = Join-Path (Split-Path -Parent $desktop.executable) 'WebView2Loader.dll'
    $resources = @{
        (Join-Path $payloadRoot 'vpn-agent.exe') = 'payload/vpn-agent.exe'
        (Join-Path $payloadRoot 'tinc\tincd.exe') = 'payload/tinc/tincd.exe'
        (Join-Path $payloadRoot 'tinc\COPYING.txt') = 'payload/tinc/COPYING.txt'
        (Join-Path $payloadRoot 'tap\tap-windows-9.24.7-I601-Win10.exe') = 'payload/tap/tap-windows-9.24.7-I601-Win10.exe'
        $manifestPath = 'payload/payload-manifest.json'
        (Join-Path $projectRoot 'packaging\windows\install-client.ps1') = 'payload/install-client.ps1'
        (Join-Path $projectRoot 'packaging\windows\uninstall-client.ps1') = 'payload/uninstall-client.ps1'
    }
    if (Test-Path -LiteralPath $loaderPath) {
        $resources[$loaderPath] = 'WebView2Loader.dll'
        Write-Output 'Including WebView2Loader.dll beside the installed desktop executable.'
    }
    $configPath = Join-Path $buildRoot 'windows-bundle.json'
    $config = @{ bundle = @{ resources = $resources } } | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText($configPath, $config, (New-Object Text.UTF8Encoding($false)))

    & npm.cmd run tauri --workspace apps/desktop -- build --bundles nsis --config $configPath
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer build failed.' }
}
finally {
    Pop-Location
}
