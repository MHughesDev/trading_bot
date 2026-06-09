@echo off
setlocal enabledelayedexpansion
chcp 437 >nul
title Trading Bot

:: ----------------------------------------------------------------
::  run.bat -- one-command startup for the trading platform
::  Usage:  run.bat
::  Stop:   Ctrl+C in this window
:: ----------------------------------------------------------------

:: Add default Rust install location to PATH in case it's not in shell PATH
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

echo.
echo ==============================================================
echo   TRADING BOT -- startup checks
echo ==============================================================
echo.

:: ── 1. Rust ───────────────────────────────────────────────────
where cargo >nul 2>&1
if errorlevel 1 (
    echo [MISSING]  Rust / cargo not found.
    echo.
    echo   Install Rust by running this in PowerShell:
    echo     winget install Rustlang.Rustup
    echo   OR download the installer from https://rustup.rs
    echo.
    echo   After installing, close this window and run run.bat again.
    goto :fail
)
for /f "tokens=*" %%v in ('cargo --version 2^>^&1') do echo [OK]       %%v

:: ── 2. Docker ─────────────────────────────────────────────────
where docker >nul 2>&1
if errorlevel 1 (
    echo [MISSING]  Docker not found.
    echo            Install Docker Desktop from https://docker.com then re-run.
    goto :fail
)
docker info >nul 2>&1
if errorlevel 1 (
    echo [OFFLINE]  Docker is installed but not running.
    echo            Start Docker Desktop, wait for it to be ready, then re-run.
    goto :fail
)
for /f "tokens=*" %%v in ('docker --version 2^>^&1') do echo [OK]       %%v

:: ── 3. sqlx-cli ───────────────────────────────────────────────
where sqlx >nul 2>&1
if errorlevel 1 (
    echo [INSTALLING] sqlx-cli not found -- installing now (this takes a few minutes)...
    cargo install sqlx-cli --no-default-features --features postgres
    if errorlevel 1 (
        echo [FAIL] sqlx-cli install failed. Check your internet connection.
        goto :fail
    )
    echo [OK]       sqlx-cli installed
) else (
    for /f "tokens=*" %%v in ('sqlx --version 2^>^&1') do echo [OK]       %%v
)

:: ── 4. .env file ──────────────────────────────────────────────
if not exist ".env" (
    echo [SETUP]    .env not found -- copying from .env.example
    copy .env.example .env >nul
    echo [OK]       .env created. Edit it to add API keys if needed.
) else (
    echo [OK]       .env present
)

:: ── 5. Infrastructure (Docker Compose) ───────────────────────
echo.
echo -- Starting infrastructure services ---------------------
docker compose up -d
if errorlevel 1 (
    echo [FAIL]  docker compose up failed.
    goto :fail
)

:: ── 6. Wait for all services to be healthy ────────────────────
echo.
echo -- Waiting for services to be healthy -------------------
set /a attempts=0
:wait_loop
    set /a attempts+=1
    if !attempts! gtr 60 (
        echo [FAIL]  Services not healthy after 60s.
        echo         Run: docker compose logs
        goto :fail
    )

    for /f %%c in ('docker compose ps --format "{{.Health}}" 2^>nul ^| findstr /i "starting unhealthy" ^| find /c /v ""') do set unready=%%c

    if "!unready!"=="0" goto :services_ready
    echo        Waiting... (!attempts!/60)
    timeout /t 1 >nul
    goto :wait_loop

:services_ready
echo [OK]       All services healthy

:: ── 7. Database migrations ────────────────────────────────────
echo.
echo -- Running database migrations --------------------------
set DATABASE_URL=postgres://trading:trading@localhost:5432/trading
sqlx migrate run
if errorlevel 1 (
    echo [FAIL]  Migrations failed.
    echo         Check Postgres is running: docker compose ps
    goto :fail
)
echo [OK]       Migrations up to date

:: ── 8. Build (incremental -- fast after first build) ─────────
echo.
echo -- Building platform ------------------------------------
cargo build -p platform
if errorlevel 1 (
    echo [FAIL]  Build failed. Fix compile errors above then re-run.
    goto :fail
)
echo [OK]       Build successful

:: ── 9. Launch ─────────────────────────────────────────────────
echo.
echo ==============================================================
echo   All checks passed. Starting trading platform...
echo   API:       http://localhost:8080
echo   WebSocket: ws://localhost:8081
echo   Press Ctrl+C to stop
echo ==============================================================
echo.

cargo run -p platform

:fail
echo.
echo [STOPPED]  Fix the issues above and re-run.
echo.
pause
endlocal
