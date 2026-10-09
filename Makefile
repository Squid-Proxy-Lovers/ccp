.DEFAULT_GOAL := help
CARGO ?= cargo
PYTHON ?= python3
LINT_BIN ?= .lint-tools/bin
export PATH := $(abspath $(LINT_BIN)):$(PATH)

.PHONY: installer-tests quality-tests tools build test integration mcp-smoke compat lint lint-fast lint-rust lint-python lint-shell lint-workflows lint-secrets security fmt help

installer-tests: ## Run isolated Docker installer regressions.
	$(PYTHON) tests/docker_install.py

quality-tests: ## Run quality-tool and CLI-accounting regressions.
	$(PYTHON) -m unittest discover -s tests -p 'test_quality.py' -v

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

compat: build ## Check TLS enrollment, client requests, and persistence reopening.
	$(PYTHON) tests/dependency_compat.py --server target/release/server --client target/release/client

lint: lint-fast ## Alias for the complete offline source checks.

lint-fast: fmt lint-rust lint-python lint-shell lint-workflows lint-secrets ## Run all offline source checks; Cargo needs cached dependencies.

fmt: ## Check Rust formatting.
	$(CARGO) fmt --all -- --check

lint-rust: ## Check all Rust targets with warnings treated as errors.
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

lint-python: ## Check Python correctness/security rules and formatting.
	$(PYTHON) -m ruff check mcp tests scripts
	$(PYTHON) -m ruff format --check mcp tests scripts

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

help: ## Show available checks.
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ {printf "%-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
