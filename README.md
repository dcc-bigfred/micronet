# micronet

Ethernet bring-up and DHCP gateway daemon for BigFred OS. One process on
the hub's physical Ethernet: JSON config, Unix-socket IPC, inotify
hot-reload. Chooses **client**, **gateway**, or **static `.252`** from a
DHCPDISCOVER probe and a ping of `gateway.ip`.

## Features

- Three modes: foreign DHCP → `client` (`dhclient`); live `gateway.ip` → `static`; empty LAN → `gateway` + dnsmasq
- Choose iface + mode **once**; afterwards only watch carrier (`linkRetrySecs`, default 15). No periodic DHCP probe
- DHCPDISCOVER only at select time (no REQUEST); ICMP ping of `gateway.ip` after a temporary `.252` if the iface has no address yet
- dnsmasq only in `gateway` (pool `.50–.200`, sticky MAC→IP lease **7d**, `option:router` / `dns-server`)
- Optional `dns` section in JSON: static unicast A records (`host-record=`) served only in `gateway`
- Physical Ethernet only (not `lo`, bridge, virtual, Wi-Fi)
- Owns **all** cable Ethernet: picks one with carrier (`interface` null / omitted / `"auto"`); remaining ifaces are flushed and admin-down. Unplug → wait `linkRetrySecs`, then one re-select. An explicit name pins the device
- JSON camelCase under `$DATA_DIR/etc/micronet.json` (no hardcoded `/data/...`); invalid reload keeps the previous config
- Unix socket `$DATA_DIR/run/micronet.sock` (4-byte LE length + JSON); `status.healthy` is the liveness verdict
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
