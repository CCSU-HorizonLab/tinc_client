param(
    [Parameter(Mandatory = $true)]
    [string]$PayloadRoot,

    [switch]$VerifyOnly
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ServiceName = "TincClientAgent"
$InterfaceName = "tinc"
$ExpectedTincVersion = "1.0.36"
$ExpectedTincSha256 = "746999BAB0FE3839304A831A4BA021C190D8F20AF554ECC39D3B122DE3A8C21C"
$ExpectedTapSha256 = "1C44E77AB148DDFB174DE8041100EFC38C4F23500183FAFF88F9BADAC5782E3C"
$AgentRoot = Join-Path $env:ProgramFiles "TincClientAgent"
$DataRoot = Join-Path $env:ProgramData "TincClient"
$DataDirectory = Join-Path $DataRoot "data"
$SettingsPath = Join-Path $DataRoot "agent-settings.json"
$InstallStatePath = Join-Path $DataRoot "install-state.json"

$securityModule = Join-Path $PSHOME "Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1"
if (-not (Test-Path -LiteralPath $securityModule -PathType Leaf)) {
    throw "The built-in Microsoft.PowerShell.Security module is unavailable."
}
Import-Module -Name $securityModule -ErrorAction Stop

function Write-Step {
    param([string]$Message)
    Write-Host "[Tinc Connect] $Message"
}

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Resolve-PayloadFile {
    param([string]$RelativePath)

    $root = (Resolve-Path -LiteralPath $PayloadRoot).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
    $candidate = (Resolve-Path -LiteralPath (Join-Path $root $RelativePath)).Path
    $requiredPrefix = $root + [IO.Path]::DirectorySeparatorChar
    if (-not $candidate.StartsWith($requiredPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Payload path escapes the approved payload root: $RelativePath"
    }
    return $candidate
}

function Assert-FileHash {
    param(
        [string]$Path,
        [string]$ExpectedSha256,
        [string]$Label
    )

    $stream = [IO.File]::OpenRead($Path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            $actual = ([BitConverter]::ToString($sha256.ComputeHash($stream))).Replace("-", "")
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
    if ($actual -ne $ExpectedSha256.ToUpperInvariant()) {
        throw "$Label SHA256 mismatch. Expected $ExpectedSha256, got $actual"
    }
}

function Wait-ServiceState {
    param(
        [string]$Name,
        [string]$State,
        [int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $service = Get-Service -Name $Name -ErrorAction SilentlyContinue
        if ($null -ne $service -and $service.Status.ToString() -eq $State) {
            return
        }
        Start-Sleep -Milliseconds 500
    } while ((Get-Date) -lt $deadline)

    throw "Service $Name did not reach state $State within $TimeoutSeconds seconds."
}

function Get-InteractiveOwnerSid {
    $interactiveUser = (Get-CimInstance -ClassName Win32_ComputerSystem).UserName
    if ([string]::IsNullOrWhiteSpace($interactiveUser)) {
        throw "No interactive Windows user is logged on; cannot bind the local IPC identity."
    }

    $account = New-Object Security.Principal.NTAccount($interactiveUser)
    return $account.Translate([Security.Principal.SecurityIdentifier]).Value
}

function Set-ProtectedDirectoryAcl {
    param([string]$Path)

    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true, $false)
    $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
    $propagation = [Security.AccessControl.PropagationFlags]::None
    $allow = [Security.AccessControl.AccessControlType]::Allow
    $fullControl = [Security.AccessControl.FileSystemRights]::FullControl

    $systemSid = New-Object Security.Principal.SecurityIdentifier("S-1-5-18")
    $administratorsSid = New-Object Security.Principal.SecurityIdentifier("S-1-5-32-544")
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($systemSid, $fullControl, $inheritance, $propagation, $allow)))
    $acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($administratorsSid, $fullControl, $inheritance, $propagation, $allow)))
    Set-Acl -LiteralPath $Path -AclObject $acl
}

function Get-TincNamedAdapter {
    return @(Get-NetAdapter -Name $InterfaceName -IncludeHidden -ErrorAction SilentlyContinue)
}

function Get-TapAdapters {
    return @(Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue | Where-Object {
        $_.InterfaceDescription -match '^TAP-(Windows|Win32) Adapter V9'
    })
}

