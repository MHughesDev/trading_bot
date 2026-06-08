"""
API launcher for Windows compatibility with psycopg async.

Uvicorn 0.44+ on Windows defaults to ProactorEventLoop via its internal loop
factory, but psycopg async requires SelectorEventLoop. Running server.serve()
through our own asyncio.run() (after setting the policy) bypasses uvicorn's
factory so the correct loop type is used.
"""

from __future__ import annotations

import asyncio
import sys

if sys.platform == "win32":
    asyncio.set_event_loop_policy(asyncio.WindowsSelectorEventLoopPolicy())

import uvicorn

if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Trading Bot control plane API")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8000)
    parser.add_argument("--reload", action="store_true")
    args = parser.parse_args()

    config = uvicorn.Config(
        "control_plane.api:app",
        host=args.host,
        port=args.port,
        reload=args.reload,
    )
    server = uvicorn.Server(config)
    asyncio.run(server.serve())
