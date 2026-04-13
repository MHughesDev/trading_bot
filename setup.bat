@echo off
setlocal EnableDelayedExpansion
cd /d "%~dp0"

echo === Trading Bot — setup ===
echo Repo: %CD%
echo.

REM --- Python: prefer py -3.11, then py -3, then python ---
set "PYEXE="
where py >nul 2>&1 && (
  py -3.11 --version >nul 2>&1 && set "PYEXE=py -3.11" && goto :have_py
  py -3 --version >nul 2>&1 && set "PYEXE=py -3" && goto :have_py
)
where python >nul 2>&1 && set "PYEXE=python" && goto :have_py

echo ERROR: Python 3.11+ not found. Install from https://www.python.org/downloads/
echo        Enable "Add python.exe to PATH" and "py launcher", then re-run setup.bat
exit /b 1

:have_py
echo Using: %PYEXE%
%PYEXE% --version || exit /b 1

REM --- venv ---
if not exist ".venv\Scripts\python.exe" (
  echo Creating virtual environment .venv ...
  %PYEXE% -m venv .venv || exit /b 1
) else (
  echo Virtual environment .venv already exists.
)

set "VPY=%CD%\.venv\Scripts\python.exe"
"%VPY%" -m pip install --upgrade pip wheel setuptools || exit /b 1

echo Installing package with dev + dashboard ^(Streamlit for run.bat^) ...
"%VPY%" -m pip install -e ".[dev,dashboard]" || exit /b 1

REM Optional: Alpaca paper (uncomment next line if you use paper trading)
REM "%VPY%" -m pip install -e ".[alpaca]" || exit /b 1

REM --- Docker (optional stack: QuestDB, Redis, Qdrant, etc.) ---
where docker >nul 2>&1
if errorlevel 1 (
  echo WARNING: Docker not found. Skipping `docker compose`. Install Docker Desktop if you need QuestDB/Redis/Qdrant locally.
) else (
  echo Pulling infra images ^(fetch new tags when compose.yml changes after git pull^) ...
  docker compose -f infra\docker-compose.yml pull
  if errorlevel 1 echo WARNING: docker compose pull failed — continuing; check network or Docker Desktop.
  echo Starting infra stack ^(docker compose up -d^) ...
  docker compose -f infra\docker-compose.yml up -d
  if errorlevel 1 echo WARNING: docker compose up failed — check Docker Desktop is running.
)

REM --- .env ---
if not exist ".env" (
  if exist ".env.example" (
    echo Copying .env.example to .env — edit .env with your NM_* secrets.
    copy /Y ".env.example" ".env" >nul
  ) else (
    echo NOTE: Create a .env file in the repo root ^(see README / .env.example^).
  )
) else (
  echo .env already present.
)

echo.
echo === Setup finished ===
echo Next: edit .env ^(NM_ALPACA_* for paper, NM_RISK_SIGNING_SECRET, etc.^)
echo Then run: run.bat
exit /b 0
