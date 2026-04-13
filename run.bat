@echo off
setlocal
cd /d "%~dp0"

call :banner_run
if not exist ".venv\Scripts\python.exe" (
  echo ERROR: .venv not found. Run setup.bat first.
  exit /b 1
)

set "VPY=%CD%\.venv\Scripts\python.exe"
set "PATH=%CD%\.venv\Scripts;%PATH%"

call :loading "Spinning up Control Plane API"
REM Control plane API (background window)
start "Trading Bot API" cmd /k ""%VPY%" -m uvicorn control_plane.api:app --host 127.0.0.1 --port 8000"

REM Brief pause so API can bind before Streamlit calls it
timeout /t 2 /nobreak >nul

call :loading "Starting Power Supervisor"
REM Power supervisor: starts/stops Kraken live runtime (uvicorn live_service_app) when system power is ON/OFF
start "Trading Bot Supervisor" cmd /k ""%VPY%" -m app.runtime.power_supervisor"

call :loading "Launching Streamlit dashboard"
REM Dashboard (Streamlit) — main operator UI; use browser when it opens
start "Trading Bot Dashboard" cmd /k ""%VPY%" -m streamlit run control_plane\Home.py --server.headless true"

echo.
echo Started:
echo   - Control plane: http://127.0.0.1:8000
echo   - Supervisor:    starts live runtime on port 8208 when power is ON ^(set OFF in dashboard sidebar^)
echo   - Dashboard:       Streamlit will print a URL ^(usually http://localhost:8501^)
echo.
echo Close the command windows to stop API, supervisor, and dashboard.
echo To disable auto live runtime: set NM_POWER_SUPERVISOR_ENABLED=false before run.bat
exit /b 0

:banner_run
echo ===============================================
echo =   Trading Bot Launchpad                    =
echo ===============================================
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
