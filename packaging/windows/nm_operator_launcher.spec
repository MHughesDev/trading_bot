# -*- mode: python ; coding: utf-8 -*-
# PyInstaller spec for the Windows operator launcher (FB-UI-01-01).
# Build from repo root:  packaging\windows\build_operator_bundle.bat
#
# The frozen exe does **not** bundle the venv or app code; it mirrors ``run.bat``
# by spawning ``.venv\Scripts\python.exe`` from the repo root next to ``setup.bat``.

import os

# SPEC is set by PyInstaller to this spec file path.
_spec_dir = os.path.dirname(os.path.abspath(SPEC))  # type: ignore[name-defined]
ROOT = os.path.abspath(os.path.join(_spec_dir, "..", ".."))

block_cipher = None

a = Analysis(
    [os.path.join(ROOT, "packaging", "windows", "nm_operator_launcher.py")],
    pathex=[ROOT],
    binaries=[],
    datas=[],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.zipfiles,
    a.datas,
    [],
    name="nm_operator_launcher",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
