.PHONY: build test bindings clean

# Build all contracts for the Soroban wasm32v1-none target.
build:
	stellar contract build
	@echo "Compiled WASM artifacts:"
	@ls -la target/wasm32v1-none/release/*.wasm 2>/dev/null \
		|| ls -la target/wasm32-unknown-unknown/release/*.wasm

# Run the Rust unit/integration tests.
test:
	cargo test --workspace

# Regenerate TypeScript bindings into the veda-sdk package.
bindings:
	stellar contract bindings typescript \
		--wasm target/wasm32v1-none/release/core_registry.wasm \
		--output-dir ../veda-sdk/src/contracts \
		--network testnet \
		--overwrite
	stellar contract bindings typescript \
		--wasm target/wasm32v1-none/release/escrow_vault.wasm \
		--output-dir ../veda-sdk/src/contracts \
		--network testnet \
		--overwrite

clean:
	cargo clean
	rm -rf target
