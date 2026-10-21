.DEFAULT_GOAL := help
CARGO ?= cargo
PYTHON ?= python3
LINT_BIN ?= .lint-tools/bin
BASE ?= origin/main
export PATH := $(abspath $(LINT_BIN)):$(PATH)

.PHONY: check nightly-tests installer-tests quality-tests tools build test integration mcp-smoke compat coverage semver test-integrity lint lint-fast lint-rust lint-docs lint-deps lint-python lint-types lint-deadcode lint-shell lint-workflows lint-secrets security fmt hooks help

check: lint-fast quality-tests coverage integration test-integrity semver ## Run the offline gates a pull request must pass.

nightly-tests: ## Check nightly packaging, failure recovery, and channel installation.
	$(PYTHON) -m unittest discover -s tests -p 'test_nightly.py' -v

installer-tests: ## Run isolated Docker installer regressions.
	$(PYTHON) tests/docker_install.py

quality-tests: ## Run quality-tool and CLI-accounting regressions.
	$(PYTHON) -m unittest discover -s tests -p 'test_quality.py' -v
	$(PYTHON) -m unittest discover -s tests -p 'test_workspace_mcp.py' -v
	node --test tests/workspace_panel.mjs

tools: ## Install pinned quality tools in the active venv and .lint-tools/bin.
	$(PYTHON) -m pip install -r requirements-lint.txt
	$(PYTHON) scripts/install-lint-tools.py --bin-dir "$(LINT_BIN)"

build: ## Build locked release binaries.
	$(CARGO) build --workspace --release --locked

test: ## Run all Rust tests except the two opt-in load tests.
	$(CARGO) test --workspace --locked -- --test-threads=1

integration: build ## Run real CLI assertions against an isolated TLS server.
	bash tests/run.sh --skip-build

mcp-smoke: build ## Exercise real FastMCP tools through the CLI and TLS server.
	$(PYTHON) tests/mcp_smoke.py
	$(PYTHON) tests/workspace_mcp_smoke.py

compat: build ## Check TLS enrollment, client requests, and persistence reopening.
	$(PYTHON) tests/dependency_compat.py --server target/release/server --client target/release/client

coverage: ## Run Rust tests with coverage and fail below tools/coverage-baseline.json.
	$(CARGO) llvm-cov --workspace --locked --json --summary-only --output-path target/coverage-summary.json -- --test-threads=1
	$(PYTHON) scripts/check-coverage.py target/coverage-summary.json

semver: ## Check the protocol crate's public API against main.
	$(CARGO) semver-checks --package protocol --baseline-rev $(BASE)

test-integrity: ## Fail if tests or assertions were removed, or tests skipped, since main.
	$(PYTHON) scripts/check-test-integrity.py --base $(BASE)

lint: lint-fast ## Alias for the complete offline source checks.

lint-fast: fmt lint-rust lint-docs lint-deps lint-python lint-types lint-deadcode lint-shell lint-workflows lint-secrets ## Run all offline source checks; Cargo needs cached dependencies.

fmt: ## Check Rust formatting.
	$(CARGO) fmt --all -- --check

lint-rust: ## Check all Rust targets with warnings treated as errors.
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

lint-docs: ## Build Rust API docs with warnings treated as errors.
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps --locked

lint-deps: ## Reject Cargo dependencies that no crate uses.
	"$(abspath $(LINT_BIN))/cargo-machete" --skip-target-dir

lint-python: ## Check Python correctness/security rules and formatting.
	$(PYTHON) -m ruff check mcp tests scripts
	$(PYTHON) -m ruff format --check mcp tests scripts

lint-types: ## Type-check the MCP package and scripts strictly; needs the MCP package installed.
	$(PYTHON) -m mypy --strict mcp/src scripts

lint-deadcode: ## Reject unused Python code; record intentional exceptions in tools/vulture_allowlist.py.
	$(PYTHON) -m vulture mcp/src scripts tests tools/vulture_allowlist.py --min-confidence 60 --ignore-decorators "@mcp.tool,@mcp.resource"

lint-shell: ## Check every Git-visible Bash/POSIX shell script.
	$(PYTHON) scripts/check-source.py shell

lint-workflows: ## Check Actions syntax, expressions, shell snippets, and security.
	"$(abspath $(LINT_BIN))/actionlint" -pyflakes ''
	zizmor --offline --persona regular --strict-collection .github/workflows

lint-secrets: ## Scan Git-visible working files, including uncommitted changes.
	$(PYTHON) scripts/check-source.py secrets

security: ## Audit Rust policy/advisories and the installed Python environment (network required).
	$(CARGO) deny --locked check advisories licenses bans sources --hide-inclusion-graph
	$(PYTHON) scripts/audit-python.py

hooks: ## Run formatting, Python, and secret checks before each local commit.
	git config core.hooksPath .githooks

help: ## Show available checks.
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ {printf "%-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
