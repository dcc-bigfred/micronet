# configure-ethernet

One-shot Ethernet bring-up for BigFred OS (Rust). Tries common club static
subnets, then falls back to DHCP. `check` is a cheap liveness probe for microinit.

## Commands

```bash
configure-ethernet          # same as up
configure-ethernet up
configure-ethernet check    # exit 0 if UP+IPv4
```

Config: `/data/etc/configure-ethernet.conf` (`PRIMARY` / `SECONDARY`).

Part of the [micronet](https://github.com/dcc-bigfred/micronet) workspace.
