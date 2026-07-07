# Migrating pixai-simple-localize off the Java CLI

`pixai-simple-localize`'s `update.sh` downloads the CLI JAR, verifies its SHA-256, and runs
`java -jar … download …` twice — once for JSON, once for YAML. The **JSON** invocation can move to
`simplelocalize-download`; the **YAML** invocation must stay on the Java CLI, since this tool is
JSON-only by design.

## The JSON invocation

Today (`update.sh`):

```bash
CLI_VERSION="2.10.0"
CLI_DOWNLOAD_PATH="/tmp/simplelocalize-cli.jar"
CLI_SHA256="559d870c175f6670542f3c2d32f0648415b5b93628f133f9a4565f1361422d9a"

curl -fsSL -o "$CLI_DOWNLOAD_PATH" \
  "https://get.simplelocalize.io/binaries/${CLI_VERSION}/simplelocalize-cli-${CLI_VERSION}.jar"
# … SHA-256 verification …

java -jar "$CLI_DOWNLOAD_PATH" download \
  --apiKey "$SIMPLELOCALIZE_API_KEY" \
  --downloadFormat single-language-json \
  --downloadPath ./json/{lang}/{ns}.json \
  --downloadOptions WRITE_NESTED \
  --downloadSort LEXICOGRAPHICAL
```

After — the JAR download, checksum, and `java -jar` collapse to one command (flags are byte-for-byte
identical thanks to the camelCase aliases):

```bash
uvx simplelocalize-download download \
  --apiKey "$SIMPLELOCALIZE_API_KEY" \
  --downloadFormat single-language-json \
  --downloadPath ./json/{lang}/{ns}.json \
  --downloadOptions WRITE_NESTED \
  --downloadSort LEXICOGRAPHICAL
```

This produces the same tree (14 languages × 33 namespaces of nested, lexicographically sorted JSON)
and, unlike the Java CLI, survives transient API flakiness while still exiting non-zero on real
failures — so the existing per-download `wait "$pid"` error handling keeps working.

## What stays on the Java CLI

The YAML download is unchanged:

```bash
java -jar "$CLI_DOWNLOAD_PATH" download \
  --apiKey "$SIMPLELOCALIZE_API_KEY" \
  --downloadFormat yaml \
  --downloadPath ./yaml/{lang}/{ns}.yaml
```

If/when a YAML port is in scope, this can migrate too.

## Post-processing

`update.sh`'s downstream steps (minified copies, `_default` renaming, SHA-1 hashed CDN bundles,
`manifest.json`, S3 sync, CloudFront invalidation) are unaffected — they operate on the downloaded
tree regardless of which tool produced it. Folding the JSON post-processing into this tool is
tracked separately as an optional follow-up.

## Verification status

This migration has been validated against the end-to-end test suite (a mock SimpleLocalize server
serving a pixai-shaped tree). It has **not** been run against the live API in this change, since no
project API key was available here; run the `uvx` command above against a real project once before
switching the pipeline over.
