.DEFAULT_GOAL := build

CARGO ?= cargo

.PHONY: build release test integration check lint fmt docker help

build: ## Compile the workspace in development mode.
	$(CARGO) build --workspace --locked

release: ## Compile optimized server and client binaries.
	$(CARGO) build --workspace --release --locked

test: ## Run all Rust tests.
	$(CARGO) test --workspace --locked
	python3 -m unittest discover -s mcp/tests -v

integration: release ## Run the end-to-end CLI test suite.
	bash tests/run.sh --skip-build

check: ## Type-check without producing binaries.
	$(CARGO) check --workspace --all-targets --locked

lint: ## Run Clippy with the project's CI lint policy.
	$(CARGO) clippy --workspace -- -D warnings -A clippy::too_many_arguments -A clippy::type_complexity -A clippy::large_enum_variant -A clippy::await_holding_lock

fmt: ## Check Rust formatting.
	$(CARGO) fmt --all -- --check

docker: ## Build the server container image.
	docker build --tag ccp:local .

help: ## Show available build targets.
	@awk 'BEGIN {FS = ":.*##"} /^[a-zA-Z_-]+:.*##/ {printf "%-14s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
