@echo off
setlocal enabledelayedexpansion
chcp 437 >nul
title Trading Bot

:: ----------------------------------------------------------------
::  run.bat -- one-command startup for the trading platform
::  Usage:  run.bat
::  Stop:   Ctrl+C in this window, then close the frontend window
:: ----------------------------------------------------------------

set "ROOT=%~dp0"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo ==============================================================
echo   TRADING BOT -- startup checks
echo ==============================================================
echo.

:: ── 1. Rust ───────────────────────────────────────────────────
where cargo >nul 2>nul
if errorlevel 1 (
    echo [MISSING]  Rust / cargo not found.
    echo            Install: winget install Rustlang.Rustup
    goto :fail
)
cargo --version
echo [OK]       Rust found

:: ── 2. Docker ─────────────────────────────────────────────────
where docker >nul 2>nul
if errorlevel 1 (
    echo [MISSING]  Docker not found. Install Docker Desktop.
    goto :fail
)
docker info >nul 2>nul
if errorlevel 1 (
    echo [OFFLINE]  Docker is installed but not running. Start Docker Desktop.
    goto :fail
)
docker --version
echo [OK]       Docker running

:: ── 3. sqlx-cli ───────────────────────────────────────────────
where sqlx >nul 2>nul
if errorlevel 1 (
    echo [INSTALLING] sqlx-cli not found -- installing now...
    cargo install sqlx-cli --no-default-features --features postgres
    if errorlevel 1 (
        echo [FAIL] sqlx-cli install failed.
        goto :fail
    )
)
sqlx --version
echo [OK]       sqlx-cli found

:: ── 4. .env file ──────────────────────────────────────────────
if not exist "%ROOT%.env" (
    if exist "%ROOT%.env.example" (
        copy "%ROOT%.env.example" "%ROOT%.env" >nul
        echo [SETUP]    .env created from .env.example
    ) else (
        echo [WARN]     No .env or .env.example found
    )
) else (
    echo [OK]       .env present
)

:: ── 5. Infrastructure (Docker Compose) ────────────────────────
echo.
echo -- Starting infrastructure services ---------------------
cd /d "%ROOT%"
docker compose up -d
if errorlevel 1 (
    echo [FAIL]  docker compose up failed.
    goto :fail
)

:: ── 6. Wait for services to be healthy ────────────────────────
echo.
echo -- Waiting for services to be healthy -------------------
set /a attempts=0
:wait_loop
    set /a attempts+=1
    if !attempts! gtr 60 (
        echo [FAIL]  Services not healthy after 60s. Run: docker compose logs
        goto :fail
    )

    docker compose ps 2>nul | findstr /i "starting\|unhealthy" >nul 2>nul
    if not errorlevel 1 (
        echo        Waiting... (!attempts!/60^)
        timeout /t 2 /nobreak >nul
        goto :wait_loop
    )

echo [OK]       All services healthy

:: ── 7. Database migrations ────────────────────────────────────
echo.
echo -- Running database migrations --------------------------
set DATABASE_URL=postgres://trading:trading@localhost:5432/trading
sqlx migrate run
if errorlevel 1 (
    echo [FAIL]  Migrations failed.
    goto :fail
)
echo [OK]       Migrations up to date

:: ── 8. Build ──────────────────────────────────────────────────
echo.
echo -- Building platform ------------------------------------
cargo build -p platform
if errorlevel 1 (
    echo [FAIL]  Build failed.
    goto :fail
)
echo [OK]       Build successful

:: ── 9. Launch frontend in a new window ────────────────────────
echo.
echo -- Starting React frontend ------------------------------
if not exist "%ROOT%frontend\node_modules" (
    echo [SETUP]    Installing npm dependencies...
    cd /d "%ROOT%frontend"
    npm install
    cd /d "%ROOT%"
)
start "Trading Bot - Frontend :5173" cmd /k "cd /d "%ROOT%frontend" && npm run dev"
echo [OK]       Frontend starting at http://localhost:5173

:: ── 10. Run platform ──────────────────────────────────────────
echo.
echo ==============================================================
echo   All checks passed. Starting trading platform...
echo   API:       http://localhost:8080
echo   Frontend:  http://localhost:5173  (separate window)
echo   Press Ctrl+C to stop  (close frontend window separately)
echo ==============================================================
echo.

cargo run -p platform
goto :end

:fail
echo.
echo [STOPPED]  Fix the issues above and re-run.
echo.
pause
exit /b 1

:end
endlocal
