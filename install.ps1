$ErrorActionPreference = "Stop"

$Repo = "LuMiSxh/Acus"
$InstallDir = if ($env:ACUS_INSTALL_DIR) { $env:ACUS_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }
$Target = "x86_64-pc-windows-msvc"
# Choose with $env:ACUS_INSTALL_SCOPE = "user" (default, no admin needed) or "system" (all users, needs an elevated shell).
$EnvTarget = if ($env:ACUS_INSTALL_SCOPE -ieq "system") { "Machine" } else { "User" }

function Main {
    $latest = Get-LatestTag
    if (-not $latest) { Write-Error "could not determine latest release"; exit 1 }

    $url = "https://github.com/$Repo/releases/download/$latest/acus-$Target.zip"
    Write-Host "downloading acus $latest for $Target..."

    $tmp = New-TemporaryFile | ForEach-Object {
        Remove-Item $_
        New-Item -ItemType Directory -Path "$($_.FullName)_dir"
    }

    try {
        $zipPath = Join-Path $tmp.FullName "acus.zip"
        Invoke-WebRequest -Uri $url -OutFile $zipPath -UseBasicParsing
        Expand-Archive -Path $zipPath -DestinationPath $tmp.FullName -Force

        if (-not (Test-Path $InstallDir)) {
            New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        }

        Copy-Item (Join-Path $tmp.FullName "acus.exe") (Join-Path $InstallDir "acus.exe") -Force
        Write-Host "installed acus to $InstallDir\acus.exe"

        # Installs the agent skill for Claude Code and Codex; $env:ACUS_NO_SKILL = "1" skips it.
        if (-not $env:ACUS_NO_SKILL) {
            & (Join-Path $InstallDir "acus.exe") skill --install
        }

        Add-ToPath
    } finally {
        Remove-Item $tmp.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
}

function Add-ToPath {
    $existingPath = [Environment]::GetEnvironmentVariable("Path", $EnvTarget)
    if ($existingPath -like "*$InstallDir*") { return }

    try {
        [Environment]::SetEnvironmentVariable("Path", "$InstallDir;$existingPath", $EnvTarget)
        $env:Path = "$InstallDir;$env:Path"
        Write-Host "added $InstallDir to $EnvTarget PATH (open a new terminal to use it there)"
    } catch {
        Write-Host "could not update $EnvTarget PATH ($($_.Exception.Message))"
        if ($EnvTarget -eq "Machine") {
            Write-Host "system-wide install needs an elevated (Administrator) shell — re-run there, or add manually:"
        } else {
            Write-Host "add manually:"
        }
        Write-Host "  [Environment]::SetEnvironmentVariable('Path', `"$InstallDir;`$env:Path`", '$EnvTarget')"
    }
}

function Get-LatestTag {
    try {
        $r = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing
        return $r.tag_name
    } catch { return $null }
}

Main
