# BigFred event WiFi — montaż i konfiguracja

**Język:** [English](./README.md) | Polski

Powiązane plany: [topologia](../../plans/2026-07-14-topologia-wifi-hala.md), [ustawienia EAP613](../../plans/2026-07-14-eap613-konfiguracja.md)

Architektura daemona: [ARCHITECTURE.md](../../ARCHITECTURE.md).

Dla mało technicznego operatora. Cel: WiFi o niskim opóźnieniu dla pilotów (`bigfred2`, 2.4 GHz) i telefonów (`bigfred5`, 5 GHz).

## Co potrzebujesz

- Raspberry Pi **5** + Ethernet = **BigFred** (serwer)
- Omada **EAP610/613 × 3** (access pointy), **kablem** do switcha (to nie jest Omada Mesh)
- **Switch PoE TL-SF1006P** (porty 1–4 PoE+, 5–6 zwykłe) — **wymagany**. AP-y biorą PoE **tylko** z tego switcha
- **Omada OC200** — **wymagany** do Fast Roaming (802.11k/v). Kontroler musi zostać włączony; gdy padnie, Fast Roaming też pada. To samo SSID na wszystkich AP nadal pozwala na *zwykły* roaming (klient sam skanuje), ale bez sterowanego handoveru
- Opcjonalnie: **dowolny router z DHCP** na wolnym porcie switcha (nie jako PoE dla Omady). Wtedy BigFred ustępuje i **nie** serwuje DHCP
- Kable Ethernet, zasilacze, 3 statywy na **2 m**, laptop/telefon do konfiguracji, opcjonalnie UPS

## Jak działa sieć na BigFredzie

Po starcie daemon **`micronet`** (pierwszy fizyczny Ethernet **z kablem**, w przeciwnym razie pierwszy Ethernet):

1. Podnosi interfejs (bez adresu).
2. Wysyła **DHCPDISCOVER** i czeka na **DHCPOFFER** (bez REQUEST).
3. **Jest oferta** → tryb **`client`**: `dhclient`, bez dnsmasq, bez `gateway.ip` na Pi.
4. **Brak oferty** → tymczasowo `.252` w skonfigurowanej podsieci, potem `ping gateway.ip`:
   - ping OK → tryb **`static`**: zostań na `.252`, default via `gateway.ip`, bez dnsmasq
   - ping fail → tryb **`gateway`**: weź `gateway.ip` (seed obrazu: **`192.168.0.1/24`**), start **dnsmasq** (pula `.50–.200`, sticky **7d**, router/DNS = BigFred). **Bez default route.** Opcjonalne `dns.records` w `$DATA_DIR/etc/micronet.json` to nazwy unicast (np. `bigfred.lan`) na `gateway.ip`. W `client` / `static` tych nazw nie ma — zostaje mDNS `bigfred.local`.

Jeśli router pojawi się na switchu **później** (po tym, jak BigFred już został gatewayem), micronet wykryje obcy DHCP (okresowa sonda) i **ustąpi**: wyłączy dnsmasq i uruchomi `dhclient`. Gdy DHCP ma być na routerze, włącz router jako pierwszy.

Nie ma wykrywania Omady ani rezerwacji `dhcp-host=` per MAC. Stickiness to leasefile dnsmasq + `7d`.

Typowe mapowanie:

| Backhaul | Obcy DHCP / żywy `gateway.ip` | Tryb BigFred | Kto daje lease laptopowi |
|---|---|---|---|
| Sam TL-SF1006P | brak | `gateway` | dnsmasq na BigFredzie |
| TL-SF1006P + router na wolnym porcie | tak (router) | `client` albo `static` | router |

Nie edytujesz dnsmasq ręcznie pod setup eventu. JSON: `$DATA_DIR/etc/micronet.json` (hot-reload). Opcjonalne `"dns": { "enabled": true, "records": [ { "name": "bigfred.lan" } ] }` dodaje tradycyjne nazwy; bez `addr` używane jest `gateway.ip`, albo `addr` to IPv4.

---

## Zestaw A — Switch TL-SF1006P (BigFred = DHCP)

### 1. Okablowanie (przed włączeniem prądu)

| Port switcha | Urządzenie | Uwagi |
|---|---|---|
| 1 | BigFred | Priority Mode |
| 2 | AP1 | PoE |
| 3 | AP2 | PoE |
| 4 | AP3 | PoE |
| 5 | OC200 | Zwykły port; OC200 ma własny zasilacz (**wymagany** do Fast Roaming) |
| 6 | wolny | Laptop albo opcjonalny router DHCP |

- [ ] BigFred → port 1
- [ ] AP1 → 2, AP2 → 3, AP3 → 4
- [ ] OC200 → 5
- [ ] Zasilacze: switch, BigFred, OC200

### 2. Przełączniki z tyłu switcha

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (inaczej porty spadną do 10 Mb/s)

