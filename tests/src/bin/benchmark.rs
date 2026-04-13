// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::Serialize;
use tokio::time::sleep;

use ccp_tests::harness::{LoadOperation, LoadResult, TestServer, run_persistent_load};

const DEFAULT_CLIENTS: usize = 16;
const DEFAULT_REQUESTS_PER_CLIENT: usize = 1_000;
const DEFAULT_SEED_ENTRIES: usize = 12;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = BenchmarkConfig::from_args(std::env::args().skip(1).collect())?;
    let report = run_benchmark_suite(&config).await?;
    let json = serde_json::to_string_pretty(&report)?;
    let markdown = report_to_markdown(&report);

    fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("failed to create {}", config.output_dir.display()))?;
    let timestamp = unix_timestamp_string()?;
    let json_path = config
        .output_dir
        .join(format!("{timestamp}-benchmark.json"));
    let md_path = config.output_dir.join(format!("{timestamp}-benchmark.md"));
    fs::write(&json_path, json.as_bytes())
        .with_context(|| format!("failed to write {}", json_path.display()))?;
    fs::write(&md_path, markdown.as_bytes())
        .with_context(|| format!("failed to write {}", md_path.display()))?;

    println!("profile={}", report.profile_name);
    println!("clients={}", report.clients);
    println!("requests_per_client={}", report.requests_per_client);
    println!("seed_entries={}", report.seed_entries);
    println!("results_json={}", json_path.display());
    println!("results_markdown={}", md_path.display());
    println!();
    print_results_table(&report.results);

    Ok(())
}

async fn run_benchmark_suite(config: &BenchmarkConfig) -> anyhow::Result<BenchmarkReport> {
    let server = TestServer::start().await?;
    eprint!(
        "seeding {} entries... ",
        config.seed_entries.max(config.clients)
    );
    let seed_client = server.enroll_read_write().await?;
    let effective_seed_entries = config.seed_entries.max(config.clients);
    let seed_entries = seed_entries(&seed_client, effective_seed_entries).await?;
    eprintln!("done");

    let scenarios = benchmark_scenarios(config, &seed_entries);
    let total = scenarios.len();
    let mut results = Vec::with_capacity(total);

    const ENROLL_THROTTLE_THRESHOLD: usize = 100;
    let enroll_delay =
        (config.clients > ENROLL_THROTTLE_THRESHOLD).then(|| Duration::from_millis(1));

    for (idx, scenario) in scenarios.into_iter().enumerate() {
        eprint!(
            "[{}/{}] {} — enrolling {} clients... ",
            idx + 1,
            total,
            scenario.name,
            config.clients
        );
        let mut clients = Vec::with_capacity(config.clients);
        for i in 0..config.clients {
            if i > 0
                && let Some(d) = enroll_delay
            {
                sleep(d).await;
            }
            let client = if scenario.requires_write {
                server.enroll_read_write().await?
            } else {
                server.enroll_read().await?
            };
            clients.push(client);
        }

        eprint!("running... ");
        let result =
            run_persistent_load(clients, config.requests_per_client, scenario.operation).await?;
        eprintln!(
            "{:.0} req/s (p50={:.2}ms)",
            result.requests_per_second, result.p50_ms
        );
        results.push(BenchmarkScenarioResult::from_load_result(
            &scenario.name,
            &scenario.description,
            result,
        ));
    }

    Ok(BenchmarkReport {
        profile_name: config.mode.clone(),
        clients: config.clients,
        requests_per_client: config.requests_per_client,
        seed_entries: effective_seed_entries,
        results,
    })
}

async fn seed_entries(
    client: &ccp_tests::harness::EnrolledClient,
    count: usize,
) -> anyhow::Result<Vec<String>> {
    let mut entry_names = Vec::with_capacity(count);
    for index in 0..count {
        let name = format!("bench-entry-{index:02}");
        let role_label = if index % 2 == 0 { "read" } else { "write" };
        let topic_label = if index % 3 == 0 {
            "protocol"
        } else {
            "context"
        };
        let nonsense = nonsense_blob(index);
        let labels = vec![
            "benchmark".to_string(),
            role_label.to_string(),
            topic_label.to_string(),
            format!("cluster-{}", index % 4),
        ];
        let description = format!("benchmark target {index}");
        let context = format!(
            "seed context for {name} with tls framing and {topic_label} coordination {nonsense}"
        );
        client
            .add_with_labels(&name, &description, &labels, &context)
            .await
            .with_context(|| format!("failed to seed {name}"))?;

        if index % 4 == 0 {
            client
                .append(&name, &format!("warm append {index}"))
                .await
                .with_context(|| format!("failed to warm append {name}"))?;
        }

        entry_names.push(name);
    }
    Ok(entry_names)
}

