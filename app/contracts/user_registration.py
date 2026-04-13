"""HTTP bodies for user registration (FB-UX-001)."""

from __future__ import annotations

from datetime import datetime

from pydantic import BaseModel, Field


class RegisterRequest(BaseModel):
    email: str = Field(..., min_length=1, max_length=254)
    password: str = Field(..., min_length=8, max_length=1024)


class RegisterResponse(BaseModel):
    id: int
    email: str
    created_at: datetime
    venue_keys_required: bool | None = None
    venue_keys_complete: bool | None = None
