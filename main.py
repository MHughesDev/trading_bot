from __future__ import annotations

import uvicorn
from observability.logging import configure_logging


def main() -> None:
    configure_logging("INFO")
    uvicorn.run("control_plane.api:app", host="0.0.0.0", port=8000, reload=False)


if __name__ == "__main__":
    main()