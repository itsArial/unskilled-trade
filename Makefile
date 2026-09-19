.PHONY: setup build run test check browser-test
setup:
	python3 scripts/setup.py
	cd frontend && npm ci --include=dev
build:
	cd frontend && npm run build
	cargo build --release --manifest-path backend/Cargo.toml
run:
	cd backend && cargo run --release
test:
	cargo test --manifest-path backend/Cargo.toml
check:
	cargo fmt --check --manifest-path backend/Cargo.toml
	cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
	cd frontend && npm run build
browser-test:
	cd frontend && npx playwright test
