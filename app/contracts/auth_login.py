"""Login / session HTTP bodies (FB-UX-002)."""

from __future__ import annotations

from datetime import datetime

from pydantic import BaseModel, Field


class LoginRequest(BaseModel):
    email: str = Field(..., min_length=1, max_length=254)
    password: str = Field(..., min_length=1, max_length=1024)


class AuthUserResponse(BaseModel):
    id: int
    email: str
    created_at: datetime
