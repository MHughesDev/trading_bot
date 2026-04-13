# Graceful shutdown — live service

`python -m app.runtime.live_service` registers **SIGINT** and **SIGTERM** on the asyncio loop (Unix). On signal, `stop_event` is set and the WS loop exits; memory task is cancelled; QuestDB connection closed.

**Flatten on exit:** not automatic — use control plane `POST /flatten` or set system mode before shutdown if policy requires it.
