@echo off
setlocal
cd /d "%~dp0..\.."

if not exist ".venv\Scripts\python.exe" (
  echo ERROR: .venv not found. Run setup.bat from the repo root first.
  exit /b 1
)

set "VPY=%CD%\.venv\Scripts\python.exe"

echo Installing PyInstaller ^(build-only^) ...
"%VPY%" -m pip install "pyinstaller>=6.0" || exit /b 1

echo Building trading_bot_operator_launcher.exe ...
"%VPY%" -m PyInstaller --noconfirm --clean "packaging\windows\trading_bot_operator_launcher.spec" || exit /b 1

echo.
echo Output: dist\trading_bot_operator_launcher\trading_bot_operator_launcher.exe
echo Copy that folder next to setup.bat ^(repo root^) or run the exe from the repo root.
exit /b 0
