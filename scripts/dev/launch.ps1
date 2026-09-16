param()
# Tinc Connect Dev Launcher
# Starts vpn-agent (admin, background) + desktop UI in one click

$root = "D:\Codes\Java\KenDeJi_RuoYi\tinc_client"
$agent = "$root\target\debug\vpn-agent.exe"
$settings = "$root\.runtime\settings.json"

# Check agent binary exists
if (-not (Test-Path $agent)) {
    [System.Windows.Forms.MessageBox]::Show(
        "vpn-agent.exe not found. Please build first:`ncargo build --locked -p vpn-agent",
        "Tinc Connect", 0, 48)
    exit 1
}

# Stop only this repository's debug agent. Never terminate the installed
# TincClientAgent service, which runs a different binary from Program Files.
Get-CimInstance Win32_Process -Filter "Name = 'vpn-agent.exe'" -ErrorAction SilentlyContinue |
    Where-Object {
        $_.ExecutablePath -and
        [StringComparer]::OrdinalIgnoreCase.Equals($_.ExecutablePath, $agent) -and
        $_.CommandLine -match '(?:^|\s)--dev(?:\s|$)'
    } |
    ForEach-Object { Stop-Process -Id $_.ProcessId -Force }

# Start agent in background (hidden window)
Start-Process -FilePath $agent `
    -ArgumentList "--dev", $settings `
    -WindowStyle Hidden

# Wait a moment for agent to initialize
Start-Sleep -Seconds 1

# Load environment and start desktop UI
Push-Location $root
. "$root\scripts\dev\env.ps1"
npm.cmd run desktop:dev
Pop-Location
