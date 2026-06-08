"""Dependency wiring for control_plane_service scaffold service."""

from __future__ import annotations

from fastapi import FastAPI

from services.common import build_scaffold_app


def create_app() -> FastAPI:
    return build_scaffold_app("control_plane_service")