### 3. Kolejność włączania

Pusta hala: ping na `192.168.0.1` pada → BigFred od razu jest gatewayem. AP-y dostaną lease po starcie.

- [ ] 1. Switch
- [ ] 2. BigFred — poczekaj aż UI odpowie na `http://192.168.0.1` (~1–2 min)
- [ ] 3. OC200 — poczekaj ~3 min
- [ ] 4. AP1/2/3 przez PoE — poczekaj ~3 min

### 4. Laptop w sieci

- [ ] Ethernet do portu 6 — laptop dostanie adres **z BigFreda**, np. `192.168.0.51`

---

## Opcjonalnie — router DHCP na switchu

**Nie** zasilaj Omady z PoE MikroTika (ani innego routera): piny PoE nie są kompatybilne z EAP610/613. AP-y zostają na **TL-SF1006P**. Router wpinasz w wolny port switcha (np. 6), jeśli *on* ma serwować DHCP.

### Kolejność włączania

- [ ] 1. Switch
- [ ] 2. Router (poczekaj aż jego DHCP wstanie)
- [ ] 3. BigFred — dołącza jako **`client`** (albo **`static` `.252`**, gdy router nie ma DHCP, ale odpowiada na ping `gateway.ip`)
- [ ] 4. OC200, potem AP-y przez PoE switcha

### Laptop

- [ ] Włóż do switcha — lease daje **router**, nie BigFred.
- [ ] `micronet status` ma być `client` albo `static`, **nie** `gateway`.

---

## 5. Konfiguracja WiFi — przez OC200

### Ścieżka A — OC200 (wymagany do Fast Roaming)

[Fast Roaming TP-Link](https://support.omadanetworks.com/en/document/12972/) (802.11k/v) włącza się z kontrolera i **wymaga, żeby kontroler działał**. Omada Mesh to osobna funkcja (bezprzewodowy backhaul); ten zestaw ma AP-y **na kablu**, więc Mesh zostaje OFF.

- [ ] Znajdź IP OC200 (**Omada Discovery** od TP-Link)
- [ ] Otwórz `https://<ip-oc200>`, zignoruj ostrzeżenie certyfikatu
- [ ] Login `admin` / `admin`, ustaw nowe hasło admina
- [ ] Wizard: region/strefa; pomiń tworzenie SSID
- [ ] **Devices** → Adopt trzech AP → status **Connected**
- [ ] Utwórz WLAN + SSID (krok 6) i tweaki radia (krok 7) **raz** w kontrolerze
- [ ] Włącz **Fast Roaming** (802.11k/v) w kontrolerze; Mesh OFF

### Ścieżka B — standalone (bez sterowanego roamingu)

Tylko gdy nie ma OC200. Kroki 6–7 zrób **na każdym AP**. To samo SSID/hasło nadal pozwala telefonowi samemu się przełączyć (wolny skan). **Bez** Fast Roaming 802.11k/v, dopóki nie działa Omada Controller. Pierwszy dostęp: SSID z naklejki lub `https://tplinkeap.net` / `https://192.168.0.254`.

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
- [ ] Load balance 2.4 GHz: max ~18 klientów; Fast Roaming **802.11k/v** ON w kontrolerze (na EAP Omada Fast Roaming to k/v, nie 802.11r)

## 8. Walidacja

- [ ] Telefon widzi `bigfred2` i `bigfred5`
- [ ] Na `bigfred5` otwórz `http://192.168.0.1` (BigFred) przy seedzie eventu / zestawie switch
- [ ] Pilot na `bigfred2`
- [ ] Ping do huba &lt; 25 ms
- [ ] RSSI na stanowiskach &gt; −65 dBm

## 9. Checklist dnia eventu

- [ ] 3 AP na 2 m wokół operatorów (nie za makietą)
- [ ] Switch: BigFred na porcie 1 (Priority), OC200 na 5, AP-y na PoE 2–4
- [ ] Bez dodatkowego routera: `micronet status` → `gateway`, laptop z lease’em BigFreda
- [ ] Opcjonalny router na wolnym porcie switcha: `micronet status` → `client`/`static`; laptop z lease’em routera
- [ ] Skan widma — ew. korekta 1/6/11
- [ ] 3–5 pilotów testowych OK
- [ ] Prośba do publiczności: wyłączyć hotspoty
- [ ] Zapasowy AP + injector PoE

## Uwagi techniczne

- Daemon: [`crates/micronet`](../../crates/micronet/) → `/usr/sbin/micronet` na BigFred OS
- Wspólne CI: reusable workflows w [`dcc-bigfred/common`](https://github.com/dcc-bigfred/common) (`@v2`); pobieranie binarek: `go run github.com/dcc-bigfred/common/cmd/fetch@latest`
- Szczegółowe menu EAP613: [plans/2026-07-14-eap613-konfiguracja.md](../../plans/2026-07-14-eap613-konfiguracja.md)
