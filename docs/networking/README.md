# BigFred event WiFi — mount and configure

**Language:** English | [Polski](./README_pl.md)

Related plans: [topology](../../plans/2026-07-14-topologia-wifi-hala.md), [EAP613 settings](../../plans/2026-07-14-eap613-konfiguracja.md)

Architecture of the daemon: [ARCHITECTURE.md](../../ARCHITECTURE.md).

For a non-technical operator. Goal: low-latency WiFi for throttles (`bigfred2`, 2.4 GHz) and phones (`bigfred5`, 5 GHz).

## What you need

- Raspberry Pi 3 + Ethernet = **BigFred** (server)
- Omada **EAP610/613 × 3** (access points)
- **One** of two L2 backhauls (operator choice; the daemon does not detect the vendor):
  - **Switch PoE TL-SF1006P** (ports 1–4 PoE+, 5–6 plain) — BigFred **serves DHCP**
  - **MikroTik hEX PoE lite RB750UPr2** (5× FE, **4 PoE** ports) — router **serves DHCP**; BigFred does **not**
- **Omada OC200** is optional (central controller). Without it, configure each AP in **standalone** mode (same SSIDs/settings on every AP; only channels differ).
- Ethernet cables, PSUs, 3 stands at **2 m**, laptop/phone for setup, optional UPS

## How BigFred networking works

On boot, the **`micronet` daemon** (`eth0` / first physical Ethernet):

1. Brings the interface up (no address).
2. Sends **DHCPDISCOVER** and waits for a **DHCPOFFER** (no REQUEST).
3. **Offer** → mode **`client`**: `dhclient`, no dnsmasq, no `gateway.ip` on the Pi.
4. **No offer** → temporarily `.252` in the configured subnet, then `ping gateway.ip`:
   - ping OK → mode **`static`**: stay on `.252`, default route via `gateway.ip`, no dnsmasq
   - ping fail → mode **`gateway`**: take `gateway.ip` (image seed: **`10.0.10.1/24`**), start **dnsmasq** (pool `.50–.200`, sticky lease **7d**, router/DNS = BigFred). **No default route.**

There is no Omada detection and no per-MAC `dhcp-host=` reservations. Stickiness is the dnsmasq leasefile + `7d`.

Typical mapping:

| Backhaul | Foreign DHCP / live `gateway.ip` | BigFred mode | Who leases the laptop |
|---|---|---|---|
| TL-SF1006P (dumb PoE switch) | none | `gateway` | BigFred dnsmasq |
| hEX PoE lite RB750UPr2 | yes (router) | `client` or `static` | MikroTik |

You do not edit dnsmasq by hand for the event setup. JSON: `$DATA_DIR/etc/micronet.json` (hot-reload).

---

## Kit A — Switch TL-SF1006P (BigFred = DHCP)

### 1. Cabling (power off)

| Switch port | Device | Notes |
|---|---|---|
| 1 | BigFred | Priority Mode |
| 2 | AP1 | PoE |
| 3 | AP2 | PoE |
| 4 | AP3 | PoE |
| 5 | OC200 (optional) | Plain port; OC200 has its own PSU |
| 6 | free | Laptop for setup |

- [ ] BigFred → port 1
- [ ] AP1 → 2, AP2 → 3, AP3 → 4
- [ ] OC200 → 5 (if used)
- [ ] Plug in switch, BigFred, and OC200 PSUs

### 2. Switch rear switches

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (otherwise ports drop to 10 Mb/s)

### 3. Power-on order

Empty hall: ping to `10.0.10.1` fails → BigFred becomes gateway immediately. APs get a lease after they boot.

- [ ] 1. Switch
- [ ] 2. BigFred — wait until UI answers at `http://10.0.10.1` (~1–2 min)
- [ ] 3. OC200 (if used) — wait ~3 min
- [ ] 4. AP1/2/3 via PoE — wait ~3 min

### 4. Join with a laptop

- [ ] Ethernet to switch port 6 — laptop gets an address **from BigFred**, e.g. `10.0.10.51`

---

## Kit B — MikroTik hEX PoE lite RB750UPr2 (router = DHCP)

BigFred **must not** serve DHCP (router already does). ether1 has **no PoE**.

| Port | Device | Notes |
|---|---|---|
| ether1 | BigFred | no PoE |
| ether2 | AP1 | PoE |
| ether3 | AP2 | PoE |
| ether4 | AP3 | PoE |
| ether5 | spare AP / laptop | PoE |

### Power-on order

- [ ] 1. MikroTik (wait until its DHCP is up)
- [ ] 2. BigFred — joins as **`client`** (or **`static` `.252`** if the router has no DHCP but answers ping on `gateway.ip`)
- [ ] 3. APs via PoE on ether2–5

### Laptop

- [ ] Plug into a spare router port — lease comes **from the MikroTik**, not from BigFred.
- [ ] Confirm `micronet status` is `client` or `static`, **not** `gateway`.

---

## 5. Configure WiFi — choose one path

### Path A — with OC200 (controller)

- [ ] Find OC200 IP (TP-Link **Omada Discovery**)
- [ ] Open `https://<oc200-ip>`, accept the cert warning
- [ ] Login `admin` / `admin`, set a new admin password
- [ ] Wizard: region/timezone; skip creating SSIDs here
- [ ] **Devices** → Adopt all three APs → wait until **Connected**
- [ ] Create WLAN group + SSIDs (step 6) and radio tweaks (step 7) **once** in the controller

### Path B — standalone (no OC200)

Do steps 6–7 **on each AP** (AP1, then AP2, then AP3). Default first access: join the sticker SSID or open `https://tplinkeap.net` / `https://192.168.0.254`, then set a management password and preferably a static/management IP once on the event subnet. Channels differ per AP (step 7.1); SSIDs and passwords are identical.

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
- [ ] Load balance 2.4 GHz: max ~18 clients; 802.11k/v/r ON

## 8. Validation

- [ ] Phone sees `bigfred2` and `bigfred5`
- [ ] On `bigfred5`, open `http://10.0.10.1` (BigFred UI) when using the event seed / switch kit
- [ ] Throttle on `bigfred2`
- [ ] Ping to the hub &lt; 25 ms
- [ ] RSSI at operator seats &gt; −65 dBm

## 9. Event-day checklist

- [ ] Three APs at 2 m around operators (not behind the layout)
- [ ] Switch kit: BigFred on port 1 (Priority), `micronet status` → `gateway`, laptop leased by BigFred
- [ ] MikroTik kit: ether1 = BigFred, ether2–5 = APs; `micronet status` → `client`/`static`; laptop leased by the router
- [ ] Spectrum check — adjust 1/6/11 if needed
- [ ] 3–5 test throttles OK
- [ ] Ask audience to disable personal hotspots
- [ ] Spare AP + PoE injector ready

## Technical notes

- Daemon: [`crates/micronet`](../../crates/micronet/) → `/usr/sbin/micronet` on BigFred OS
- Shared CI: reusable workflows in [`dcc-bigfred/common`](https://github.com/dcc-bigfred/common) (`@v2`); binary fetch via `go run github.com/dcc-bigfred/common/cmd/fetch@latest`
- Detailed EAP613 menu paths: [plans/2026-07-14-eap613-konfiguracja.md](../../plans/2026-07-14-eap613-konfiguracja.md)
