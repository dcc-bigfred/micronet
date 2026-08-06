# BigFred event WiFi — mount and configure

**Language:** English | [Polski](./README_pl.md)

Related plans: [topology](./plans/2026-07-14-topologia-wifi-hala.md), [EAP613 settings](./plans/2026-07-14-eap613-konfiguracja.md)

For a non-technical operator. Goal: low-latency WiFi for throttles (`bigfred2`, 2.4 GHz) and phones (`bigfred5`, 5 GHz).

## What you need

- Raspberry Pi 3 + Ethernet = **BigFred** (server)
- Omada **EAP610/613 × 3** (access points)
- Switch PoE **TL-SF1006P** (ports 1–4 PoE+, 5–6 plain)
- **Omada OC200** is optional (central controller). Without it, configure each AP in **standalone** mode (same SSIDs/settings on every AP; only channels differ).
- 4–5 Ethernet cables, PSUs (Pi3, switch; OC200 if used), 3 stands at **2 m**, laptop/phone for setup, optional UPS

## How BigFred networking works

On boot, BigFred OS:

1. Brings Ethernet up (`configure-ethernet`).
2. Runs **`configure-dhcp`**, which probes the LAN for an event WiFi stack (today: **Omada** AP or OC200).
3. **Only if Omada gear is detected** does it set BigFred to `10.0.10.1/24` and start **dnsmasq** (pool `10.0.10.50–10.0.10.200`, **7-day** lease, gateway/DNS = BigFred). Detected Omada MACs get sticky DHCP reservations.
4. On a club LAN **without** Omada, DHCP is **not** started (no conflict with the club DHCP server).

You do not edit dnsmasq by hand for the event setup.

---

## 1. Cabling (power off)

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

## 2. Switch rear switches

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (otherwise ports drop to 10 Mb/s)

## 3. Power-on order

BigFred (DHCP) first:

- [ ] 1. Switch
- [ ] 2. BigFred — wait ~2 min (`configure-dhcp` detects Omada and starts DHCP)
- [ ] 3. OC200 (if used) — wait ~3 min
- [ ] 4. AP1/2/3 via PoE — wait ~3 min

## 4. Join the network with a laptop

- [ ] Ethernet to switch port 6 (laptop gets an address from BigFred, e.g. `10.0.10.51`)

## 5. Configure WiFi — choose one path

### Path A — with OC200 (controller)

- [ ] Find OC200 IP (TP-Link **Omada Discovery**, or on BigFred: `configure-dhcp check`)
- [ ] Open `https://<oc200-ip>`, accept the cert warning
- [ ] Login `admin` / `admin`, set a new admin password
- [ ] Wizard: region/timezone; skip creating SSIDs here
- [ ] **Devices** → Adopt all three APs → wait until **Connected**
- [ ] Create WLAN group + SSIDs (step 6) and radio tweaks (step 7) **once** in the controller

### Path B — standalone (no OC200)

Do steps 6–7 **on each AP** (AP1, then AP2, then AP3). Default first access: join the sticker SSID or open `https://tplinkeap.net` / `https://192.168.0.254`, then set a management password and preferably a static/management IP once on the BigFred subnet. Channels differ per AP (step 7.1); SSIDs and passwords are identical.

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
- [ ] On `bigfred5`, open `http://10.0.10.1` (BigFred UI)
- [ ] Throttle on `bigfred2`
- [ ] Ping to `10.0.10.1` &lt; 25 ms
- [ ] RSSI at operator seats &gt; −65 dBm

## 9. Event-day checklist

- [ ] Three APs at 2 m around operators (not behind the layout)
- [ ] BigFred on port 1 (Priority), DHCP running
- [ ] Spectrum check — adjust 1/6/11 if needed
- [ ] 3–5 test throttles OK
- [ ] Ask audience to disable personal hotspots
- [ ] Spare AP + PoE injector ready

## Technical notes

- Tools (Rust workspace): [`crates/configure-dhcp`](./crates/configure-dhcp/), [`crates/configure-ethernet`](./crates/configure-ethernet/) → `/usr/sbin/` on BigFred OS (GitHub Actions artifacts / Releases)
- Shared CI scripts: `make ci-scripts-update` clones `dcc-bigfred/.github` @ `v2` into `.ci-github/`
- Detailed EAP613 menu paths: [plans/2026-07-14-eap613-konfiguracja.md](./plans/2026-07-14-eap613-konfiguracja.md).
