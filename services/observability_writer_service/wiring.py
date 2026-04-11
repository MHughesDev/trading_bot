"""Dependency wiring for observability_writer_service scaffold service."""

from __future__ import annotations

from fastapi import FastAPI

from services.common import build_scaffold_app


def create_app() -> FastAPI:
    return build_scaffold_app("observability_writer_service")
