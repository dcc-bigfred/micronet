# micronet

Ethernet bring-up and DHCP gateway daemon for BigFred OS. One process on
the hub's physical Ethernet: JSON config, Unix-socket IPC, inotify
hot-reload. Chooses **client**, **gateway**, or **static `.252`** from a
DHCPDISCOVER probe and a ping of `gateway.ip`.

## Features

- Three modes: foreign DHCP → `client` (`dhclient`); live `gateway.ip` → `static`; empty LAN → `gateway` + dnsmasq
- DHCPDISCOVER only (no REQUEST); ICMP ping of `gateway.ip` after a temporary `.252`
- dnsmasq only in `gateway` (pool `.50–.200`, sticky MAC→IP lease **7d**, `option:router` / `dns-server`)
- Physical Ethernet only (not `lo`, bridge, virtual, Wi-Fi)
- JSON camelCase under `$DATA_DIR/etc/micronet.json` (no hardcoded `/data/...`); invalid reload keeps the previous config
- Unix socket `$DATA_DIR/run/micronet.sock` (4-byte LE length + JSON)
- `std::thread` (no tokio); musl arm64

## Docs

- [ARCHITECTURE.md](ARCHITECTURE.md) — canonical design
- [CODING-GUIDELINES.md](CODING-GUIDELINES.md) — engineering standard (copy of microinit)
- Event WiFi mount: [docs/networking](docs/networking/README.md) (EN) / [PL](docs/networking/README_pl.md)

## Build

```bash
make build
make test
make release-musl
```
