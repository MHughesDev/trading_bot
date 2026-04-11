@echo off
setlocal
cd /d "%~dp0"

if not exist ".venv\Scripts\python.exe" (
  echo ERROR: .venv not found. Run setup.bat first.
  exit /b 1
)

set "VPY=%CD%\.venv\Scripts\python.exe"
set "PATH=%CD%\.venv\Scripts;%PATH%"

REM Control plane API (background window)
start "NautilusMonster API" cmd /k ""%VPY%" -m uvicorn control_plane.api:app --host 127.0.0.1 --port 8000"

REM Brief pause so API can bind before Streamlit calls it
timeout /t 2 /nobreak >nul

REM Dashboard (Streamlit) — main operator UI; use browser when it opens
start "NautilusMonster Dashboard" cmd /k ""%VPY%" -m streamlit run control_plane\Home.py --server.headless true"

echo.
echo Started:
echo   - Control plane: http://127.0.0.1:8000
echo   - Dashboard:     Streamlit will print a URL ^(usually http://localhost:8501^)
echo.
echo Close the two command windows to stop API and dashboard.
echo For the full Kraken live loop, run separately: "%VPY%" -m app.runtime.live_service
exit /b 0
