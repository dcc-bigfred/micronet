# BigFred event WiFi — montaż i konfiguracja

**Język:** [English](./README.md) | Polski

Powiązane plany: [topologia](../../plans/2026-07-14-topologia-wifi-hala.md), [ustawienia EAP613](../../plans/2026-07-14-eap613-konfiguracja.md)

Architektura daemona: [ARCHITECTURE.md](../../ARCHITECTURE.md).

Dla mało technicznego operatora. Cel: WiFi o niskim opóźnieniu dla pilotów (`bigfred2`, 2.4 GHz) i telefonów (`bigfred5`, 5 GHz).

## Co potrzebujesz

- Raspberry Pi 3 + Ethernet = **BigFred** (serwer)
- Omada **EAP610/613 × 3** (access pointy)
- **Jeden** z dwóch backhaulów L2 (wybór operatora; daemon nie rozpoznaje modelu):
  - **Switch PoE TL-SF1006P** (porty 1–4 PoE+, 5–6 zwykłe) — BigFred **serwuje DHCP**
  - **MikroTik hEX PoE lite RB750UPr2** (5× FE, **4 porty PoE**) — DHCP na routerze; BigFred **nie** serwuje DHCP
- **Omada OC200** jest opcjonalny (centralny kontroler). Bez niego konfigurujesz każdy AP w trybie **standalone** (te same SSID/ustawienia; różnią się tylko kanały).
- Kable Ethernet, zasilacze, 3 statywy na **2 m**, laptop/telefon do konfiguracji, opcjonalnie UPS

## Jak działa sieć na BigFredzie

Po starcie daemon **`micronet`** (pierwszy fizyczny Ethernet):

1. Podnosi interfejs (bez adresu).
2. Wysyła **DHCPDISCOVER** i czeka na **DHCPOFFER** (bez REQUEST).
3. **Jest oferta** → tryb **`client`**: `dhclient`, bez dnsmasq, bez `gateway.ip` na Pi.
4. **Brak oferty** → tymczasowo `.252` w skonfigurowanej podsieci, potem `ping gateway.ip`:
   - ping OK → tryb **`static`**: zostań na `.252`, default via `gateway.ip`, bez dnsmasq
   - ping fail → tryb **`gateway`**: weź `gateway.ip` (seed obrazu: **`192.168.0.1/24`**), start **dnsmasq** (pula `.50–.200`, sticky **7d**, router/DNS = BigFred). **Bez default route.** Opcjonalne `dns.records` w `$DATA_DIR/etc/micronet.json` to nazwy unicast (np. `bigfred.lan`) na `gateway.ip`. W `client` / `static` tych nazw nie ma — zostaje mDNS `bigfred.local`.

Jeśli router pojawi się **później** (po tym, jak BigFred już został gatewayem), micronet wykryje obcy DHCP (okresowa sonda) i **ustąpi**: wyłączy dnsmasq i uruchomi `dhclient`. Kolejność włączania zestawu B to nadal „najpierw router”, ale późniejsze podłączenie jest obsłużone.

Nie ma wykrywania Omady ani rezerwacji `dhcp-host=` per MAC. Stickiness to leasefile dnsmasq + `7d`.

Typowe mapowanie:

| Backhaul | Obcy DHCP / żywy `gateway.ip` | Tryb BigFred | Kto daje lease laptopowi |
|---|---|---|---|
| TL-SF1006P (głupi switch PoE) | brak | `gateway` | dnsmasq na BigFredzie |
| hEX PoE lite RB750UPr2 | tak (router) | `client` albo `static` | MikroTik |

Nie edytujesz dnsmasq ręcznie pod setup eventu. JSON: `$DATA_DIR/etc/micronet.json` (hot-reload). Opcjonalne `"dns": { "enabled": true, "records": [ { "name": "bigfred.lan" } ] }` dodaje tradycyjne nazwy; bez `ip` używane jest `gateway.ip`.

---

## Zestaw A — Switch TL-SF1006P (BigFred = DHCP)

### 1. Okablowanie (przed włączeniem prądu)

| Port switcha | Urządzenie | Uwagi |
|---|---|---|
| 1 | BigFred | Priority Mode |
| 2 | AP1 | PoE |
| 3 | AP2 | PoE |
| 4 | AP3 | PoE |
| 5 | OC200 (opcjonalnie) | Zwykły port; OC200 ma własny zasilacz |
| 6 | wolny | Laptop do konfiguracji |

- [ ] BigFred → port 1
- [ ] AP1 → 2, AP2 → 3, AP3 → 4
- [ ] OC200 → 5 (jeśli używasz)
- [ ] Zasilacze: switch, BigFred, OC200

### 2. Przełączniki z tyłu switcha

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (inaczej porty spadną do 10 Mb/s)

### 3. Kolejność włączania

Pusta hala: ping na `192.168.0.1` pada → BigFred od razu jest gatewayem. AP-y dostaną lease po starcie.

- [ ] 1. Switch
- [ ] 2. BigFred — poczekaj aż UI odpowie na `http://192.168.0.1` (~1–2 min)
- [ ] 3. OC200 (jeśli jest) — poczekaj ~3 min
- [ ] 4. AP1/2/3 przez PoE — poczekaj ~3 min

### 4. Laptop w sieci

- [ ] Ethernet do portu 6 — laptop dostanie adres **z BigFreda**, np. `192.168.0.51`

---

## Zestaw B — MikroTik hEX PoE lite RB750UPr2 (router = DHCP)

BigFred **nie** może serwować DHCP (router już to robi). ether1 **bez PoE**.

