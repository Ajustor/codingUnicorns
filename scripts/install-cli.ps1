<#
.SYNOPSIS
  Installs the "cu" CLI alias for Coding Unicorns.

.DESCRIPTION
  Creates a cu.cmd wrapper next to the coding-unicorns.exe binary so that
  you can open the IDE from any terminal by typing:

      cu .              # open current directory
      cu path/to/folder # open a specific folder

  The script looks for coding-unicorns.exe in these locations (first match wins):
    1. The directory where the MSI installer placed it (Program Files)
    2. %USERPROFILE%\.cargo\bin  (cargo install)

  If the target directory is not in PATH, the script offers to add it for the
  current user.
#>
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# --- Locate the binary -------------------------------------------------------
$candidates = @(
    # MSI install
    Join-Path $env:ProgramFiles 'Coding Unicorns\coding-unicorns.exe'
    # cargo install
    Join-Path $env:USERPROFILE '.cargo\bin\coding-unicorns.exe'
)

$binaryPath = $null
foreach ($c in $candidates) {
    if (Test-Path $c) { $binaryPath = $c; break }
}

if (-not $binaryPath) {
    # Fallback: try the system PATH
    $found = Get-Command coding-unicorns -ErrorAction SilentlyContinue
    if ($found) { $binaryPath = $found.Source }
}

if (-not $binaryPath) {
    Write-Error "coding-unicorns.exe not found. Install the app first (MSI or cargo install --path .)."
    exit 1
}

$binDir = Split-Path $binaryPath

# --- Write cu.cmd -------------------------------------------------------------
$cmdPath = Join-Path $binDir 'cu.cmd'
$cmdContent = "@echo off`r`n`"%~dp0coding-unicorns.exe`" %*`r`n"
[System.IO.File]::WriteAllText($cmdPath, $cmdContent, [System.Text.Encoding]::ASCII)
Write-Host "Created $cmdPath" -ForegroundColor Green

# --- Ensure the directory is in PATH -----------------------------------------
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -split ';' | Where-Object { $_.TrimEnd('\') -eq $binDir.TrimEnd('\') }) {
    Write-Host "'$binDir' is already in your PATH." -ForegroundColor Cyan
} else {
    Write-Host "'$binDir' is not in your user PATH." -ForegroundColor Yellow
    $reply = Read-Host "Add it now? [Y/n]"
    if ($reply -match '^[Yy]?$') {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User')
        Write-Host "Added to PATH. Restart your terminal for changes to take effect." -ForegroundColor Green
    }
}

Write-Host "`nDone! You can now run:  cu <path>" -ForegroundColor Green
