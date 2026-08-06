# micronet — build / cross-compile

TARGET_MUSL ?= aarch64-unknown-linux-musl
CARGO ?= cargo
RUSTUP_TOOLCHAIN ?= stable
export RUSTUP_TOOLCHAIN

CI_SCRIPTS_REPO ?= https://github.com/dcc-bigfred/.github.git
CI_SCRIPTS_REF  ?= v2
CI_SCRIPTS_DIR  ?= .ci-github

.PHONY: all build release release-musl check test test-release-assertions \
	clean fmt clippy ci-scripts-update

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

$(CI_SCRIPTS_DIR)/.ok:
	@echo "Cloning $(CI_SCRIPTS_REPO) @ $(CI_SCRIPTS_REF) → $(CI_SCRIPTS_DIR)"
	@rm -rf "$(CI_SCRIPTS_DIR)"
	@git clone --depth 1 --branch "$(CI_SCRIPTS_REF)" "$(CI_SCRIPTS_REPO)" "$(CI_SCRIPTS_DIR)" \
		|| { echo "error: failed to clone $(CI_SCRIPTS_REPO) @ $(CI_SCRIPTS_REF)"; exit 1; }
	@touch "$@"

ci-scripts-update:
	rm -rf "$(CI_SCRIPTS_DIR)"
	$(MAKE) "$(CI_SCRIPTS_DIR)/.ok"
