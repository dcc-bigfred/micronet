# Instrukcja konfiguracji: 3× TP-Link EAP613 + TL-SF1006P (BigFred event)

Data: 2026-07-14  
Powiązany plan: [2026-07-14-topologia-wifi-hala.md](./2026-07-14-topologia-wifi-hala.md)  
Montaż krok po kroku: [../README.md](../README.md) (EN) / [../README_pl.md](../README_pl.md) (PL)  
Tryb: **Standalone** (bez kontrolera Omada, bez internetu) — OC200 opcjonalny, patrz README

---

## 1. Co konfigurujemy

| Element | Ilość | Rola |
|---------|-------|------|
| TP-Link EAP613 | 3 | WiFi dla WiFredów (2.4 GHz) i telefonów (5 GHz) |
| TP-Link TL-SF1006P | 1 | PoE + L2 switch |
| BigFred | 1 | Serwer, DHCP, mDNS, WebSocket DCC |

**Cel:** latency WiFi < 25 ms dla ~40 klientów sterujących.

---

## 2. Podłączenie fizyczne

```
BigFred ──port 1──► TL-SF1006P ◄──port 2── AP1 (EAP613)
                              ◄──port 3── AP2 (EAP613)
                              ◄──port 4── AP3 (EAP613)
                              port 5–6 wolne
```

| Port TL-SF1006P | Urządzenie | Uwagi |
|-----------------|------------|-------|
| 1 | BigFred | **Priority Mode ON** na switchu (przełącznik z tyłu) |
| 2 | AP1 | PoE |
| 3 | AP2 | PoE |
| 4 | AP3 | PoE |
| 5–6 | — | rezerwa |

Na switchu:
- **Extend Mode: OFF** (inaczej porty spadną do 10 Mb/s).
- **Priority Mode: ON** (port 1 ma priorytet).

Montaż AP: **2 m**, dysk poziomo na maszcie/statywie, wokół strefy operatorów (nie za makietą przy publiczności).

---

## 3. Adresacja IP (zalecana)

| Urządzenie | IP | Uwagi |
|------------|-----|-------|
| BigFred | `10.0.10.1/24` | statyczny |
| AP1 | `10.0.10.11/24` | statyczny (zalecane) |
| AP2 | `10.0.10.12/24` | statyczny (zalecane) |
| AP3 | `10.0.10.13/24` | statyczny (zalecane) |
| Brama | `10.0.10.1` | BigFred |
| DHCP pool (na BigFredzie) | `10.0.10.50` – `10.0.10.200` | klienci WiFi |
| Maska | `255.255.255.0` | — |

> DHCP serwuje BigFred. AP nie muszą robić NAT ani routingu — tylko most L2 + WiFi.

---

## 4. Dostęp do panelu EAP613 (standalone)

Wykonaj **osobno dla każdego AP** (AP1 → AP2 → AP3).

### 4.1 Pierwsze logowanie

1. Podłącz AP do switcha, poczekaj ~2 min na boot.
2. Na telefonie/laptopie połącz się z domyślnym SSID z naklejki na spodzie AP:
   - `TP-Link_2.4G_XXXXXX` lub `Omada_2.4G_XXXXXX`
3. Otwórz przeglądarkę:
   - `https://tplinkeap.net`  
   - jeśli nie działa: `https://192.168.0.254`
4. Login fabryczny: `admin` / `admin`
5. Ustaw **nowy login i hasło zarządzania** (zapisz w bezpiecznym miejscu).
6. W Quick Setup możesz pominąć tworzenie docelowych SSID — zrobimy to ręcznie poniżej.

### 4.2 Dostęp po podłączeniu do sieci BigFred

Gdy AP ma IP z DHCP BigFreda (lub statyczne `10.0.10.11–13`):

- Wejdź na `http://10.0.10.11` (lub `.12` / `.13`) z laptopa w tej samej podsieci.

### 4.3 (Opcjonalnie) Statyczne IP AP

**System → Network** (lub **Device → Network** — zależnie od wersji firmware):

| Parametr | Wartość |
|----------|---------|
| IP Assignment | Static |
| IP Address | patrz tabela w §3 |
| Subnet Mask | `255.255.255.0` |
| Default Gateway | `10.0.10.1` |
| Primary DNS | `10.0.10.1` (lub puste) |

Zapisz. AP może na chwilę się rozłączyć — wejdź pod nowym IP.

---

## 5. Nazewnictwo AP (dla porządku)

W **System → Device Info** ustaw:

