@echo off
setlocal
cd /d "%~dp0"

if not exist ".venv\Scripts\python.exe" (
  echo ERROR: .venv not found. Run setup.bat first.
  exit /b 1
)

set "VPY=%CD%\.venv\Scripts\python.exe"

REM Optional: override Streamlit URL if not on default port
REM set NM_STREAMLIT_DESKTOP_URL=http://127.0.0.1:8501

echo Starting desktop window (pywebview) — ensure Streamlit is already running (run.bat).
"%VPY%" -m operator_packaging.desktop_shell
exit /b %ERRORLEVEL%
