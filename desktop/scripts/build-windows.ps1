param(
    [ValidateSet('standard', 'offline', 'all')]
    [string]$Variant = 'all',
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$campusDesktopRoot = Split-Path -Parent $PSScriptRoot
$campusProjectRoot = Split-Path -Parent $campusDesktopRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $campusProjectRoot '.build-work\desktop-release' }
$campusOutput = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $campusOutput | Out-Null
$campusVersion = (Get-Content -LiteralPath (Join-Path $campusDesktopRoot 'tauri.conf.json') -Raw | ConvertFrom-Json).version
$campusOriginalToolchain = $env:RUSTUP_TOOLCHAIN
try {
    $env:RUSTUP_TOOLCHAIN = 'stable-x86_64-pc-windows-msvc'
    Push-Location $campusDesktopRoot
    if ($Variant -in @('standard', 'all')) {
        & node node_modules\@tauri-apps\cli\tauri.js build --target x86_64-pc-windows-msvc --bundles nsis --ci
        if ($LASTEXITCODE -ne 0) { throw 'Windows 标准版构建失败。' }
        $campusSource = Join-Path $campusDesktopRoot "target\x86_64-pc-windows-msvc\release\bundle\nsis\CampusAuth_${campusVersion}_x64-setup.exe"
        Copy-Item -LiteralPath $campusSource -Destination (Join-Path $campusOutput "CampusAuth-${campusVersion}-windows-x64-setup.exe")
    }
    if ($Variant -eq 'offline') {
        & node node_modules\@tauri-apps\cli\tauri.js build --target x86_64-pc-windows-msvc --bundles nsis --config tauri.offline.conf.json --ci
        if ($LASTEXITCODE -ne 0) { throw 'Windows 完整离线版构建失败。' }
    } elseif ($Variant -eq 'all') {
        # Both variants use exactly the same application binary. Only the
        # installer runtime mode changes; do not rebuild the frontend twice.
        & node node_modules\@tauri-apps\cli\tauri.js bundle --target x86_64-pc-windows-msvc --bundles nsis --config tauri.offline.conf.json --ci
        if ($LASTEXITCODE -ne 0) { throw 'Windows 完整离线安装包打包失败。' }
    }
    if ($Variant -in @('offline', 'all')) {
        $campusSource = Join-Path $campusDesktopRoot "target\x86_64-pc-windows-msvc\release\bundle\nsis\CampusAuth_${campusVersion}_x64-setup.exe"
        Copy-Item -LiteralPath $campusSource -Destination (Join-Path $campusOutput "CampusAuth-${campusVersion}-windows-x64-full-offline.exe")
    }
    Get-ChildItem -LiteralPath $campusOutput -File -Filter "CampusAuth-${campusVersion}-windows-x64-*.exe" |
        ForEach-Object {
            $campusHash = Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256
            '{0}  {1}' -f $campusHash.Hash.ToLowerInvariant(), $_.Name
        } | Set-Content -LiteralPath (Join-Path $campusOutput 'SHA256SUMS-windows.txt') -Encoding ascii
    Write-Output "安装包已输出到 $campusOutput"
} finally {
    Pop-Location
    if ($null -eq $campusOriginalToolchain) { Remove-Item Env:\RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue }
    else { $env:RUSTUP_TOOLCHAIN = $campusOriginalToolchain }
}
