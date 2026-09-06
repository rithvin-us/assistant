# Applies migrations/ to the database in DATABASE_URL.
# Requires sqlx-cli:  cargo install sqlx-cli --no-default-features --features postgres,rustls
#   pwsh scripts/migrate.ps1
$ErrorActionPreference = "Stop"
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if (-not $env:DATABASE_URL) {
        Write-Error "DATABASE_URL is not set. Set it, or load it from .env, before running migrations."
    }
    sqlx migrate run --source migrations
}
finally { Pop-Location }