function Get-AdapterGuidValue {
    param([object]$Adapter)
    return ([string]$Adapter.InterfaceGuid).Trim('{', '}').ToUpperInvariant()
}

function Wait-NewTapAdapter {
    param(
        [string[]]$BeforeGuids,
        [int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $candidate = Get-TapAdapters | Where-Object { $BeforeGuids -notcontains (Get-AdapterGuidValue -Adapter $_) } | Select-Object -First 1
        if ($null -ne $candidate) {
            return $candidate
        }
        Start-Sleep -Seconds 1
    } while ((Get-Date) -lt $deadline)

    return $null
}

function Find-TapInstallTool {
    $roots = @($env:ProgramFiles)
    if (-not [string]::IsNullOrWhiteSpace(${env:ProgramFiles(x86)})) {
        $roots += ${env:ProgramFiles(x86)}
    }

    foreach ($root in $roots | Select-Object -Unique) {
        $tool = Join-Path $root "TAP-Windows\bin\tapinstall.exe"
        $driver = Join-Path $root "TAP-Windows\driver\OemVista.inf"
        if ((Test-Path -LiteralPath $tool -PathType Leaf) -and (Test-Path -LiteralPath $driver -PathType Leaf)) {
            return [pscustomobject]@{ Tool = $tool; Driver = $driver }
        }
    }

    return $null
}

function Ensure-TapAdapter {
    param([string]$TapInstaller)

    $named = Get-TincNamedAdapter
    if ($named.Count -gt 1) {
        throw "Multiple adapters are named '$InterfaceName'. Resolve the name collision before installing."
    }
    if ($named.Count -eq 1) {
        if ($named[0].InterfaceDescription -notmatch '^TAP-(Windows|Win32) Adapter V9') {
            throw "Adapter '$InterfaceName' exists but is not a TAP-Windows Adapter V9."
        }
        Write-Step "Reusing the existing TAP adapter '$InterfaceName'."
        return [pscustomobject]@{ Adapter = $named[0]; Created = $false }
    }

    $beforeGuids = @(Get-TapAdapters | ForEach-Object { Get-AdapterGuidValue -Adapter $_ })
    Write-Step "Installing the bundled, signed TAP-Windows driver."
    $tapProcess = Start-Process -FilePath $TapInstaller -ArgumentList @(
        "/S",
        "/SELECT_TAP=1",
        "/SELECT_UTILITIES=0",
        "/SELECT_SDK=0"
    ) -Wait -PassThru -WindowStyle Hidden
    if ($tapProcess.ExitCode -ne 0) {
        throw "TAP-Windows installer failed with exit code $($tapProcess.ExitCode)."
    }

    $adapter = Wait-NewTapAdapter -BeforeGuids $beforeGuids -TimeoutSeconds 15
    if ($null -eq $adapter) {
        $tapTool = Find-TapInstallTool
        if ($null -eq $tapTool) {
            throw "TAP-Windows installed, but its adapter creation utility was not found."
        }

        Write-Step "Creating a dedicated TAP adapter for Tinc Connect."
        $createProcess = Start-Process -FilePath $tapTool.Tool -ArgumentList @(
            "install",
            $tapTool.Driver,
            "tap0901"
        ) -Wait -PassThru -WindowStyle Hidden
        if ($createProcess.ExitCode -ne 0) {
            throw "TAP adapter creation failed with exit code $($createProcess.ExitCode)."
        }
        $adapter = Wait-NewTapAdapter -BeforeGuids $beforeGuids -TimeoutSeconds 30
    }

    if ($null -eq $adapter) {
        throw "The TAP-Windows driver completed, but no new TAP adapter appeared."
    }

    Rename-NetAdapter -InputObject $adapter -NewName $InterfaceName -Confirm:$false
    $renamed = Get-TincNamedAdapter
    if ($renamed.Count -ne 1) {
        throw "The TAP adapter could not be verified after renaming it to '$InterfaceName'."
    }
    Write-Step "Created TAP adapter '$InterfaceName'."
    return [pscustomobject]@{ Adapter = $renamed[0]; Created = $true }
}

function Write-JsonAtomically {
    param(
        [object]$Value,
        [string]$Path
    )

    $temporaryPath = "$Path.new"
    $Value | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $temporaryPath -Encoding UTF8
    Move-Item -LiteralPath $temporaryPath -Destination $Path -Force
}

Write-Step "Validating the offline installation payload."
$manifestPath = Resolve-PayloadFile "payload-manifest.json"
$agentSource = Resolve-PayloadFile "vpn-agent.exe"
$tincSource = Resolve-PayloadFile "tinc\tincd.exe"
$tincLicenseSource = Resolve-PayloadFile "tinc\COPYING.txt"
$tapSource = Resolve-PayloadFile "tap\tap-windows-9.24.7-I601-Win10.exe"
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json

if ($manifest.schemaVersion -ne 1) {
    throw "Unsupported payload manifest schema: $($manifest.schemaVersion)"
}
if ($manifest.tincSha256.ToUpperInvariant() -ne $ExpectedTincSha256) {
    throw "The manifest does not contain the approved Tinc binary hash."
}
if ($manifest.tapSha256.ToUpperInvariant() -ne $ExpectedTapSha256) {
    throw "The manifest does not contain the approved TAP-Windows installer hash."
}

Assert-FileHash -Path $agentSource -ExpectedSha256 $manifest.agentSha256 -Label "vpn-agent.exe"
Assert-FileHash -Path $tincSource -ExpectedSha256 $ExpectedTincSha256 -Label "tincd.exe"
Assert-FileHash -Path $tincLicenseSource -ExpectedSha256 $manifest.tincLicenseSha256 -Label "Tinc license"
Assert-FileHash -Path $tapSource -ExpectedSha256 $ExpectedTapSha256 -Label "TAP-Windows installer"

$tincVersion = (& $tincSource --version 2>&1 | Select-Object -First 1).ToString()
if ($tincVersion -notmatch [regex]::Escape($ExpectedTincVersion)) {
    throw "Expected Tinc $ExpectedTincVersion, got: $tincVersion"
}

$tapSignature = Microsoft.PowerShell.Security\Get-AuthenticodeSignature -FilePath $tapSource
if ($tapSignature.Status -ne [Management.Automation.SignatureStatus]::Valid) {
    throw "TAP-Windows installer signature is not valid: $($tapSignature.StatusMessage)"
}
if ($tapSignature.SignerCertificate.Subject -notmatch 'O=OpenVPN Inc\.') {
    throw "Unexpected TAP-Windows installer signer: $($tapSignature.SignerCertificate.Subject)"
}

Write-Step "Payload validation succeeded (Tinc $ExpectedTincVersion, signed TAP-Windows, verified Agent)."
if ($VerifyOnly) {
    exit 0
}

if (-not (Test-IsAdministrator)) {
    throw "Administrator privileges are required to install the TAP driver and Windows service."
}

$ownerSid = Get-InteractiveOwnerSid
$existingService = Get-CimInstance -ClassName Win32_Service -Filter "Name='$ServiceName'" -ErrorAction SilentlyContinue
$agentPath = Join-Path $AgentRoot "vpn-agent.exe"
$expectedServiceCommand = '"' + $agentPath + '" --service'
$serviceWasRunning = $false

if ($null -ne $existingService) {
    if ($existingService.PathName -ne $expectedServiceCommand) {
        throw "Service $ServiceName exists with an unexpected executable path: $($existingService.PathName)"
    }
    $serviceWasRunning = $existingService.State -eq "Running"
    if ($existingService.State -ne "Stopped") {
        Write-Step "Stopping the existing Agent service for an in-place upgrade."
        Stop-Service -Name $ServiceName -Force
        Wait-ServiceState -Name $ServiceName -State "Stopped"
    }
}

try {
    $tapResult = Ensure-TapAdapter -TapInstaller $tapSource

    foreach ($directory in @($AgentRoot, (Join-Path $AgentRoot "tinc"), $DataRoot, $DataDirectory)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }
    Set-ProtectedDirectoryAcl -Path $AgentRoot
    Set-ProtectedDirectoryAcl -Path $DataRoot

    Write-Step "Installing the verified Agent and Tinc runtime."
    Copy-Item -LiteralPath $agentSource -Destination "$agentPath.new" -Force
    Assert-FileHash -Path "$agentPath.new" -ExpectedSha256 $manifest.agentSha256 -Label "installed Agent staging file"
    Move-Item -LiteralPath "$agentPath.new" -Destination $agentPath -Force

    $installedTincPath = Join-Path $AgentRoot "tinc\tincd.exe"
    Copy-Item -LiteralPath $tincSource -Destination "$installedTincPath.new" -Force
    Assert-FileHash -Path "$installedTincPath.new" -ExpectedSha256 $ExpectedTincSha256 -Label "installed Tinc staging file"
    Move-Item -LiteralPath "$installedTincPath.new" -Destination $installedTincPath -Force
    Copy-Item -LiteralPath $tincLicenseSource -Destination (Join-Path $AgentRoot "tinc\COPYING.txt") -Force

    $settings = [ordered]@{
        data_dir = $DataDirectory
        tinc_binary = $installedTincPath
        interface = $InterfaceName
        interface_prefix = 32
        owner_windows_sid = $ownerSid
    }
    Write-JsonAtomically -Value $settings -Path $SettingsPath

    if ($null -eq $existingService) {
        Write-Step "Registering the Tinc Client Agent Windows service."
        New-Service -Name $ServiceName `
            -BinaryPathName $expectedServiceCommand `
            -DisplayName "Tinc Client Agent" `
            -Description "Privileged local service for Tinc Connect" `
            -StartupType Automatic | Out-Null
    } else {
        Set-Service -Name $ServiceName -StartupType Automatic
    }

    $adapterPnpId = $null
    if ($tapResult.Adapter.PSObject.Properties.Name -contains "PnPDeviceID") {
        $adapterPnpId = $tapResult.Adapter.PnPDeviceID
    }
    if ([string]::IsNullOrWhiteSpace($adapterPnpId)) {
        $adapterGuid = Get-AdapterGuidValue -Adapter $tapResult.Adapter
        $adapterCim = Get-CimInstance -ClassName Win32_NetworkAdapter | Where-Object {
            ([string]$_.GUID).Trim('{', '}').ToUpperInvariant() -eq $adapterGuid
        } | Select-Object -First 1
        if ($null -ne $adapterCim) {
            $adapterPnpId = $adapterCim.PNPDeviceID
        }
    }

    $installState = [ordered]@{
        schema_version = 1
        installed_at_utc = (Get-Date).ToUniversalTime().ToString("o")
        agent_sha256 = $manifest.agentSha256.ToUpperInvariant()
        tinc_sha256 = $ExpectedTincSha256
        tap_installer_sha256 = $ExpectedTapSha256
        adapter_name = $InterfaceName
        adapter_guid = Get-AdapterGuidValue -Adapter $tapResult.Adapter
        adapter_pnp_id = $adapterPnpId
        adapter_created_by_installer = [bool]$tapResult.Created
        owner_windows_sid = $ownerSid
    }
    Write-JsonAtomically -Value $installState -Path $InstallStatePath

    Write-Step "Starting and verifying the Agent service."
    Start-Service -Name $ServiceName
    Wait-ServiceState -Name $ServiceName -State "Running"
    Assert-FileHash -Path $agentPath -ExpectedSha256 $manifest.agentSha256 -Label "installed Agent"
    Assert-FileHash -Path $installedTincPath -ExpectedSha256 $ExpectedTincSha256 -Label "installed Tinc"

    $installedAdapter = Get-TincNamedAdapter
    if ($installedAdapter.Count -ne 1 -or $installedAdapter[0].InterfaceDescription -notmatch '^TAP-(Windows|Win32) Adapter V9') {
        throw "The dedicated '$InterfaceName' TAP adapter could not be verified after installation."
    }

    Write-Step "Installation completed: desktop prerequisites, Agent service, Tinc, and TAP adapter are ready."
} catch {
    if ($serviceWasRunning -and $null -ne (Get-Service -Name $ServiceName -ErrorAction SilentlyContinue)) {
        try {
            Start-Service -Name $ServiceName -ErrorAction SilentlyContinue
        } catch {
            Write-Warning "The previous Agent service could not be restarted after installation failure."
        }
    }
    throw
}
