# configure-dhcp

Rust micro-CLI for BigFred OS. Starts **dnsmasq** only when a pluggable event
WiFi **stack** detects gear on the LAN (today: **Omada** AP / OC200). Otherwise
exits 0 and leaves club networking alone.

## Build

```bash
cargo build --release -p configure-dhcp
# from this directory:
cargo build --release
```

Binary: `target/release/configure-dhcp` → install as `/usr/sbin/configure-dhcp`.

## Commands

```bash
configure-dhcp up       # detect → maybe DHCP + reservations (default)
configure-dhcp check    # report stacks / gate / dnsmasq
```

## Behaviour

1. Probe registered stacks (`OmadaStack`: TP-Link OUI in ARP + UDP discovery + hostname match).
2. Gate **ON** if any device found **or** sticky state `/data/etc/configure-dhcp.state` lists a stack.
3. Gate **OFF** → skip (no `10.0.10.1`, no dnsmasq).
4. Gate **ON** → set `10.0.10.1/24`, write `/data/etc/dnsmasq.conf` (pool `.50–.200`, lease **7d**), start dnsmasq, promote matched MAC→IP into `/data/etc/dnsmasq.reservations.conf`.

## Extending stacks

Implement `configure_dhcp::stack::Stack` and `registry.register(Box::new(...))` in `main.rs`.

## Dependencies

- `dnsmasq` on the image (`/usr/sbin/dnsmasq`) — required only when the gate wants DHCP.
- microinit service `configure-dhcp` after `network` (see bigfred-os overlays).

## Tests

```bash
cargo test
```