fn benchmark_scenarios(config: &BenchmarkConfig, seeded_entries: &[String]) -> Vec<Scenario> {
    let entry_names = seeded_entries.to_vec();

    let all = vec![
        Scenario {
            name: "list".to_string(),
            description: "List entry summaries repeatedly over persistent mTLS sessions."
                .to_string(),
            requires_write: false,
            operation: LoadOperation::List,
        },
        Scenario {
            name: "get".to_string(),
            description: "Fetch full entries repeatedly over persistent mTLS sessions.".to_string(),
            requires_write: false,
            operation: LoadOperation::Get {
                entry_names: entry_names.clone(),
            },
        },
        Scenario {
            name: "search-entries-simple".to_string(),
            description: "Search name, description, and labels for a simple hit.".to_string(),
            requires_write: false,
            operation: LoadOperation::SearchEntries {
                query: "benchmark".to_string(),
            },
        },
        Scenario {
            name: "search-entries-complex".to_string(),
            description: "Search name, description, and labels for a multi-term hit.".to_string(),
            requires_write: false,
            operation: LoadOperation::SearchEntries {
                query: "benchmark protocol".to_string(),
            },
        },
        Scenario {
            name: "search-entries-miss".to_string(),
            description: "Search name, description, and labels for a guaranteed miss.".to_string(),
            requires_write: false,
            operation: LoadOperation::SearchEntries {
                query: "voidneedleabsent".to_string(),
            },
        },
        Scenario {
            name: "search-context-simple".to_string(),
            description: "Search context text for a simple hit and return snippets.".to_string(),
            requires_write: false,
            operation: LoadOperation::SearchContext {
                query: "tls framing".to_string(),
            },
        },
        Scenario {
            name: "search-context-complex".to_string(),
            description: "Search context text for a multi-term hit in generated nonsense."
                .to_string(),
            requires_write: false,
            operation: LoadOperation::SearchContext {
                query: "glorbax zenthar".to_string(),
            },
        },
        Scenario {
            name: "search-context-miss".to_string(),
            description: "Search context text for a guaranteed miss.".to_string(),
            requires_write: false,
            operation: LoadOperation::SearchContext {
                query: "voidneedleabsent".to_string(),
            },
        },
        Scenario {
            name: "append".to_string(),
            description: "Append to a single hot entry from many concurrent clients.".to_string(),
            requires_write: true,
            operation: LoadOperation::Append {
                entry_name: entry_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "bench-entry-00".to_string()),
                prefix: "benchmark-append".to_string(),
            },
        },
        Scenario {
            name: "mixed".to_string(),
            description:
                "Mixed workload: list, get, simple/complex search, miss-path search, and append."
                    .to_string(),
            requires_write: true,
            operation: LoadOperation::Mixed {
                entry_names,
                label_query: "protocol".to_string(),
                complex_label_query: "benchmark protocol".to_string(),
                context_query: "tls framing".to_string(),
                complex_context_query: "glorbax zenthar".to_string(),
                nonsense_query: "voidneedleabsent".to_string(),
                prefix: "benchmark-mixed".to_string(),
            },
        },
    ];

    match config.mode.as_str() {
        "suite" | "full-suite" => all,
        single => all
            .into_iter()
            .filter(|scenario| scenario.name == single)
            .collect(),
    }
}

fn report_to_markdown(report: &BenchmarkReport) -> String {
    let mut lines = vec![
        "# CCP Benchmark Report".to_string(),
        String::new(),
        format!("- profile: `{}`", report.profile_name),
        format!("- clients: `{}`", report.clients),
        format!("- requests_per_client: `{}`", report.requests_per_client),
        format!("- seed_entries: `{}`", report.seed_entries),
        String::new(),
        "| Scenario | Requests | Elapsed ms | Req/s | P50 ms | P95 ms | P99 ms |".to_string(),
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: |".to_string(),
    ];

    for result in &report.results {
        lines.push(format!(
            "| {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |",
            result.name,
            result.total_requests,
            result.elapsed_ms,
            result.requests_per_second,
            result.p50_ms,
            result.p95_ms,
            result.p99_ms
        ));
    }

    lines.push(String::new());
    for result in &report.results {
        lines.push(format!("## {}", result.name));
        lines.push(String::new());
        lines.push(result.description.clone());
        lines.push(String::new());
    }

    format!("{}\n", lines.join("\n"))
}

