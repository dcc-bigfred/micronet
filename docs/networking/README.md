# BigFred event WiFi — mount and configure

**Language:** English | [Polski](./README_pl.md)

Related plans: [topology](../../plans/2026-07-14-topologia-wifi-hala.md), [EAP613 settings](../../plans/2026-07-14-eap613-konfiguracja.md)

Architecture of the daemon: [ARCHITECTURE.md](../../ARCHITECTURE.md).

For a non-technical operator. Goal: low-latency WiFi for throttles (`bigfred2`, 2.4 GHz) and phones (`bigfred5`, 5 GHz).

## What you need

- Raspberry Pi **5** + Ethernet = **BigFred** (server)
- Omada **EAP610/613 × 3** (access points), **wired** to the switch (this is not Omada Mesh)
- **Switch PoE TL-SF1006P** (ports 1–4 PoE+, 5–6 plain) — **required**. APs take PoE from this switch only
- **Omada OC200** — **required** for Fast Roaming (802.11k/v). The controller must stay powered; if it stops, Fast Roaming stops. Same SSID on all APs still allows *basic* client-driven roaming without a controller, but not steered handovers
- Optional: **any DHCP router** on a spare switch port (not as PoE for the Omadas). Then BigFred yields and does **not** serve DHCP
- Ethernet cables, PSUs, 3 stands at **2 m**, laptop/phone for setup, optional UPS

## How BigFred networking works

On boot, the **`micronet` daemon** (first physical Ethernet **with a cable**, else first Ethernet):

1. Brings the interface up (no address).
2. Sends **DHCPDISCOVER** and waits for a **DHCPOFFER** (no REQUEST).
3. **Offer** → mode **`client`**: `dhclient`, no dnsmasq, no `gateway.ip` on the Pi.
4. **No offer** → temporarily `.252` in the configured subnet, then `ping gateway.ip`:
   - ping OK → mode **`static`**: stay on `.252`, default route via `gateway.ip`, no dnsmasq
   - ping fail → mode **`gateway`**: take `gateway.ip` (image seed: **`192.168.0.1/24`**), start **dnsmasq** (pool `.50–.200`, sticky lease **7d**, router/DNS = BigFred). **No default route.** Optional `dns.records` in `$DATA_DIR/etc/micronet.json` become unicast names (e.g. `bigfred.lan`) pointing at `gateway.ip`. In `client` / `static` those names are not served — use mDNS `bigfred.local`.

If a router is plugged into the switch or boots **after** BigFred already became gateway, micronet notices the foreign DHCP (periodic probe) and **yields**: stops dnsmasq and runs `dhclient`. Power the router first when you want it to own DHCP.

There is no Omada detection and no per-MAC `dhcp-host=` reservations. Stickiness is the dnsmasq leasefile + `7d`.

Typical mapping:

| Backhaul | Foreign DHCP / live `gateway.ip` | BigFred mode | Who leases the laptop |
|---|---|---|---|
| TL-SF1006P only | none | `gateway` | BigFred dnsmasq |
| TL-SF1006P + router on a spare port | yes (router) | `client` or `static` | the router |

You do not edit dnsmasq by hand for the event setup. JSON: `$DATA_DIR/etc/micronet.json` (hot-reload). Optional `"dns": { "enabled": true, "records": [ { "name": "bigfred.lan" } ] }` is how traditional names are added; omit `addr` to use `gateway.ip`, or set `addr` to an IPv4.

---

## Kit A — Switch TL-SF1006P (BigFred = DHCP)

### 1. Cabling (power off)

| Switch port | Device | Notes |
|---|---|---|
| 1 | BigFred | Priority Mode |
| 2 | AP1 | PoE |
| 3 | AP2 | PoE |
| 4 | AP3 | PoE |
| 5 | OC200 | Plain port; OC200 has its own PSU (**required** for Fast Roaming) |
| 6 | free | Laptop, or optional DHCP router |

- [ ] BigFred → port 1
- [ ] AP1 → 2, AP2 → 3, AP3 → 4
- [ ] OC200 → 5
- [ ] Plug in switch, BigFred, and OC200 PSUs

### 2. Switch rear switches

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (otherwise ports drop to 10 Mb/s)

### 3. Power-on order

Empty hall: ping to `192.168.0.1` fails → BigFred becomes gateway immediately. APs get a lease after they boot.

- [ ] 1. Switch
- [ ] 2. BigFred — wait until UI answers at `http://192.168.0.1` (~1–2 min)
- [ ] 3. OC200 — wait ~3 min
- [ ] 4. AP1/2/3 via PoE — wait ~3 min

### 4. Join with a laptop

- [ ] Ethernet to switch port 6 — laptop gets an address **from BigFred**, e.g. `192.168.0.51`

---

## Optional — DHCP router on the switch

Do **not** power Omada APs from a MikroTik (or other) PoE router: those PoE pins are not compatible with EAP610/613. Keep APs on the **TL-SF1006P**. Plug the router into a spare switch port (e.g. 6) if you want *it* to serve DHCP.

### Power-on order

