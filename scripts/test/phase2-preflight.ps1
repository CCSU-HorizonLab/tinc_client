param(
    [string]$ExpectedAgentPath = "",
    [string]$InstallerPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$packagePath = Join-Path $projectRoot "package.json"
$package = Get-Content -LiteralPath $packagePath -Raw -Encoding UTF8 | ConvertFrom-Json
$expectedVersion = [string]$package.version

if ([string]::IsNullOrWhiteSpace($ExpectedAgentPath)) {
    $ExpectedAgentPath = Join-Path $projectRoot "target\release\vpn-agent.exe"
}
if ([string]::IsNullOrWhiteSpace($InstallerPath)) {
    $InstallerPath = Join-Path $projectRoot "target\release\bundle\nsis\Tinc Connect_${expectedVersion}_x64-setup.exe"
}

$nativeProgramFiles = [Environment]::GetEnvironmentVariable("ProgramW6432")
if ([string]::IsNullOrWhiteSpace($nativeProgramFiles)) {
    $nativeProgramFiles = $env:ProgramFiles
}
$agentRoot = Join-Path $nativeProgramFiles "TincClientAgent"
$installedAgentPath = Join-Path $agentRoot "vpn-agent.exe"
$dataRoot = Join-Path $env:ProgramData "TincClient"
$serviceName = "TincClientAgent"
$interfaceName = "tinc"
$warnings = [Collections.Generic.List[string]]::new()

function Get-Sha256 {
    param([string]$Path)

    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            return ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function Get-FileSummary {
    param([string]$Path)

    try {
        $exists = Test-Path -LiteralPath $Path -PathType Leaf
    } catch {
        return [ordered]@{
            path = $Path
            exists = $null
            accessible = $false
            length = $null
            last_write_utc = $null
            sha256 = $null
        }
    }
    if (-not $exists) {
        return [ordered]@{
            path = $Path
            exists = $false
            accessible = $true
            length = $null
            last_write_utc = $null
            sha256 = $null
        }
    }
    $file = Get-Item -LiteralPath $Path
    return [ordered]@{
        path = $file.FullName
        exists = $true
        accessible = $true
        length = $file.Length
        last_write_utc = $file.LastWriteTimeUtc.ToString("o")
        sha256 = Get-Sha256 -Path $file.FullName
    }
}

function Get-StateFileSummary {
    param([string]$Path)

    try {
        $exists = Test-Path -LiteralPath $Path -PathType Leaf
    } catch {
        return [ordered]@{ path = $Path; exists = $null; accessible = $false; length = $null; last_write_utc = $null }
    }
    if (-not $exists) {
        return [ordered]@{ path = $Path; exists = $false; accessible = $true; length = $null; last_write_utc = $null }
    }
    $file = Get-Item -LiteralPath $Path
    return [ordered]@{
        path = $file.FullName
        exists = $true
        accessible = $true
        length = $file.Length
        last_write_utc = $file.LastWriteTimeUtc.ToString("o")
    }
}

function Get-UpgradeRequired {
    param(
        [Collections.IDictionary]$ExpectedAgent,
        [Collections.IDictionary]$InstalledAgent
    )

    if ($InstalledAgent.exists -eq $false) {
        return $true
    }
    $comparisonAvailable = $ExpectedAgent.exists -eq $true -and
        $ExpectedAgent.accessible -eq $true -and
        $InstalledAgent.exists -eq $true -and
        $InstalledAgent.accessible -eq $true
    if (-not $comparisonAvailable) {
        return $null
    }
    return $ExpectedAgent.sha256 -ne $InstalledAgent.sha256
}

$expectedAgent = Get-FileSummary -Path $ExpectedAgentPath
$installedAgent = Get-FileSummary -Path $installedAgentPath
$installer = Get-FileSummary -Path $InstallerPath
$installerSignature = "Unavailable"
if ($installer.exists) {
    try {
        $installerSignature = (Get-AuthenticodeSignature -LiteralPath $InstallerPath).Status.ToString()
    } catch {
        $warnings.Add("Unable to read the installer signature status.")
    }
}

$service = $null
$serviceQueryAvailable = $true
try {
    $service = Get-CimInstance -ClassName Win32_Service -Filter "Name='$serviceName'" -ErrorAction Stop
} catch {
    $serviceQueryAvailable = $false
    $warnings.Add("Unable to read Agent service details through CIM; retry in an elevated PowerShell.")
}
$serviceSummary = if (-not $serviceQueryAvailable) {
    [ordered]@{ exists = $null; state = $null; start_mode = $null; path_matches = $false }
} elseif ($null -eq $service) {
    [ordered]@{ exists = $false; state = $null; start_mode = $null; path_matches = $false }
} else {
    $expectedCommand = '"' + $installedAgentPath + '" --service'
    [ordered]@{
        exists = $true
        state = [string]$service.State
        start_mode = [string]$service.StartMode
        path_matches = ([string]$service.PathName).Equals($expectedCommand, [StringComparison]::OrdinalIgnoreCase)
    }
}

$adapters = @()
try {
    $adapters = @(Get-NetAdapter -Name $interfaceName -IncludeHidden -ErrorAction SilentlyContinue | ForEach-Object {
        [ordered]@{
            name = [string]$_.Name
            description = [string]$_.InterfaceDescription
            status = [string]$_.Status
            interface_guid = ([string]$_.InterfaceGuid).Trim('{', '}').ToUpperInvariant()
        }
    })
} catch {
    $warnings.Add("Unable to read the Tinc TAP adapter; retry in an elevated PowerShell.")
}

$routes = @()
try {
    $routes = @(Get-NetRoute -InterfaceAlias $interfaceName -AddressFamily IPv4 -ErrorAction SilentlyContinue | ForEach-Object {
        [ordered]@{
            destination_prefix = [string]$_.DestinationPrefix
            next_hop = [string]$_.NextHop
            route_metric = [int]$_.RouteMetric
            protocol = [string]$_.Protocol
            store = [string]$_.Store
        }
    } | Sort-Object destination_prefix, next_hop)
} catch {
    $warnings.Add("Unable to read routes for the Tinc interface; retry in an elevated PowerShell.")
}

$upgradeRequired = Get-UpgradeRequired -ExpectedAgent $expectedAgent -InstalledAgent $installedAgent
$hashMatches = $upgradeRequired -eq $false
if ($null -eq $upgradeRequired) {
    $warnings.Add("Unable to compare the installed Agent hash; upgrade requirement is unknown. Retry in an elevated PowerShell.")
}
$servicePathReady = $serviceSummary.exists -and $serviceSummary.path_matches
$tapReady = $adapters.Count -eq 1 -and
    $adapters[0].description -match '^TAP-(Windows|Win32) Adapter V9'

$report = [ordered]@{
    schema_version = 2
    collected_at_utc = (Get-Date).ToUniversalTime().ToString("o")
    client_version = $expectedVersion
    expected_agent = $expectedAgent
    installed_agent = $installedAgent
    installer = $installer
    installer_signature = $installerSignature
    service = $serviceSummary
    adapters = $adapters
    tinc_interface_routes = $routes
    state_files = [ordered]@{
        installer_log = Get-StateFileSummary -Path (Join-Path $dataRoot "installer.log")
        settings = Get-StateFileSummary -Path (Join-Path $dataRoot "settings.json")
        install_state = Get-StateFileSummary -Path (Join-Path $dataRoot "install-state.json")
        adapter_state = Get-StateFileSummary -Path (Join-Path $dataRoot "data\adapter-state.json")
    }
    upgrade_required = $upgradeRequired
    ready_for_upgrade = $installer.exists -and $expectedAgent.exists -and $serviceQueryAvailable -and
        (-not $serviceSummary.exists -or $servicePathReady)
    ready_for_vpn_test = $hashMatches -and $servicePathReady -and
        $serviceSummary.state -eq "Running" -and $tapReady
    warnings = @($warnings)
}

$report | ConvertTo-Json -Depth 8
