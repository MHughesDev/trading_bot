@echo off
setlocal
cd /d "%~dp0"

echo [doctor] Python check
py -3.12 --version >nul 2>&1
if not errorlevel 1 (
  set "PY=py -3.12"
) else (
  py -3.11 --version >nul 2>&1
  if not errorlevel 1 (
    set "PY=py -3.11"
  ) else (
    set "PY=python"
  )
)

%PY% --version || exit /b 1
%PY% -c "import sys; assert sys.version_info[:2] >= (3,11), 'Python >=3.11 required'; print('python_ok:', sys.version.split()[0])" || exit /b 1

echo.
echo [doctor] Proxy/index vars
echo HTTP_PROXY=%HTTP_PROXY%
echo HTTPS_PROXY=%HTTPS_PROXY%
echo PIP_INDEX_URL=%PIP_INDEX_URL%
echo PIP_EXTRA_INDEX_URL=%PIP_EXTRA_INDEX_URL%

echo.
echo [doctor] Package index preflight
%PY% scripts\env_preflight.py || exit /b 1

echo.
echo [doctor] Create virtualenv + install dev dependencies
%PY% -m venv .venv || exit /b 1
set "VPY=%CD%\.venv\Scripts\python.exe"
"%VPY%" -m pip install --upgrade pip setuptools wheel || exit /b 1
"%VPY%" -m pip install -e ".[dev]" || exit /b 1

echo.
echo [doctor] Module import smoke
"%VPY%" -c "import importlib,sys;mods=['pydantic','fastapi','httpx','yaml','polars','numpy','pytest','ruff','pip_audit','bandit'];missing=[m for m in mods if __import__('importlib').util.find_spec(m) is None];print('missing:', missing if missing else 'none');sys.exit(1 if missing else 0)" || exit /b 1

echo.
echo [doctor] Full local checks
"%VPY%" -m ruff check . || exit /b 1
"%VPY%" -m pytest tests/ -q || exit /b 1
bash scripts/ci_spec_compliance.sh || exit /b 1
python3 scripts/ci_queue_consistency.py || exit /b 1
bash scripts/ci_pip_audit.sh || exit /b 1
bash scripts/ci_bandit.sh || exit /b 1
bash scripts/ci_mlflow_promotion_policy.sh || exit /b 1

echo.
echo SUCCESS: environment is audit-ready.
exit /b 0