fn print_results_table(results: &[BenchmarkScenarioResult]) {
    let scenario_width = results
        .iter()
        .map(|result| result.name.len())
        .max()
        .unwrap_or(8)
        .max("Scenario".len());
    let requests_width = results
        .iter()
        .map(|result| result.total_requests.to_string().len())
        .max()
        .unwrap_or(8)
        .max("Requests".len());
    let elapsed_width = results
        .iter()
        .map(|result| format!("{:.2}", result.elapsed_ms).len())
        .max()
        .unwrap_or(10)
        .max("Elapsed ms".len());
    let rps_width = results
        .iter()
        .map(|result| format!("{:.2}", result.requests_per_second).len())
        .max()
        .unwrap_or(8)
        .max("Req/s".len());
    let p50_width = results
        .iter()
        .map(|result| format!("{:.2}", result.p50_ms).len())
        .max()
        .unwrap_or(6)
        .max("P50 ms".len());
    let p95_width = results
        .iter()
        .map(|result| format!("{:.2}", result.p95_ms).len())
        .max()
        .unwrap_or(6)
        .max("P95 ms".len());
    let p99_width = results
        .iter()
        .map(|result| format!("{:.2}", result.p99_ms).len())
        .max()
        .unwrap_or(6)
        .max("P99 ms".len());

    let separator = format!(
        "+-{:-<scenario$}-+-{:-<requests$}-+-{:-<elapsed$}-+-{:-<rps$}-+-{:-<p50$}-+-{:-<p95$}-+-{:-<p99$}-+",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        scenario = scenario_width,
        requests = requests_width,
        elapsed = elapsed_width,
        rps = rps_width,
        p50 = p50_width,
        p95 = p95_width,
        p99 = p99_width
    );

    println!("{separator}");
    println!(
        "| {scenario:<scenario_width$} | {requests:>requests_width$} | {elapsed:>elapsed_width$} | {rps:>rps_width$} | {p50:>p50_width$} | {p95:>p95_width$} | {p99:>p99_width$} |",
        scenario = "Scenario",
        requests = "Requests",
        elapsed = "Elapsed ms",
        rps = "Req/s",
        p50 = "P50 ms",
        p95 = "P95 ms",
        p99 = "P99 ms",
        scenario_width = scenario_width,
        requests_width = requests_width,
        elapsed_width = elapsed_width,
        rps_width = rps_width,
        p50_width = p50_width,
        p95_width = p95_width,
        p99_width = p99_width
    );
    println!("{separator}");

    for result in results {
        println!(
            "| {scenario:<scenario_width$} | {requests:>requests_width$} | {elapsed:>elapsed_width$.2} | {rps:>rps_width$.2} | {p50:>p50_width$.2} | {p95:>p95_width$.2} | {p99:>p99_width$.2} |",
            scenario = result.name,
            requests = result.total_requests,
            elapsed = result.elapsed_ms,
            rps = result.requests_per_second,
            p50 = result.p50_ms,
            p95 = result.p95_ms,
            p99 = result.p99_ms,
            scenario_width = scenario_width,
            requests_width = requests_width,
            elapsed_width = elapsed_width,
            rps_width = rps_width,
            p50_width = p50_width,
            p95_width = p95_width,
            p99_width = p99_width
        );
    }
    println!("{separator}");
}

fn unix_timestamp_string() -> anyhow::Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before UNIX_EPOCH")?
        .as_secs()
        .to_string())
}

struct BenchmarkConfig {
    mode: String,
    clients: usize,
    requests_per_client: usize,
    seed_entries: usize,
    output_dir: PathBuf,
}