| Fizyczny AP | Device Name | IP |
|-------------|-------------|-----|
| AP przy stanowisku A | `BigFred-AP1` | `10.0.10.11` |
| AP przy stanowisku B | `BigFred-AP2` | `10.0.10.12` |
| AP przy stanowisku C | `BigFred-AP3` | `10.0.10.13` |

---

## 6. Zestaw ustawień — tabela główna

Poniżej **wartości docelowe**. Kolumny AP1/AP2/AP3 różnią się tylko kanałami.

### 6.1 Radio — 2.4 GHz (krytyczne dla WiFredów)

| Parametr | AP1 | AP2 | AP3 | Menu |
|----------|-----|-----|-----|------|
| 2.4 GHz Radio | **ON** | **ON** | **ON** | Wireless → Basic / Radio |
| Wireless Mode | **802.11b/g/n/ax** (lub najszerszy z `ax`) | j.w. | j.w. | Wireless → Basic / Radio |
| Channel Width | **20 MHz** | **20 MHz** | **20 MHz** | Wireless → Basic / Radio |
| Channel | **1** | **6** | **11** | Wireless → Basic / Radio |
| Channel Selection | **Manual** (nie Auto) | Manual | Manual | Wireless → Basic / Radio |
| Tx Power | **Medium** | **Medium** | **Medium** | Wireless → Basic / Radio |
| Minimum Data Rate* | **6 Mbps** (jeśli dostępne) | j.w. | j.w. | Wireless → Advanced / Radio |

\* Jeśli w firmware nie ma „Minimum Data Rate”, zostaw domyślne — kluczowe jest **20 MHz** i **ręczny kanał**.

### 6.2 Radio — 5 GHz (telefony operatorów)

| Parametr | AP1 | AP2 | AP3 | Menu |
|----------|-----|-----|-----|------|
| 5 GHz Radio | **ON** | **ON** | **ON** | Wireless → Basic / Radio |
| Wireless Mode | **802.11a/n/ac/ax** (lub najszerszy z `ax`) | j.w. | j.w. | Wireless → Basic / Radio |
| Channel Width | **40 MHz** | **40 MHz** | **40 MHz** | Wireless → Basic / Radio |
| Channel | **36** | **149** | **44** (lub **157**) | Wireless → Basic / Radio |
| Channel Selection | **Manual** | Manual | Manual | Wireless → Basic / Radio |
| Tx Power | **Medium** | **Medium** | **Medium** | Wireless → Basic / Radio |

> **Nie używaj kanałów DFS** (52–144) — radar może wyłączyć radio na kilka minut.

### 6.3 Zaawansowane radio (oba pasma, wspólne)

**Wireless → Advanced Settings** (lub **Wireless → Advanced**):

| Parametr | Wartość | Uzasadnienie |
|----------|---------|--------------|
| Beacon Interval | **100** ms (domyślne) | standard |
| DTIM Period | **1** (domyślne) | ważne dla mDNS/multicast |
| RTS Threshold | **2347** (domyślne) | — |
| Fragmentation Threshold | **2346** (domyślne) | — |
| Airtime Fairness | **ON** | szybsi klienci nie blokowani przez wolnych |
| OFDMA | **ON** (jeśli widoczne; na EAP613 zwykle domyślnie włączone w 802.11ax) | mniejsze opóźnienia przy wielu klientach |
| MU-MIMO | **ON** (jeśli widoczne) | — |
| Omada Mesh | **OFF** | mamy wired backhaul |
| Band Steering | **OFF** | mamy osobne SSID per pasmo — steering zbędny |

### 6.4 Load Balance (opcjonalnie, zalecane na 2.4 GHz)

**Wireless → Load Balance** → zakładka **2.4 GHz**:

| Parametr | Wartość |
|----------|---------|
| Load Balance | **ON** |
| Max Associated Clients | **18** (≈ 40 WiFredów / 3 AP, z zapasem) |

Na **5 GHz** możesz zostawić **OFF** lub ustawić limit **25**.

---

## 7. SSID — dokładna konfiguracja

Usuń lub wyłącz **domyślne SSID** z quick setup (bez hasła = ryzyko).

Hasła poniżej to **propozycje** — ustaw własne i zapisz w jednym miejscu dla zespołu.

### 7.1 SSID `BigFred-Throttle` (tylko 2.4 GHz — WiFredy)

**Wireless → Wireless Settings → 2.4 GHz → Add (+)**

| Parametr | Wartość |
|----------|---------|
| SSID | `BigFred-Throttle` |
| SSID Broadcast | **ON** |
| Security Mode | **WPA-PSK** |
| Version | **WPA2-PSK** |
| Encryption | **AES** |
| Wireless Password | np. `BigFred-Throttle-2026!` |
| Wireless VLAN ID | **0** (brak VLAN tagu) |
| Portal | **OFF** |
| SSID Isolation | **OFF** na start* |
| Status | **Enabled** |

