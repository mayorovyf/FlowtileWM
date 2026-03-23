param(
    [switch]$RequireUiHost
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Write-Host "Running cargo fmt --check..."
cargo fmt --all --check

Write-Host "Running cargo clippy..."
cargo clippy --workspace --all-targets -- -D warnings

Write-Host "Running cargo test..."
cargo test --workspace

$uiHostProject = Join-Path $PSScriptRoot "..\ui\ui-host\Flowtile.UiHost.csproj"
if (Test-Path $uiHostProject) {
    Write-Host "Running dotnet build for UI Host..."
    dotnet build $uiHostProject -c Debug
} elseif ($RequireUiHost) {
    throw "UI Host project is required but has not been bootstrapped yet."
} else {
    Write-Warning "Skipping UI Host build because Flowtile.UiHost.csproj is not present yet."
}

Write-Host "Check completed."
