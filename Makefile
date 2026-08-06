# micronet — build / cross-compile

TARGET_MUSL ?= aarch64-unknown-linux-musl
CARGO ?= cargo
RUSTUP_TOOLCHAIN ?= stable
export RUSTUP_TOOLCHAIN

.PHONY: all build release release-musl check test test-release-assertions \
	clean fmt clippy

all: build

build:
	$(CARGO) build --workspace

release:
	$(CARGO) build --workspace --release

release-musl:
	RUSTFLAGS='-C target-feature=+crt-static' \
		$(CARGO) build --workspace --release --target $(TARGET_MUSL)
	@mkdir -p dist
	cp -f target/$(TARGET_MUSL)/release/configure-dhcp dist/configure-dhcp-linux-arm64
	cp -f target/$(TARGET_MUSL)/release/configure-ethernet dist/configure-ethernet-linux-arm64
	@chmod 755 dist/configure-dhcp-linux-arm64 dist/configure-ethernet-linux-arm64
	@echo "wrote dist/configure-*-linux-arm64"

check:
	$(CARGO) check --workspace

test:
	$(CARGO) test --workspace

test-release-assertions:
	$(CARGO) test --workspace --profile release-assertions

fmt:
	$(CARGO) fmt --all

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

clean:
	$(CARGO) clean
	rm -rf dist