\* Po teście mDNS (§9) możesz włączyć **SSID Isolation ON** — tylko jeśli WiFredy i telefony łączą się ze **stałym IP/hostem BigFreda** (`10.0.10.1`), a nie polegają na discovery.

**Na 5 GHz:** ten SSID **nie tworzymy**.

### 7.2 SSID `BigFred` (tylko 5 GHz — telefony operatorów)

**Wireless → Wireless Settings → 5 GHz → Add (+)**

| Parametr | Wartość |
|----------|---------|
| SSID | `BigFred` |
| SSID Broadcast | **ON** |
| Security Mode | **WPA-PSK** |
| Version | **WPA2-PSK** |
| Encryption | **AES** |
| Wireless Password | np. `BigFred-Phones-2026!` |
| Wireless VLAN ID | **0** |
| Portal | **OFF** |
| SSID Isolation | **OFF** |
| Status | **Enabled** |

**Na 2.4 GHz:** ten SSID **nie tworzymy**.

### 7.3 Podsumowanie SSID per AP

Każdy z 3 AP ma **identyczne SSID i hasła**, różnią się tylko kanałami radio.

| SSID | Pasmo | Klienci |
|------|-------|---------|
| `BigFred-Throttle` | 2.4 GHz only | WiFredy (ESP32-C6), ewentualnie telefony 2.4-only |
| `BigFred` | 5 GHz only | Telefony operatorów |

---

## 8. Kolejność pracy — krok po kroku

Zrób to **3 razy** (raz per AP). Najpierw AP1, potem AP2, potem AP3.

```
[ ] 1. Podłącz AP do switcha, nadaj statyczne IP (§3, §4.3)
[ ] 2. Ustaw Device Name (§5)
[ ] 3. Skonfiguruj 2.4 GHz radio (§6.1) — KANAŁ wg tabeli AP1/2/3
[ ] 4. Skonfiguruj 5 GHz radio (§6.2) — KANAŁ wg tabeli AP1/2/3
[ ] 5. Ustaw Advanced (§6.3): Airtime Fairness ON, Mesh OFF
[ ] 6. (Opcja) Load Balance 2.4 GHz: max 18 klientów (§6.4)
[ ] 7. Dodaj SSID BigFred-Throttle na 2.4 GHz (§7.1)
[ ] 8. Dodaj SSID BigFred na 5 GHz (§7.2)
[ ] 9. Usuń/wyłącz domyślne SSID TP-Link/Omada
[ ] 10. Save / Apply — poczekaj ~60 s
[ ] 11. Sprawdź, czy oba SSID są widoczne (skaner WiFi)
[ ] 12. Przejdź do następnego AP
```

### Szybka ściągawka kanałów

| AP | 2.4 GHz (20 MHz) | 5 GHz (40 MHz) |
|----|------------------|----------------|
| **AP1** (`10.0.10.11`) | ch **1** | ch **36** |
| **AP2** (`10.0.10.12`) | ch **6** | ch **149** |
| **AP3** (`10.0.10.13`) | ch **11** | ch **44** lub **157** |

---

## 9. Walidacja po konfiguracji

### 9.1 Skan widma (na miejscu, przed eventem)

Na telefonie z **WiFi Analyzer** (Android) lub podobną aplikacją:

- [ ] Widać 3 sieci `BigFred-Throttle` na kanałach **1, 6, 11** (po jednej na kanał).
- [ ] Widać 3 sieci `BigFred` na 5 GHz (36, 149, 44/157).
- [ ] Jeśli któryś kanał jest zajęty przez obcy hotspot — zamień na najczystszy z {1, 6, 11} / {36, 44, 149, 157}.

### 9.2 Pokrycie (RSSI)

Przy każdym stanowisku operatora:

- [ ] Połącz WiFreda z `BigFred-Throttle`.
- [ ] RSSI **> −65 dBm** (w panelu AP: **Status → Clients** lub aplikacja WiFi).
- [ ] Jeden AP dominujący — unikaj „skakania” między AP.

### 9.3 Discovery i latency

- [ ] Telefon na `BigFred` (5 GHz) otwiera stronę BigFreda (`http://10.0.10.1` lub hostname).
- [ ] WiFred łączy się z BigFredem (mDNS lub stały IP).
- [ ] Ping / RTT z klienta do `10.0.10.1` **< 25 ms** (średnio).
- [ ] Test throttle: zmiana prędkości/lokomotywy bez zauważalnej zwłoki.

