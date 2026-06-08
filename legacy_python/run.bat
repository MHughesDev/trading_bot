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
set "NM_CONTROL_PLANE_URL=http://127.0.0.1:8001"
set "NM_AUTH_SESSION_ENABLED=true"

REM Build the React frontend if dist/ is missing or stale
if not exist "frontend\dist\index.html" (
  call :loading "Building React frontend"
  cd frontend
  call npm run build
  cd ..
)

call :loading "Spinning up Control Plane API"
REM API serves both the REST endpoints and the React SPA from frontend/dist/
start "Trading Bot API" cmd /k ""%VPY%" run_api.py --host 127.0.0.1 --port 8001"

REM Brief pause so API can bind before the browser opens
timeout /t 2 /nobreak >nul

call :loading "Starting Power Supervisor"
start "Trading Bot Supervisor" cmd /k ""%VPY%" -m app.runtime.power_supervisor"

echo.
echo Started:
echo   - Control plane + UI:  http://127.0.0.1:8001
echo   - Supervisor:          starts live runtime on port 8208 when power is ON
echo.
echo Open http://127.0.0.1:8001 in your browser to access the dashboard.
echo Close the command windows to stop the API and supervisor.
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
