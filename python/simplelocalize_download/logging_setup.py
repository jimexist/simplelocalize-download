"""loguru logging with a bridge from the standard library.

Rust `log` records are forwarded to Python's ``logging`` by ``pyo3-log`` (see the
Rust ``_core`` module); this module routes all stdlib ``logging`` records —
including those — into loguru so Rust and Python logs share one format.
"""

from __future__ import annotations

import logging
import sys
from types import FrameType

from loguru import logger

_LOG_FORMAT = (
    "<green>{time:HH:mm:ss}</green> "
    "<level>{level: <7}</level> "
    "<cyan>{name}</cyan> - <level>{message}</level>"
)


class InterceptHandler(logging.Handler):
    """A stdlib logging handler that re-emits every record through loguru."""

    def emit(self, record: logging.LogRecord) -> None:
        level: str | int
        try:
            level = logger.level(record.levelname).name
        except ValueError:
            level = record.levelno

        frame: FrameType | None = logging.currentframe()
        depth = 2
        while frame is not None and frame.f_code.co_filename == logging.__file__:
            frame = frame.f_back
            depth += 1

        logger.opt(depth=depth, exception=record.exc_info).log(level, record.getMessage())


def configure_logging(level: str) -> None:
    """Direct loguru to stderr and route stdlib logging (incl. Rust) through it."""
    logger.remove()
    logger.add(
        sys.stderr,
        level=level,
        format=_LOG_FORMAT,
        colorize=True,
        backtrace=False,
        diagnose=False,
    )
    logging.basicConfig(handlers=[InterceptHandler()], level=logging.NOTSET, force=True)
    logging.getLogger("simplelocalize_download").setLevel(level)
