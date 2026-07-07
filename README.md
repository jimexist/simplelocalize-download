# simplelocalize-download

A resilient, Rust-powered port of the [SimpleLocalize CLI](https://github.com/simplelocalize/simplelocalize-cli)
`download` command, shipped as a Python package with a [Typer](https://typer.tiangolo.com/) CLI.

Scope by design: the **`download`** command, **JSON only** (`single-language-json`). The core is
written in Rust (tokio + reqwest) for concurrent, retrying, atomic downloads and exposed to Python
via [PyO3](https://pyo3.rs/)/[maturin](https://www.maturin.rs/).

> Early development. See the [tracking issue](https://github.com/jimexist/simplelocalize-download/issues/9)
> for the roadmap. Full usage docs land with the first release.

## Development

```bash
uv sync
uv run maturin develop
uv run pytest
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for details.
