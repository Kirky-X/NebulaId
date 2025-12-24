.PHONY: dev-up dev-down dev-logs test build clean lint clippy doc

# Development commands
dev-up:
	docker-compose up -d

dev-down:
	docker-compose down

dev-logs:
	docker-compose logs -f

# Build and test
build:
	cargo build --release

build-dev:
	cargo build

test:
	cargo test --all -- --test-threads=4

test-coverage:
	cargo tarpaulin --out Html

# Code quality
lint:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all -- -D warnings -A clippy::derivable-clones -A clippy::redundant-pub-crate

doc:
	cargo doc --no-deps --open

# Clean
clean:
	cargo clean
	docker-compose down -v

# Database
db-migrate:
	cargo run --bin migrate -- crates/server

# One-liners
shell:
	docker-compose exec app bash

db-shell:
	docker-compose exec postgres psql -U idgen -d idgen

redis-cli:
	docker-compose exec redis redis-cli

# Help
help:
	@echo "Available commands:"
	@echo "  dev-up      - Start development environment"
	@echo "  dev-down    - Stop development environment"
	@echo "  dev-logs    - Follow logs"
	@echo "  build       - Build release binary"
	@echo "  build-dev   - Build development binary"
	@echo "  test        - Run all tests"
	@echo "  lint        - Check code formatting"
	@echo "  clippy      - Run clippy lints"
	@echo "  doc         - Generate documentation"
	@echo "  clean       - Clean build artifacts"
	@echo "  db-migrate  - Run database migrations"
	@echo "  shell       - Enter app container"
	@echo "  db-shell    - Connect to PostgreSQL"
	@echo "  redis-cli   - Connect to Redis"
	@echo "  help        - Show this help"
