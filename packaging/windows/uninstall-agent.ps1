#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'
$service = Get-Service -Name TincClientAgent -ErrorAction SilentlyContinue
if ($service) {
    Stop-Service -Name TincClientAgent
    $service.WaitForStatus('Stopped',[TimeSpan]::FromSeconds(130))
    & "$env:SystemRoot\System32\sc.exe" delete TincClientAgent
    if ($LASTEXITCODE -ne 0) { throw '删除服务注册失败。' }
}
Write-Output '服务注册已移除。程序、配置和私钥保留，未删除用户数据。'
