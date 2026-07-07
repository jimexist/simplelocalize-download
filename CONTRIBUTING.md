# Contributing

This is a [maturin](https://www.maturin.rs/) mixed Rust/Python project: a Rust core compiled into
the `simplelocalize_download._core` extension, plus pure-Python code (Typer CLI, logging setup)
under `python/`.

## Layout

```
Cargo.toml                       # Rust crate (cdylib + rlib)
pyproject.toml                   # maturin build + Python metadata + tooling config
src/                             # Rust source
python/simplelocalize_download/  # Python package (python-source layout)
  __init__.py                    # re-exports from ._core
  _core.pyi                      # hand-maintained stubs for the extension
  py.typed                       # PEP 561 marker
  cli.py                         # Typer app
tests/                           # pytest suite
```

## Prerequisites

- A Rust toolchain (`rustup`)
- [uv](https://docs.astral.sh/uv/)

## Common tasks

```bash
uv sync                      # create the venv and install dev + runtime deps
uv run maturin develop       # build the extension and install it editable
uv run pytest                # run the Python test suite
cargo test                   # run the Rust test suite
cargo fmt --check            # formatting
cargo clippy --all-targets -- -D warnings
uv run ruff check            # Python lint
uv run ruff format --check   # Python format
uv run mypy                  # Python type check
```

Rebuild the extension with `uv run maturin develop` after changing any Rust code before re-running
the Python tests.
