#Requires -RunAsAdministrator
param(
    [Parameter(Mandatory=$true)][string]$AgentBinary,
    [Parameter(Mandatory=$true)][string]$TincDirectory,
    [Parameter(Mandatory=$true)][string]$OwnerSid,
    [Parameter(Mandatory=$true)][string]$Interface,
    # The installer must receive the hash approved for the exact Tinc payload.
    # Do not silently trust an arbitrary executable supplied as an installer input.
    [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$TincSha256
)
$ErrorActionPreference = 'Stop'
if ($OwnerSid -notmatch '^S-1-\d+(?:-\d+)+$') { throw 'A valid interactive user SID is required.' }
if ($Interface -notmatch '^[a-zA-Z0-9_. -]{1,128}$') { throw 'The interface name contains unsupported characters or has an invalid length.' }
if (Get-Service -Name TincClientAgent -ErrorAction SilentlyContinue) { throw 'The service already exists. Stop and uninstall it first; settings and keys are preserved.' }
$sourceAgent = (Resolve-Path -LiteralPath $AgentBinary).Path
$sourceTinc = (Resolve-Path -LiteralPath $TincDirectory).Path
if (-not (Test-Path -LiteralPath $sourceAgent -PathType Leaf)) { throw 'AgentBinary must point to vpn-agent.exe.' }
$sourceTincBinary = Join-Path $sourceTinc 'tincd.exe'
if (-not (Test-Path -LiteralPath $sourceTincBinary -PathType Leaf)) { throw 'TincDirectory does not contain tincd.exe.' }
$actualTincSha256 = (Get-FileHash -LiteralPath $sourceTincBinary -Algorithm SHA256).Hash
if ($actualTincSha256 -ne $TincSha256.ToUpperInvariant()) { throw 'The tincd.exe SHA-256 does not match the installer-approved value.' }
$tincVersion = & $sourceTincBinary --version 2>&1
if ($LASTEXITCODE -ne 0 -or (($tincVersion | Out-String) -notmatch '(?im)^tinc version 1\.0\.36(?:\s|$)')) {
    throw 'This installer accepts only the reviewed tinc version 1.0.36.'
}
$adapter = Get-NetAdapter -Name $Interface -IncludeHidden -ErrorAction SilentlyContinue
if (-not $adapter) { throw "The dedicated TAP adapter '$Interface' was not found. Install the driver and create the adapter with the trusted installer first." }
if ($adapter.InterfaceDescription -notmatch '(?i)TAP-Windows Adapter V9') {
    throw "The adapter '$Interface' is not a supported TAP-Windows Adapter V9."
}
$installRoot = Join-Path $env:ProgramFiles 'TincClientAgent'
$dataRoot = Join-Path $env:ProgramData 'TincClient'
if (Test-Path -LiteralPath $installRoot) { throw 'The installation directory already exists and will not be overwritten. Review the existing installation first.' }
if (Test-Path -LiteralPath (Join-Path $dataRoot 'settings.json')) { throw 'Existing settings will not be overwritten. Follow the documented upgrade procedure.' }
New-Item -ItemType Directory -Path $installRoot,$dataRoot -Force | Out-Null
# All write access is restricted before copying executable code or storing keys.
foreach ($path in @($installRoot,$dataRoot)) {
    $acl = New-Object Security.AccessControl.DirectorySecurity
    $acl.SetAccessRuleProtection($true,$false)
    foreach ($sidText in @('S-1-5-18','S-1-5-32-544')) {
        $sidObj = New-Object Security.Principal.SecurityIdentifier($sidText)
        $rule = New-Object Security.AccessControl.FileSystemAccessRule($sidObj,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $path -AclObject $acl
}
Copy-Item -LiteralPath $sourceAgent -Destination (Join-Path $installRoot 'vpn-agent.exe')
# The approved hash covers tincd.exe only. Do not copy unverified sidecar DLLs;
# a future multi-file Tinc payload needs a separately reviewed hash manifest.
$tincRoot = Join-Path $installRoot 'tinc'
New-Item -ItemType Directory -Path $tincRoot | Out-Null
Copy-Item -LiteralPath $sourceTincBinary -Destination (Join-Path $tincRoot 'tincd.exe')
$settings = @{ data_dir = (Join-Path $dataRoot 'data'); tinc_binary = (Join-Path $tincRoot 'tincd.exe'); interface = $Interface; interface_prefix = 32; owner = $OwnerSid }
[IO.File]::WriteAllText((Join-Path $dataRoot 'settings.json'), ($settings | ConvertTo-Json), (New-Object Text.UTF8Encoding($false)))
$servicePath = '"' + (Join-Path $installRoot 'vpn-agent.exe') + '" --service'
New-Service -Name TincClientAgent -DisplayName 'Tinc Client Agent' -BinaryPathName $servicePath -StartupType Automatic -Description 'Restricted local Tinc VPN management service' | Out-Null
Start-Service -Name TincClientAgent
(Get-Service -Name TincClientAgent).WaitForStatus('Running',[TimeSpan]::FromSeconds(20))
Write-Output 'The service is installed and running. The reviewed Tinc 1.0.36 and designated TAP adapter were verified; no adapter address or route was changed.'