- [ ] 1. Switch
- [ ] 2. Router (wait until its DHCP is up)
- [ ] 3. BigFred — joins as **`client`** (or **`static` `.252`** if the router has no DHCP but answers ping on `gateway.ip`)
- [ ] 4. OC200, then APs via switch PoE

### Laptop

- [ ] Plug into the switch — lease comes **from the router**, not from BigFred.
- [ ] Confirm `micronet status` is `client` or `static`, **not** `gateway`.

---

## 5. Configure WiFi — use the OC200

### Path A — OC200 (required for Fast Roaming)

[TP-Link Fast Roaming](https://support.omadanetworks.com/en/document/12972/) (802.11k/v) is enabled from the controller and **needs the controller running**. Omada Mesh is a separate wireless-backhaul feature; this kit uses **wired** APs, so Mesh stays OFF.

- [ ] Find OC200 IP (TP-Link **Omada Discovery**)
- [ ] Open `https://<oc200-ip>`, accept the cert warning
- [ ] Login `admin` / `admin`, set a new admin password
- [ ] Wizard: region/timezone; skip creating SSIDs here
- [ ] **Devices** → Adopt all three APs → wait until **Connected**
- [ ] Create WLAN group + SSIDs (step 6) and radio tweaks (step 7) **once** in the controller
- [ ] Enable **Fast Roaming** (802.11k/v) in the controller; leave Mesh OFF

### Path B — standalone (no steered roaming)

Only if the OC200 is missing. Do steps 6–7 **on each AP**. Same SSID/password still lets phones roam on their own (slow scan). There is **no** 802.11k/v Fast Roaming without a running Omada Controller. Default first access: sticker SSID or `https://tplinkeap.net` / `https://192.168.0.254`.

## 6. SSIDs: `bigfred2` and `bigfred5`

Same password for both.

### `bigfred2` (2.4 GHz only — throttles)

- [ ] SSID `bigfred2`, broadcast ON, band **2.4 GHz only**
- [ ] WPA2-PSK, AES, your password
- [ ] VLAN 0, Portal OFF, SSID/Client Isolation **OFF**, Save

### `bigfred5` (5 GHz only — phones)

- [ ] SSID `bigfred5`, broadcast ON, band **5 GHz only**
- [ ] Same security and password, VLAN 0, Portal OFF, Isolation OFF, Save

## 7. Low-latency radio tweaks

### 7.1 Channels (per AP)

2.4 GHz, **20 MHz**, Manual:

| AP | Channel | Width | Tx |
|---|---|---|---|
| AP1 | 1 | 20 MHz | Medium |
| AP2 | 6 | 20 MHz | Medium |
| AP3 | 11 | 20 MHz | Medium |

5 GHz, **40 MHz**, non-DFS:

| AP | Channel | Width | Tx |
|---|---|---|---|
| AP1 | 36 | 40 MHz | Medium |
| AP2 | 149 | 40 MHz | Medium |
| AP3 | 44 (or 157) | 40 MHz | Medium |

- [ ] Channel selection = **Manual** (not Auto)
- [ ] Do **not** use DFS channels 52–144

### 7.2 Advanced

- [ ] Airtime Fairness ON, OFDMA ON, MU-MIMO ON
- [ ] Beacon 100, DTIM 1, min data rate 2.4 GHz = 6 Mbps (if available)
- [ ] Mesh OFF, Band Steering OFF

### 7.3 WMM / multicast / roaming

- [ ] WMM Enable on both SSIDs
- [ ] Multicast filter OFF (mDNS `224.0.0.251` must pass); IGMP snooping + multicast-to-unicast ON if available
- [ ] Client Isolation OFF
- [ ] Load balance 2.4 GHz: max ~18 clients; Fast Roaming **802.11k/v** ON in the controller (Omada EAP Fast Roaming is k/v, not 802.11r)

## 8. Validation

- [ ] Phone sees `bigfred2` and `bigfred5`
- [ ] On `bigfred5`, open `http://192.168.0.1` (BigFred UI) when using the event seed / switch kit
- [ ] Throttle on `bigfred2`
- [ ] Ping to the hub &lt; 25 ms
- [ ] RSSI at operator seats &gt; −65 dBm

## 9. Event-day checklist

- [ ] Three APs at 2 m around operators (not behind the layout)
- [ ] Switch: BigFred on port 1 (Priority), OC200 on port 5, APs on PoE 2–4
- [ ] No extra router: `micronet status` → `gateway`, laptop leased by BigFred
- [ ] Optional router on a spare switch port: `micronet status` → `client`/`static`; laptop leased by the router
- [ ] Spectrum check — adjust 1/6/11 if needed
- [ ] 3–5 test throttles OK
- [ ] Ask audience to disable personal hotspots
- [ ] Spare AP + PoE injector ready

## Technical notes

- Daemon: [`crates/micronet`](../../crates/micronet/) → `/usr/sbin/micronet` on BigFred OS
- Shared CI: reusable workflows in [`dcc-bigfred/common`](https://github.com/dcc-bigfred/common) (`@v2`); binary fetch via `go run github.com/dcc-bigfred/common/cmd/fetch@latest`
- Detailed EAP613 menu paths: [plans/2026-07-14-eap613-konfiguracja.md](../../plans/2026-07-14-eap613-konfiguracja.md)