### 9.4 mDNS i izolacja

- [ ] Discovery działa przy **SSID Isolation OFF**.
- [ ] Jeśli włączysz izolację: potwierdź, że WiFredy mają **wpisany stały adres BigFreda** — inaczej discovery padnie.

### 9.5 Telefony bez internetu

- [ ] Android/iOS mogą pokazać „Brak internetu” — to **OK**, nie rozłączaj sieci.
- [ ] Operatorzy wchodzą **bezpośrednio** na adres BigFreda (zakładka / QR / skrót).

---

## 10. Ustawienia, których NIE włączać

| Funkcja | Ustawienie | Dlaczego |
|---------|------------|----------|
| Portal / Captive Portal | **OFF** | blokuje dostęp bez internetu |
| Omada Mesh | **OFF** | mamy kable; mesh zjada 2.4 GHz i dodaje latency |
| 40 MHz na 2.4 GHz | **NIE** | tylko 20 MHz — więcej interferencji, mniej kanałów |
| Auto Channel (2.4 GHz) | **NIE** | AP muszą trzymać 1/6/11 sztywno |
| DFS na 5 GHz (52–144) | **NIE** | ryzyko przerw w radiu |
| Extend Mode na switchu | **OFF** | ogranicza do 10 Mb/s |
| WEP / TKIP | **NIE** | słabe, psuje wydajność 802.11n/ax |

---

## 11. Dzień eventu — szybka checklista

**Rano, przed publicznością:**

- [ ] UPS na BigFred + switch (jeśli masz).
- [ ] 3 AP na 2 m, zasilone, linki PoE świecą.
- [ ] BigFred na `10.0.10.1`, DHCP działa.
- [ ] Skan kanałów — ewentualna korekta 1/6/11.
- [ ] 3–5 WiFredów testowych na różnych stanowiskach — throttle OK.
- [ ] Prośba do publiczności: **wyłączyć hotspoty osobiste**.

**W trakcie:**

- [ ] Panel AP (`Status → Clients`) — czy klienci rozkładają się na 3 AP.
- [ ] Jeśli jeden AP ma >18 klientów 2.4 GHz — rozważ przesunięcie AP lub włączenie load balance.

**Awaria:**

- [ ] Zapasowy AP + injector PoE pod ręką.
- [ ] Kabel Ethernet zapasowy.

---

## 12. Rozwiązywanie problemów

| Objaw | Prawdopodobna przyczyna | Działanie |
|-------|-------------------------|-----------|
| Wysoki ping (>50 ms) | zatłoczone 2.4 GHz, hotspoty | skan kanałów; telefony tylko na `BigFred` 5G; prośba o wyłączenie hotspotów |
| WiFred się rozłącza | słaby RSSI, roaming | przesuń AP; sprawdź RSSI > −65 dBm; zostań przy jednym AP |
| Telefon nie widzi BigFreda | zły SSID / 5 GHz only | użyj `BigFred` (5 GHz); wejdź na `http://10.0.10.1` ręcznie |
| mDNS nie działa | SSID Isolation ON | wyłącz izolację lub ustaw stały IP BigFreda w WiFredzie |
| Wolne ładowanie ikon | normalne przy <200 kB | akceptowalne; nie wpływa na DCC jeśli ping OK |
| AP nie wstaje | PoE | sprawdź port switcha; budżet 67 W wystarcza na 3× ~11 W |

---

## 13. Eksport ustawień (zalecenie)

Po poprawnej konfiguracji **AP1**:

1. W panelu: **System → Backup & Restore → Backup** (jeśli dostępne).
2. Zapisz plik `bigfred-ap1-backup.bin`.
3. Dla AP2/AP3: zmień **tylko kanały** i **IP/statyczną nazwę** — reszta identyczna.

> W standalone każdy AP ma własną konfigurację — backup przyspiesza odtworzenie po awarii.

---

## 14. Podsumowanie zestawu docelowego

| Składnik | Model | Ilość | Koszt orient. |
|----------|-------|-------|---------------|
| Access Point | TP-Link EAP613 (AX1800) | 3 | ~840–990 zł |
| Switch PoE | TP-Link TL-SF1006P | 1 | ~130–160 zł |
| Statywy 2 m | dowolne stabilne | 3 | poza budżetem AP |

**Klucz konfiguracji:** 2.4 GHz **1/6/11 @ 20 MHz** dla WiFredów, 5 GHz **non-DFS** dla telefonów, **OFDMA + Airtime Fairness ON**, **wired backhaul**, **bez mesh i bez portalu**.
