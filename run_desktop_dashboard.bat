@echo off
setlocal
cd /d "%~dp0"

if not exist ".venv\Scripts\python.exe" (
  echo ERROR: .venv not found. Run setup.bat first.
  exit /b 1
)

set "VPY=%CD%\.venv\Scripts\python.exe"

REM Optional: override the API URL shown in the pywebview window
REM set NM_DESKTOP_URL=http://127.0.0.1:8001

echo Starting desktop window (pywebview) -- ensure run.bat (or run.sh) is already running.
"%VPY%" -m operator_packaging.desktop_shell
exit /b %ERRORLEVEL%