impl BenchmarkConfig {
    fn from_args(args: Vec<String>) -> anyhow::Result<Self> {
        let mut mode = "suite".to_string();
        let mut clients = DEFAULT_CLIENTS;
        let mut requests_per_client = DEFAULT_REQUESTS_PER_CLIENT;
        let mut seed_entries = DEFAULT_SEED_ENTRIES;
        let mut output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benchmark-results");
        let mut iter = args.into_iter();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--mode" => mode = iter.next().context("missing value for --mode")?,
                "--clients" => {
                    clients = iter
                        .next()
                        .context("missing value for --clients")?
                        .parse()
                        .context("invalid --clients value")?;
                }
                "--requests-per-client" => {
                    requests_per_client = iter
                        .next()
                        .context("missing value for --requests-per-client")?
                        .parse()
                        .context("invalid --requests-per-client value")?;
                }
                "--seed-entries" => {
                    seed_entries = iter
                        .next()
                        .context("missing value for --seed-entries")?
                        .parse()
                        .context("invalid --seed-entries value")?;
                }
                "--output-dir" => {
                    output_dir =
                        PathBuf::from(iter.next().context("missing value for --output-dir")?);
                }
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }

        if !matches!(
            mode.as_str(),
            "suite"
                | "full-suite"
                | "list"
                | "get"
                | "search-entries-simple"
                | "search-entries-complex"
                | "search-entries-miss"
                | "search-context-simple"
                | "search-context-complex"
                | "search-context-miss"
                | "append"
                | "mixed"
        ) {
            bail!(
                "--mode must be one of: suite, full-suite, list, get, search-entries-simple, search-entries-complex, search-entries-miss, search-context-simple, search-context-complex, search-context-miss, append, mixed"
            );
        }
        if clients == 0 {
            bail!("--clients must be greater than 0");
        }
        if requests_per_client == 0 {
            bail!("--requests-per-client must be greater than 0");
        }
        if seed_entries == 0 {
            bail!("--seed-entries must be greater than 0");
        }

        Ok(Self {
            mode,
            clients,
            requests_per_client,
            seed_entries,
            output_dir,
        })
    }
}

#[derive(Clone)]
struct Scenario {
    name: String,
    description: String,
    requires_write: bool,
    operation: LoadOperation,
}

#[derive(Serialize)]
struct BenchmarkReport {
    profile_name: String,
    clients: usize,
    requests_per_client: usize,
    seed_entries: usize,
    results: Vec<BenchmarkScenarioResult>,
}

#[derive(Serialize)]
struct BenchmarkScenarioResult {
    name: String,
    description: String,
    total_requests: usize,
    elapsed_ms: f64,
    requests_per_second: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

impl BenchmarkScenarioResult {
    fn from_load_result(name: &str, description: &str, result: LoadResult) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            total_requests: result.total_requests,
            elapsed_ms: result.elapsed.as_secs_f64() * 1000.0,
            requests_per_second: result.requests_per_second,
            p50_ms: result.p50_ms,
            p95_ms: result.p95_ms,
            p99_ms: result.p99_ms,
        }
    }
}

fn print_usage() {
    eprintln!("usage: cargo run --manifest-path src/tests/Cargo.toml --bin benchmark -- [options]");
    eprintln!(
        "  --mode <suite|full-suite|list|get|search-entries-simple|search-entries-complex|search-entries-miss|search-context-simple|search-context-complex|search-context-miss|append|mixed>"
    );
    eprintln!(
        "  --clients <count>                 Standardized concurrent clients. Default: {DEFAULT_CLIENTS}"
    );
    eprintln!(
        "  --requests-per-client <count>     Standardized requests per client. Default: {DEFAULT_REQUESTS_PER_CLIENT}"
    );
    eprintln!(
        "  --seed-entries <count>            Seed entries created before the suite. Default: {DEFAULT_SEED_ENTRIES}"
    );
    eprintln!(
        "  --output-dir <path>               Write JSON and Markdown reports here. Default: src/tests/benchmark-results"
    );
}

fn nonsense_blob(index: usize) -> String {
    const SYLLABLES: &[&str] = &[
        "glorbax", "zenthar", "quellix", "morbidra", "thalnix", "vorrin", "kelthar", "jandor",
        "plexith", "ormexa", "nivor", "xaleth",
    ];
    (0..24)
        .map(|offset| SYLLABLES[(index + offset) % SYLLABLES.len()])
        .collect::<Vec<_>>()
        .join(" ")
}
