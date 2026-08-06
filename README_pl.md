# BigFred event WiFi — montaż i konfiguracja

**Język:** [English](./README.md) | Polski

Powiązane plany: [topologia](./plans/2026-07-14-topologia-wifi-hala.md), [ustawienia EAP613](./plans/2026-07-14-eap613-konfiguracja.md)

Dla mało technicznego operatora. Cel: WiFi o niskim opóźnieniu dla pilotów (`bigfred2`, 2.4 GHz) i telefonów (`bigfred5`, 5 GHz).

## Co potrzebujesz

- Raspberry Pi 3 + Ethernet = **BigFred** (serwer)
- Omada **EAP610/613 × 3** (access pointy)
- Switch PoE **TL-SF1006P** (porty 1–4 PoE+, 5–6 zwykłe)
- **Omada OC200** jest opcjonalny (centralny kontroler). Bez niego konfigurujesz każdy AP w trybie **standalone** (te same SSID/ustawienia; różnią się tylko kanały).
- 4–5 kabli Ethernet, zasilacze (Pi3, switch; OC200 jeśli jest), 3 statywy na **2 m**, laptop/telefon do konfiguracji, opcjonalnie UPS

## Jak działa sieć na BigFredzie

Po starcie BigFred OS:

1. Podnosi Ethernet (`configure-ethernet`).
2. Uruchamia **`configure-dhcp`**, które sonduje LAN pod kątem stacka WiFi eventowego (dziś: **Omada** AP lub OC200).
3. **Tylko gdy wykryje sprzęt Omada** ustawia BigFred na `10.0.10.1/24` i startuje **dnsmasq** (pula `10.0.10.50–10.0.10.200`, lease **7 dni**, brama/DNS = BigFred). Wykryte MAC Omada dostają stałe rezerwacje DHCP.
4. W sieci klubowej **bez** Omada DHCP **nie** startuje (brak konfliktu z klubowym DHCP).

Nie edytujesz dnsmasq ręcznie pod setup eventu.

---

## 1. Okablowanie (przed włączeniem prądu)

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

## 2. Przełączniki z tyłu switcha

- [ ] **Priority Mode = ON** (port 1 = BigFred)
- [ ] **Extend Mode = OFF** (inaczej porty spadną do 10 Mb/s)

## 3. Kolejność włączania

Najpierw BigFred (DHCP):

- [ ] 1. Switch
- [ ] 2. BigFred — poczekaj ~2 min (`configure-dhcp` wykryje Omada i uruchomi DHCP)
- [ ] 3. OC200 (jeśli jest) — poczekaj ~3 min
- [ ] 4. AP1/2/3 przez PoE — poczekaj ~3 min

## 4. Laptop w sieci

- [ ] Ethernet do portu 6 (laptop dostanie adres z BigFreda, np. `10.0.10.51`)

## 5. Konfiguracja WiFi — wybierz ścieżkę

### Ścieżka A — z OC200 (kontroler)

- [ ] Znajdź IP OC200 (**Omada Discovery** od TP-Link albo na BigFredzie: `configure-dhcp check`)
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
- [ ] Na `bigfred5` otwórz `http://10.0.10.1` (BigFred)
- [ ] Pilot na `bigfred2`
- [ ] Ping do `10.0.10.1` &lt; 25 ms
- [ ] RSSI na stanowiskach &gt; −65 dBm

## 9. Checklist dnia eventu

- [ ] 3 AP na 2 m wokół operatorów (nie za makietą)
- [ ] BigFred na porcie 1 (Priority), DHCP działa
- [ ] Skan widma — ew. korekta 1/6/11
- [ ] 3–5 pilotów testowych OK
- [ ] Prośba do publiczności: wyłączyć hotspoty
- [ ] Zapasowy AP + injector PoE

## Uwagi techniczne

- Narzędzia (workspace Rust): [`crates/configure-dhcp`](./crates/configure-dhcp/), [`crates/configure-ethernet`](./crates/configure-ethernet/) → `/usr/sbin/` na BigFred OS (pakiet OCI `micronet`)
- Lokalny publish OCI: `make publish-oci` (klonuje wspólne skrypty z `dcc-bigfred/.github` @ `v1` do `.ci-github/`)
- Szczegółowe menu EAP613: [plans/2026-07-14-eap613-konfiguracja.md](./plans/2026-07-14-eap613-konfiguracja.md).
