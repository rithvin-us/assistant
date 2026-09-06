# Runs the assistant server with the repository-root .env loaded.
#   pwsh scripts/dev-server.ps1
$ErrorActionPreference = "Stop"
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if (-not (Test-Path ".env")) {
        Write-Error "No .env found. Copy .env.example to .env first."
    }
    cargo run -p assistant-server
}
finally { Pop-Location }
