@echo off
setlocal EnableDelayedExpansion
cd /d "%~dp0"

call :banner_setup
echo === Trading Bot — setup ===
echo Repo: %CD%
echo.

REM Skip Docker only (CI or no Docker): set NM_SKIP_DOCKER=1
if /i "%NM_SKIP_DOCKER%"=="1" (
  echo NM_SKIP_DOCKER=1 — will skip Docker install / compose after Python setup.
)

REM --- Python: prefer 3.12 (CI baseline), then 3.11 ---
set "PYEXE="
where py >nul 2>&1 && (
  py -3.12 --version >nul 2>&1 && set "PYEXE=py -3.12" && goto :have_py
  py -3.11 --version >nul 2>&1 && set "PYEXE=py -3.11" && goto :have_py
)
where python >nul 2>&1 && set "PYEXE=python" && goto :have_py

echo ERROR: Python 3.11+ not found. Install from https://www.python.org/downloads/
echo        Enable "Add python.exe to PATH" and "py launcher", then re-run setup.bat
exit /b 1

:have_py
echo Using: %PYEXE%
%PYEXE% --version || exit /b 1
for /f "tokens=2 delims= " %%V in ('%PYEXE% --version') do set "PYVER=%%V"
for /f "tokens=1,2 delims=." %%A in ("%PYVER%") do (
  set "PYMAJOR=%%A"
  set "PYMINOR=%%B"
)
if not "%PYMAJOR%"=="3" (
  echo ERROR: Python %PYVER% detected. Python 3.11+ is required.
  exit /b 1
)
if %PYMINOR% LSS 11 (
  echo ERROR: Python %PYVER% detected. Python 3.11+ is required.
  exit /b 1
)
if not "%PYVER:~0,4%"=="3.12" (
  echo NOTE: CI uses Python 3.12. You selected %PYVER% ^(supported^), which may differ from CI behavior.
)

REM --- venv ---
if not exist ".venv\Scripts\python.exe" (
  echo Creating virtual environment .venv ...
  %PYEXE% -m venv .venv || exit /b 1
) else (
  echo Virtual environment .venv already exists.
)

set "VPY=%CD%\.venv\Scripts\python.exe"

call :loading "Warming up package checks"
echo Running package-index preflight ...
"%VPY%" scripts\env_preflight.py || (
  echo ERROR: package index is unreachable from this environment. Resolve proxy/index settings, then retry setup.bat.
  exit /b 1
)

"%VPY%" -m pip install --upgrade pip wheel setuptools || exit /b 1

echo Installing package with dev + dashboard ^(Streamlit for run.bat^) ...
"%VPY%" -m pip install -e ".[dev,dashboard]" || (
  echo.
  echo ERROR: dependency install failed.
  echo If you are behind a proxy, verify HTTP(S)_PROXY and set PIP_INDEX_URL to a reachable package index.
  echo Then re-run setup.bat.
  exit /b 1
)

REM Optional: Alpaca paper (uncomment next line if you use paper trading)
REM "%VPY%" -m pip install -e ".[alpaca]" || exit /b 1

if /i "%NM_SKIP_DOCKER%"=="1" goto :after_docker

REM --- Docker Desktop / Engine: install if missing, wait for daemon, then compose ---
call :ensure_docker
if errorlevel 1 goto :after_docker

call :loading "Docking with Docker Engine"
echo Pulling infra images ^(fetch new tags when compose.yml changes after git pull^) ...
docker compose -f infra\docker-compose.yml pull
if errorlevel 1 echo WARNING: docker compose pull failed — check network or `docker login` for private registries.
echo Starting infra stack ^(docker compose up -d^) ...
docker compose -f infra\docker-compose.yml up -d
if errorlevel 1 echo WARNING: docker compose up failed — check Docker Desktop is running.

:after_docker
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

REM ---------------------------------------------------------------------------
REM :ensure_docker — Docker CLI present? If not, try winget or open download page.
REM Then wait for Docker Engine. Prompt user after engine is up (sign-in optional).
REM ---------------------------------------------------------------------------
:ensure_docker
where docker >nul 2>&1
if errorlevel 1 goto :docker_missing_cli

:docker_wait_engine
echo Waiting for Docker Engine ^(start Docker Desktop from the Start menu if needed^) ...
set /a _dw=0
:docker_retry
docker info >nul 2>&1
if not errorlevel 1 goto :docker_engine_ok
set /a _dw+=1
if !_dw! GEQ 80 (
  echo.
  echo Docker Engine is not responding. Start **Docker Desktop** and wait until it shows **Running**.
  echo If Docker Desktop is not installed, run this script again after installing from:
  echo   https://docs.docker.com/desktop/install/windows-install/
  echo.
  set /p _dok="Press Enter after Docker Engine is running, or type S to skip Docker for now: "
  if /i "!_dok!"=="S" exit /b 1
  set /a _dw=0
)
if exist "%ProgramFiles%\Docker\Docker\Docker Desktop.exe" (
  start "" "%ProgramFiles%\Docker\Docker\Docker Desktop.exe" 2>nul
)
timeout /t 3 /nobreak >nul
goto :docker_retry

:docker_engine_ok
echo Docker Engine is up.
echo.
echo If Docker Desktop asked you to sign in, complete that in the Docker window now.
echo Signing in is optional for **public** images ^(this project uses public pulls from Docker Hub^).
echo Private registries require `docker login`.
echo.
pause
exit /b 0

:docker_missing_cli
echo.
echo ** Docker CLI not found. This project uses Docker for QuestDB, Redis, Qdrant, etc. **
echo.
set /p _dinstall="Install Docker Desktop now via winget? [Y/n]: "
if /i "!_dinstall!"=="n" (
  echo Skipping Docker. Install Docker Desktop later, then re-run setup.bat
  exit /b 1
)
where winget >nul 2>&1
if errorlevel 1 (
  echo winget is not available. Opening the Docker Desktop download page.
  start "" "https://docs.docker.com/desktop/install/windows-install/"
  echo Install Docker Desktop, restart Windows if the installer asks, then run setup.bat again.
  pause
  exit /b 1
)
echo Running: winget install Docker.DockerDesktop ...
winget install -e --id Docker.DockerDesktop --accept-package-agreements --accept-source-agreements
if errorlevel 1 (
  echo winget install failed. Opening the download page — install manually, then re-run setup.bat.
  start "" "https://docs.docker.com/desktop/install/windows-install/"
  pause
  exit /b 1
)
echo.
echo Docker Desktop was installed or updated. **Start Docker Desktop** from the Start menu if it does not open automatically.
echo After the whale icon shows **Running**, run **setup.bat** again to pull images.
pause
exit /b 1

:banner_setup
echo ***********************************************
echo *   Trading Bot Setup Wizard                 *
echo ***********************************************
exit /b 0

:loading
set "_msg=%~1"
<nul set /p="%_msg%"
for %%G in (1 2 3) do (
  <nul set /p=" ."
  timeout /t 1 /nobreak >nul
)
echo  [OK]
exit /b 0
