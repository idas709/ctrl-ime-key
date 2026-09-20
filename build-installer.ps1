$ErrorActionPreference = "Stop"
$projectRoot = $PSScriptRoot
$manifestPath = Join-Path $projectRoot "Cargo.toml"
$installerScript = Join-Path $projectRoot "installer\ctrl-ime-key.iss"

$manifest = Get-Content -LiteralPath $manifestPath -Raw
$versionMatch = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"')
if (-not $versionMatch.Success) {
    throw "Could not read the version from Cargo.toml."
}
$appVersion = $versionMatch.Groups[1].Value

Push-Location $projectRoot
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed."
    }

    $distDir = Join-Path $projectRoot "dist"
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
    $standaloneName = "ctrl-ime-key-$appVersion-standalone-x64.exe"
    Copy-Item -LiteralPath (Join-Path $projectRoot "target\release\ctrl-ime-key.exe") `
        -Destination (Join-Path $distDir $standaloneName) -Force

    $innoRegistryKeys = @(
        "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
        "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
        "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
    )
    $registeredCompilers = Get-ItemProperty -Path $innoRegistryKeys -ErrorAction SilentlyContinue |
        Where-Object { $_.DisplayName -like "Inno Setup*" -and $_.InstallLocation } |
        ForEach-Object { Join-Path $_.InstallLocation "ISCC.exe" }

    $compilerCandidates = @(
        (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1),
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
        $registeredCompilers
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }

    $compiler = $compilerCandidates | Select-Object -First 1
    if (-not $compiler) {
        throw "Inno Setup 6 was not found. Run 'winget install JRSoftware.InnoSetup' and try again."
    }

    & $compiler "/DMyAppVersion=$appVersion" $installerScript
    if ($LASTEXITCODE -ne 0) {
        throw "Installer compilation failed."
    }

    Write-Host "Created: dist\$standaloneName"
    Write-Host "Created: dist\ctrl-ime-key-$appVersion-setup-x64.exe"
}
finally {
    Pop-Location
}