| Port | Urządzenie | Uwagi |
|---|---|---|
| ether1 | BigFred | bez PoE |
| ether2 | AP1 | PoE |
| ether3 | AP2 | PoE |
| ether4 | AP3 | PoE |
| ether5 | zapasowy AP / laptop | PoE |

### Kolejność włączania

- [ ] 1. MikroTik (poczekaj aż jego DHCP wstanie)
- [ ] 2. BigFred — dołącza jako **`client`** (albo **`static` `.252`**, gdy router nie ma DHCP, ale odpowiada na ping `gateway.ip`)
- [ ] 3. AP-y przez PoE na ether2–5

### Laptop

- [ ] Włóż do wolnego portu routera — lease daje **MikroTik**, nie BigFred.
- [ ] `micronet status` ma być `client` albo `static`, **nie** `gateway`.

---

## 5. Konfiguracja WiFi — wybierz ścieżkę

### Ścieżka A — z OC200 (kontroler)

- [ ] Znajdź IP OC200 (**Omada Discovery** od TP-Link)
- [ ] Otwórz `https://<ip-oc200>`, zignoruj ostrzeżenie certyfikatu
- [ ] Login `admin` / `admin`, ustaw nowe hasło admina
- [ ] Wizard: region/strefa; pomiń tworzenie SSID
- [ ] **Devices** → Adopt trzech AP → status **Connected**
- [ ] Utwórz WLAN + SSID (krok 6) i tweaki radia (krok 7) **raz** w kontrolerze

### Ścieżka B — standalone (bez OC200)

Kroki 6–7 zrób **na każdym AP** (AP1, potem AP2, potem AP3). Pierwszy dostęp: SSID z naklejki lub `https://tplinkeap.net` / `https://192.168.0.254`, ustaw hasło zarządzania. Kanały różnią się per AP (krok 7.1); SSID i hasła są identyczne.

## 6. SSID: `bigfred2` i `bigfred5`

To samo hasło dla obu.

### `bigfred2` (tylko 2.4 GHz — piloty)

- [ ] SSID `bigfred2`, broadcast ON, pasmo **tylko 2.4 GHz**
- [ ] WPA2-PSK, AES, Twoje hasło
- [ ] VLAN 0, Portal OFF, Isolation **OFF**, Save

### `bigfred5` (tylko 5 GHz — telefony)

- [ ] SSID `bigfred5`, broadcast ON, pasmo **tylko 5 GHz**
- [ ] To samo zabezpieczenie i hasło, VLAN 0, Portal OFF, Isolation OFF, Save

## 7. Tweaki radia pod niskie opóźnienie

### 7.1 Kanały (per AP)

2.4 GHz, **20 MHz**, Manual:

| AP | Kanał | Szerokość | Tx |
|---|---|---|---|
| AP1 | 1 | 20 MHz | Medium |
| AP2 | 6 | 20 MHz | Medium |
| AP3 | 11 | 20 MHz | Medium |

5 GHz, **40 MHz**, non-DFS:

| AP | Kanał | Szerokość | Tx |
|---|---|---|---|
| AP1 | 36 | 40 MHz | Medium |
| AP2 | 149 | 40 MHz | Medium |
| AP3 | 44 (lub 157) | 40 MHz | Medium |

- [ ] Channel selection = **Manual** (nie Auto)
- [ ] **Nie** używaj kanałów DFS 52–144

### 7.2 Advanced

- [ ] Airtime Fairness ON, OFDMA ON, MU-MIMO ON
- [ ] Beacon 100, DTIM 1, min data rate 2.4 GHz = 6 Mbps (jeśli jest)
- [ ] Mesh OFF, Band Steering OFF

### 7.3 WMM / multicast / roaming

- [ ] WMM Enable na obu SSID
- [ ] Multicast filter OFF (mDNS `224.0.0.251` musi przechodzić); IGMP snooping + multicast-to-unicast ON jeśli dostępne
- [ ] Client Isolation OFF
- [ ] Load balance 2.4 GHz: max ~18 klientów; 802.11k/v/r ON

## 8. Walidacja

- [ ] Telefon widzi `bigfred2` i `bigfred5`
- [ ] Na `bigfred5` otwórz `http://192.168.0.1` (BigFred) przy seedzie eventu / zestawie switch
- [ ] Pilot na `bigfred2`
- [ ] Ping do huba &lt; 25 ms
- [ ] RSSI na stanowiskach &gt; −65 dBm

## 9. Checklist dnia eventu

- [ ] 3 AP na 2 m wokół operatorów (nie za makietą)
- [ ] Zestaw switch: BigFred na porcie 1 (Priority), `micronet status` → `gateway`, laptop z lease’em BigFreda
- [ ] Zestaw MikroTik: ether1 = BigFred, ether2–5 = AP; `micronet status` → `client`/`static`; laptop z lease’em routera
- [ ] Skan widma — ew. korekta 1/6/11
- [ ] 3–5 pilotów testowych OK
- [ ] Prośba do publiczności: wyłączyć hotspoty
- [ ] Zapasowy AP + injector PoE

## Uwagi techniczne

- Daemon: [`crates/micronet`](../../crates/micronet/) → `/usr/sbin/micronet` na BigFred OS
- Wspólne CI: reusable workflows w [`dcc-bigfred/common`](https://github.com/dcc-bigfred/common) (`@v2`); pobieranie binarek: `go run github.com/dcc-bigfred/common/cmd/fetch@latest`
- Szczegółowe menu EAP613: [plans/2026-07-14-eap613-konfiguracja.md](../../plans/2026-07-14-eap613-konfiguracja.md)
