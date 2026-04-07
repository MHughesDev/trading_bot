from __future__ import annotations

import asyncio
from dataclasses import dataclass, field

from app.config.settings import Settings, load_settings
from app.runtime.runtime_service import NautilusRuntimeService
from observability.logging import configure_logging


@dataclass(slots=True)
class RuntimeApp:
    settings: Settings
    runtime: NautilusRuntimeService
    _started: bool = False
    _lock: asyncio.Lock = field(default_factory=asyncio.Lock)

    async def start(self) -> None:
        async with self._lock:
            if self._started:
                return
            configure_logging(self.settings.service.log_level)
            await self.runtime.start()
            self._started = True

    async def stop(self) -> None:
        async with self._lock:
            if not self._started:
                return
            await self.runtime.stop()
            self._started = False


def build_runtime_app() -> RuntimeApp:
    settings = load_settings()
    runtime = NautilusRuntimeService(settings=settings)
    return RuntimeApp(settings=settings, runtime=runtime)
