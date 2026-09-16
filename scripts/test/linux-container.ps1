param(
    [string]$Image = 'rust:1.98-bookworm'
)

$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$cacheRoot = Join-Path $projectRoot '.runtime\docker-cache'
$cacheDirectories = @{
    CargoRegistry = Join-Path $cacheRoot 'cargo-registry'
    CargoGit      = Join-Path $cacheRoot 'cargo-git'
    AptLists      = Join-Path $cacheRoot 'apt-lists'
    AptArchives   = Join-Path $cacheRoot 'apt-archives'
}
foreach ($directory in $cacheDirectories.Values) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

$script = @'
set -eu
rustup component add clippy
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  build-essential \
  libwebkit2gtk-4.1-dev \
  libxdo-dev \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
cargo test --locked --workspace --exclude tinc-desktop
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p vpn-agent -p tinc-desktop
'@

$dockerArguments = @(
    'run', '--rm', '--name', 'tinc-client-linux-check',
    '--mount', "type=bind,source=$projectRoot,target=/workspace",
    '--mount', "type=bind,source=$($cacheDirectories.CargoRegistry),target=/usr/local/cargo/registry",
    '--mount', "type=bind,source=$($cacheDirectories.CargoGit),target=/usr/local/cargo/git",
    '--mount', "type=bind,source=$($cacheDirectories.AptLists),target=/var/lib/apt/lists",
    '--mount', "type=bind,source=$($cacheDirectories.AptArchives),target=/var/cache/apt/archives",
    '--workdir', '/workspace',
    '--env', 'CARGO_TARGET_DIR=/workspace/target-linux',
    $Image, 'sh', '-c', $script
)

& docker @dockerArguments
if ($LASTEXITCODE -ne 0) {
    throw "Linux container checks failed with exit code $LASTEXITCODE."
}
