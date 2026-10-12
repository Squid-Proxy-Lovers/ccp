# Recorded benchmarks

These results were previously published in the README. They are historical,
machine-specific measurements, not a fresh benchmark of the current dependency
set or a production capacity guarantee. The table does not record a source
revision; rerun the harness when comparing changes.

The recorded workload used 16 concurrent mTLS clients and 1,000 requests per
client. See the [test guide](../tests/README.md#benchmarks) for locked release-build
commands, scenario selection, and reporting requirements.

Below are some of the machines we benchmarked on:

- **M2**: Apple M2, 8GB RAM, macOS
- **EPYC**: AMD EPYC 12-core, 48GB RAM, Linux (cloud VPS)

| Operation | M2 (macOS) | EPYC 12-core (Linux) | P50 (M2) | P50 (EPYC) |
| --- | --- | --- | --- | --- |
| list entries | 36,714 req/s | 65,451 req/s | 0.33ms | 0.23ms |
| get entry | 44,520 req/s | 33,993 req/s | 0.26ms | 0.36ms |
| search (simple) | 34,091 req/s | 47,301 req/s | 0.37ms | 0.29ms |
| search (complex) | 32,393 req/s | 42,844 req/s | 0.38ms | 0.31ms |
| search (miss) | 44,783 req/s | 59,543 req/s | 0.19ms | 0.24ms |
| context search (simple) | 33,287 req/s | 51,201 req/s | 0.38ms | 0.27ms |
| context search (complex) | 28,308 req/s | 51,262 req/s | 0.44ms | 0.29ms |
| context search (miss) | 44,300 req/s | 66,111 req/s | 0.22ms | 0.22ms |
| append | 28,828 req/s | 19,285 req/s | 0.25ms | 0.80ms |
| mixed (all ops) | 2,695 req/s | 2,678 req/s | 3.64ms | 1.19ms |

Run a new measurement: `cargo run --release -p ccp-tests --locked --bin benchmark -- --mode full-suite`
