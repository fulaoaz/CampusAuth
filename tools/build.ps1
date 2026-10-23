param([ValidateSet('desktop-standard','desktop-offline','server')][string]$Target='desktop-standard')
$ErrorActionPreference='Stop'
$projectRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location -LiteralPath $projectRoot
try {
    if($Target -eq 'server') {
        & cargo build --release --locked
        if($LASTEXITCODE -ne 0){throw '服务器构建失败'}
        New-Item -ItemType Directory -Force -Path (Join-Path $projectRoot 'dist') | Out-Null
        Copy-Item -LiteralPath (Join-Path $projectRoot 'target\release\campus-auth.exe') -Destination (Join-Path $projectRoot 'dist\campus-auth-server-windows-amd64.exe')
    } else {
        Push-Location -LiteralPath (Join-Path $projectRoot 'desktop')
        try {
            $script=if($Target -eq 'desktop-offline'){'build:offline'}else{'build:standard'}
            & npm run $script
            if($LASTEXITCODE -ne 0){throw '桌面安装包构建失败'}
        } finally {Pop-Location}
    }
} finally {Pop-Location}
