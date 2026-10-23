param(
    [Parameter(Mandatory = $true)][string]$RuntimeInstaller,
    [Parameter(Mandatory = $true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$RuntimeSha256,
    [string]$OutputDirectory = ''
)
$ErrorActionPreference = 'Stop'
$campusDesktopRoot = Split-Path -Parent $PSScriptRoot
$campusProjectRoot = Split-Path -Parent $campusDesktopRoot
$campusBuildDirectory = Join-Path $campusDesktopRoot 'target\x86_64-pc-windows-msvc\release\nsis\x64'
$campusTemplatePath = Join-Path $campusBuildDirectory 'installer.nsi'
if (-not (Test-Path -LiteralPath $campusTemplatePath)) { throw '请先生成标准版安装包，保留 Tauri 生成的 NSIS 模板。' }
$campusRuntime = (Resolve-Path -LiteralPath $RuntimeInstaller).Path
$campusActualHash = (Get-FileHash -LiteralPath $campusRuntime -Algorithm SHA256).Hash
if ($campusActualHash -ne $RuntimeSha256) { throw 'WebView2 离线安装程序 SHA-256 不匹配。' }
$campusSignature = Get-AuthenticodeSignature -LiteralPath $campusRuntime
if ($campusSignature.Status -ne 'Valid' -or $campusSignature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
    throw 'WebView2 离线安装程序必须具有有效的 Microsoft Corporation 签名。'
}
$campusVersion = (Get-Content (Join-Path $campusDesktopRoot 'tauri.conf.json') -Raw | ConvertFrom-Json).version
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $campusProjectRoot '.build-work\desktop-release' }
$campusOutput = [System.IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $campusOutput | Out-Null
$campusTemplate = [System.IO.File]::ReadAllText($campusTemplatePath)
if ($campusTemplate -notmatch '(?m)^!define INSTALLWEBVIEW2MODE ""\r?$') { throw 'NSIS 输入必须来自标准版 skip 模式。' }
if ($campusRuntime.Contains('"') -or $campusRuntime.Contains("`r") -or $campusRuntime.Contains("`n")) { throw '运行时路径含无效字符。' }
# Use Tauri's already-generated, maintained offlineInstaller branch. This avoids
# the CLI's network HEAD request when its TLS redirect fails despite a verified
# official full installer already being available locally.
$campusOffline = $campusTemplate.Replace('!define INSTALLWEBVIEW2MODE ""', '!define INSTALLWEBVIEW2MODE "offlineInstaller"')
$campusOffline = $campusOffline.Replace('!define WEBVIEW2INSTALLERPATH ""', ('!define WEBVIEW2INSTALLERPATH "' + $campusRuntime + '"'))
$campusBinaryPattern = '(?m)^!define MAINBINARYSRCPATH "([^"]+)"\r?$'
$campusBinaryMatch = [regex]::Match($campusTemplate, $campusBinaryPattern)
if (-not $campusBinaryMatch.Success) { throw '无法确定 NSIS 主程序路径。' }
$campusBinarySource = $campusBinaryMatch.Groups[1].Value
$campusBinaryBytes = [System.IO.File]::ReadAllBytes($campusBinarySource)
# Tauri temporarily changes its three-byte bundle-type marker to NSS while
# bundling and restores UNK afterward. Apply the same marker to a private copy
# so the standard and cached offline installers contain identical app bytes.
$campusByteEncoding = [System.Text.Encoding]::GetEncoding(28591)
$campusBinaryText = $campusByteEncoding.GetString($campusBinaryBytes)
$campusMarker = '__TAURI_BUNDLE_TYPE_VAR_'
if ([regex]::Matches($campusBinaryText, [regex]::Escape($campusMarker)).Count -ne 1) {
    throw 'Tauri 包类型标记缺失或不唯一。'
}
if (-not $campusBinaryText.Contains($campusMarker + 'UNK') -and -not $campusBinaryText.Contains($campusMarker + 'NSS')) {
    throw 'Tauri 包类型标记不符合预期。'
}
$campusBinaryDirectory = Join-Path $campusBuildDirectory 'cached-offline-app'
New-Item -ItemType Directory -Force -Path $campusBinaryDirectory | Out-Null
$campusBinaryCopy = Join-Path $campusBinaryDirectory 'CampusAuth.exe'
[System.IO.File]::WriteAllBytes($campusBinaryCopy, $campusByteEncoding.GetBytes($campusBinaryText.Replace($campusMarker + 'UNK', $campusMarker + 'NSS')))
$campusOffline = $campusOffline.Replace($campusBinaryMatch.Value, ('!define MAINBINARYSRCPATH "' + $campusBinaryCopy + '"'))
$campusOfflinePath = Join-Path $campusBuildDirectory 'installer-cached-offline.nsi'
[System.IO.File]::WriteAllText($campusOfflinePath, $campusOffline, [System.Text.UTF8Encoding]::new($false))
$campusMakensis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
if (-not (Test-Path -LiteralPath $campusMakensis)) { throw '找不到 Tauri 缓存的 NSIS 编译器。' }
Push-Location $campusBuildDirectory
try {
    & $campusMakensis /V2 $campusOfflinePath
    if ($LASTEXITCODE -ne 0) { throw '完整离线 NSIS 安装包编译失败。' }
    $campusDestination = Join-Path $campusOutput "CampusAuth-${campusVersion}-windows-x64-full-offline.exe"
    Copy-Item -LiteralPath (Join-Path $campusBuildDirectory 'nsis-output.exe') -Destination $campusDestination
    $campusEvidence = [ordered]@{
        tool = 'Tauri generated NSIS offlineInstaller branch';
        reason = 'reuse verified official offline installer without network metadata lookup';
        runtime_path = $campusRuntime;
        runtime_sha256 = $campusActualHash.ToLowerInvariant();
        runtime_signature = $campusSignature.Status.ToString();
        runtime_signer = $campusSignature.SignerCertificate.Subject;
        application_sha256 = (Get-FileHash $campusBinaryCopy -Algorithm SHA256).Hash.ToLowerInvariant();
        output = $campusDestination;
        output_sha256 = (Get-FileHash $campusDestination -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $campusEvidence | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $campusOutput 'offline-bundle-provenance.json') -Encoding UTF8
    Write-Output $campusDestination
} finally { Pop-Location }
