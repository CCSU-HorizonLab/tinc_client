param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ServiceName = "TincClientAgent"

function Get-NativeProgramFiles {
    $nativeProgramFiles = [Environment]::GetEnvironmentVariable("ProgramW6432")
    if ([string]::IsNullOrWhiteSpace($nativeProgramFiles)) {
        $nativeProgramFiles = $env:ProgramFiles
    }
    if ([string]::IsNullOrWhiteSpace($nativeProgramFiles)) {
        throw "The native Program Files directory is unavailable."
    }
    return [IO.Path]::GetFullPath($nativeProgramFiles).TrimEnd('\')
}

$AgentRoot = Join-Path (Get-NativeProgramFiles) "TincClientAgent"
$DataRoot = Join-Path $env:ProgramData "TincClient"
$InstallStatePath = Join-Path $DataRoot "install-state.json"

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-IsAdministrator)) {
    throw "Administrator privileges are required to remove the Agent service."
}

$expectedRoot = [IO.Path]::GetFullPath((Join-Path (Get-NativeProgramFiles) "TincClientAgent")).TrimEnd('\')
$resolvedRoot = [IO.Path]::GetFullPath($AgentRoot).TrimEnd('\')
if (-not $resolvedRoot.Equals($expectedRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove an unexpected Agent path: $resolvedRoot"
}

$expectedAgentPath = Join-Path $resolvedRoot "vpn-agent.exe"
$expectedCommand = '"' + $expectedAgentPath + '" --service'
$service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$ServiceName'" -ErrorAction SilentlyContinue
if ($null -ne $service) {
    if ($service.PathName -ne $expectedCommand) {
        throw "Refusing to remove service $ServiceName because its executable path is unexpected: $($service.PathName)"
    }
    if ($service.State -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force
        (Get-Service -Name $ServiceName).WaitForStatus("Stopped", [TimeSpan]::FromSeconds(30))
    }
    & sc.exe delete $ServiceName | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to delete Windows service $ServiceName."
    }
}

$installState = $null
if (Test-Path -LiteralPath $InstallStatePath -PathType Leaf) {
    $installState = Get-Content -LiteralPath $InstallStatePath -Raw -Encoding UTF8 | ConvertFrom-Json
}

if (Test-Path -LiteralPath $resolvedRoot -PathType Container) {
    Remove-Item -LiteralPath $resolvedRoot -Recurse -Force
}

if ($null -ne $installState -and $installState.adapter_created_by_installer -eq $true -and
    -not [string]::IsNullOrWhiteSpace($installState.adapter_pnp_id)) {
    $matchingAdapter = Get-CimInstance -ClassName Win32_NetworkAdapter | Where-Object {
        $_.PNPDeviceID -eq $installState.adapter_pnp_id -and $_.NetConnectionID -eq $installState.adapter_name
    } | Select-Object -First 1

    if ($null -ne $matchingAdapter) {
        & pnputil.exe /remove-device $installState.adapter_pnp_id | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "The installer-created TAP adapter could not be removed; it can be removed later in Device Manager."
        }
    }
}

# Deliberately preserve ProgramData\TincClient. It contains the user's downloaded
# network configuration, public/private key material, and diagnostics history.
Write-Host "Tinc Connect service and installed runtime removed. User network data was preserved at $DataRoot."
