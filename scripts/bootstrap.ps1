Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-Command {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name
    )

    if (-not (Get-Command -Name $Name -ErrorAction SilentlyContinue)) {
        throw "Required command '$Name' was not found in PATH."
    }
}

Assert-Command -Name pwsh
Assert-Command -Name cargo
Assert-Command -Name cargo-clippy
Assert-Command -Name cargo-fmt
Assert-Command -Name dotnet

$pwshVersion = [Version]$PSVersionTable.PSVersion.ToString()
if ($pwshVersion.Major -lt 7) {
    throw "PowerShell 7+ is required. Current version: $pwshVersion"
}

Write-Host "Tool versions:"
Write-Host ("  cargo: " + (cargo --version))
Write-Host ("  cargo-clippy: " + (cargo clippy --version))
Write-Host ("  cargo-fmt: " + (cargo fmt --version))
Write-Host ("  dotnet: " + ((dotnet --version).Trim()))

Write-Host "Rust workspace metadata:"
cargo metadata --format-version 1 --no-deps | Out-Null
Write-Host "  cargo metadata: ok"

$uiHostProject = Join-Path $PSScriptRoot "..\ui\ui-host\Flowtile.UiHost.csproj"
if (Test-Path $uiHostProject) {
    Write-Host "UI Host project detected. Running restore..."
    dotnet restore $uiHostProject | Out-Null
    Write-Host "  dotnet restore: ok"
} else {
    Write-Warning "UI Host bootstrap is still pending. WinUI 3 / Windows App SDK setup is not touched in this script."
}

Write-Host "Bootstrap checks completed."
