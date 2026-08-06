# BigFred — topologia WiFi dla eventu w hali 300 m² (low-latency DCC)

Data: 2026-07-14
Autor analizy: czyste spojrzenie (bez oparcia o `docs/`)
Status: projekt do wdrożenia i walidacji na miejscu

**Instrukcja montażu (EN/PL):** [../README.md](../README.md) / [../README_pl.md](../README_pl.md)  
**Instrukcja konfiguracji AP:** [2026-07-14-eap613-konfiguracja.md](./2026-07-14-eap613-konfiguracja.md)

---

## 1. Kontekst i cel

Uruchomienie BigFreda na evencie w warunkach:

- Hala ~300 m².
- Makieta ~150 m otaczająca 40 operatorów (klientów sterujących).
- Za makietą publiczność ~100 osób.
- Wokół ~200 obcych telefonów (potencjalne hotspoty osobiste + Bluetooth).
- Brak internetu. Sieć wyłącznie lokalna (LAN/WLAN).
- Klienci gadają z BigFredem po LAN; komunikacja klient–klient niepotrzebna.

Wymagania:

- Sterowanie DCC maksymalnie responsywne — latency WiFi < 25 ms.
- Dobry zasięg na całej strefie operatorów.
- Odporność na zakłócenia (mało obcych AP, ale dużo telefonów).
- ~40 klientów z niskim latency jednocześnie.
- Klienci: telefony operatorów (nieznany sprzęt, dual-band) oraz dedykowane WiFredy (ESP32-C6 + antena 4 dBi).

Ograniczenia potwierdzone z użytkownikiem:

- Wired backhaul do każdego AP dostępny (kabel Ethernet).
- Budżet: pierwotnie 2× AP ≤ 1000 zł (switch/PoE/kable osobno); zaakceptowano wariant 3× EAP613 mieszczący się w ~1000 zł.
- Montaż AP maksymalnie na 1,5–2 m — brak dostępu do sufitu.
- Profil ruchu: tylko małe pakiety + sporadyczne obrazki < 200 kB (ikony, drobne zdjęcia). Brak ciężkiego ruchu przepustowościowego.

---

## 2. Kluczowe ustalenia techniczne

Z analizy kodu BigFreda i researchu sprzętu:

- **WiFred = ESP32-C6** → WiFi 6 **tylko 2.4 GHz**, 1×1 (jeden strumień), **tylko 20 MHz** w trybie 802.11ax, TX ~19 dBm, antena 4 dBi. Wspiera OFDMA (up/down), TWT, beamformee, DCM. To najważniejsze ograniczenie projektu: **krytyczne klienty żyją na najbardziej zatłoczonym paśmie 2.4 GHz**.
- **Transport BigFred: WebSocket po TCP** (`bigfred/web/src/hooks/useWsConnection.ts`, `bigfred/web/src/context/SocketContext.tsx`) → wrażliwy na straty pakietów (retransmisja TCP = skok latency). Priorytetem jest **niska strata pakietów**, nie przepustowość.
- **Discovery przez mDNS/DNS-SD** — BigFred ogłasza usługi `_withrottle._tcp` i `_z21._udp` (`bigfred/pkgs/bigfred/mdns` + `dcc-bus/discovery`). Wykrywanie klientów zależy od multicastu → wpływ na decyzje o izolacji klientów i filtrowaniu multicastu.

Konsekwencja: cały projekt sprowadza się do **utrzymania czystego, mało obciążonego pasma 2.4 GHz dla WiFredów** i zdjęcia z niego wszystkiego, co się da (telefony → 5 GHz).

---

## 3. Analiza środowiska RF

- Ruch DCC to **małe, częste pakiety**. Latency jest zdominowane przez **rywalizację o medium (airtime) i retransmisje**, a NIE przez przepustowość. Klucz do < 25 ms: czyste 2.4 GHz + OFDMA + minimalny narzut zarządzania.
- **200 obcych telefonów**: same w LTE to umiarkowany szum (skanowanie WiFi, Bluetooth). Realne ryzyko to **hotspoty osobiste** (domyślnie 2.4 GHz) — kilka włączonych potrafi zauważalnie zapchać pasmo.
- **~140 ciał ludzkich** (40 operatorów + 100 widzów) tłumi 2.4 GHz o ~3–6 dB na ciało i tworzy multipath. Bardzo istotne przy trzymanych w dłoni WiFredach i przy niskim montażu AP.
- **2.4 GHz ma tylko 3 nienakładające się kanały** (1/6/11) przy 20 MHz — to naturalnie sprzyja wariantowi 3-AP.

---

## 4. Topologia (płaski L2, wired backhaul)

```mermaid
flowchart TD
  BigFred["BigFred (serwer, IP statyczny)"] -->|Ethernet| SW["Switch PoE TL-SF1006P (port 1 = Priority)"]
  SW -->|"PoE + backhaul"| AP1["AP1 WiFi6 EAP613\n2.4G ch1\n5G ch36"]
  SW -->|"PoE + backhaul"| AP2["AP2 WiFi6 EAP613\n2.4G ch6\n5G ch149"]
  SW -->|"PoE + backhaul"| AP3["AP3 WiFi6 EAP613 (wariant A)\n2.4G ch11\n5G ch44"]
  AP1 -. 2.4GHz .-> WiFredy["~40 WiFredow (ESP32-C6)"]
  AP2 -. 2.4GHz .-> WiFredy
  AP3 -. 2.4GHz .-> WiFredy
  AP1 -. 5GHz .-> Fony["Telefony operatorow"]
  AP2 -. 5GHz .-> Fony
  AP3 -. 5GHz .-> Fony
```

- **Wariant A (rekomendowany)**: 3 AP na kanałach 2.4 GHz 1/6/11.
- **Wariant B (fallback)**: 2 AP na kanałach 2.4 GHz 1/11 (AP3 pomijamy).
- **Jeden VLAN / jedna podsieć** dla wszystkich klientów + BigFred → mDNS działa natywnie. Bez routingu, bez internetu.
- **Wired backhaul (nie mesh)** — zero dodatkowego latency i zero zjadania pasma 2.4 GHz na backhaul.

---

## 5. Dobór sprzętu

### 5.1 Wariant A — REKOMENDOWANY (niski montaż 2 m + tłum)

**3× TP-Link Omada EAP613 (AX1800, WiFi 6)** — ~280–330 zł/szt → **~840–990 zł** (w budżecie ~1000 zł).

Specyfikacja istotna dla nas:

- 2.4 GHz: 574 Mb/s, anteny 2× 4 dBi; 5 GHz: 1201 Mb/s, anteny 2× 5 dBi.
- **OFDMA, MU-MIMO, 1024-QAM, band steering, airtime fairness, beamforming, roaming 802.11k/v/r.**
- Port Gigabit, zasilanie 802.3af/at PoE (~11 W), tryb **standalone lub Omada**.

Dlaczego 3 AP w tym scenariuszu:

- Wykorzystuje **wszystkie 3 czyste kanały 2.4 GHz (1/6/11)** — brak co-channel między naszymi AP.
- 3 punkty pokrycia = **mniejsze komórki i mniej dziur** w tłumie przy montażu na 2 m.
- Rozłożenie ~40 WiFredów na 3 radia 2.4 GHz zamiast 2 → mniejsza rywalizacja o airtime.

> EAP610 to równoważna (starsza, często tańsza) alternatywa — jeśli dostępna taniej, również pasuje.

### 5.2 Wariant B — fallback (mocniejszy AP, mniej punktów)

**2× TP-Link Omada EAP650 (AX3000, WiFi 6)** — ~450–500 zł/szt → **~900–1000 zł**.

- OFDMA, MU-MIMO, airtime fairness, band steering.
- Lepszy w otwartej przestrzeni / przy montażu pod sufitem. Przy montażu 2 m w tłumie **2 punkty dają gorsze pokrycie niż 3 słabsze** — stąd niższy priorytet.

### 5.3 Switch PoE (poza budżetem AP)

**TP-Link TL-SF1006P** — w pełni wystarczający dla obu wariantów. ~130–160 zł.

- 6 portów, **4× PoE+ (802.3af/at, do 30 W/port, budżet 67 W)**, unmanaged, plug-and-play.
- Dla 3 AP: 3 porty PoE na AP + 1 port na BigFreda = 4/6 portów zajęte. Pobór 3× ~11 W = **~33 W ≪ 67 W** budżetu.
- **Fast Ethernet 10/100 Mb/s to zero problemu** przy potwierdzonym profilu ruchu (małe pakiety DCC + sporadyczne obrazki < 200 kB, brak internetu). 100 Mb/s full-duplex daje ogromny zapas, serializacja ramki ~0,12 ms — bez wpływu na cel < 25 ms.
- Unmanaged pasuje do płaskiego L2 (bez VLAN). mDNS przejdzie (flood na małej sieci pomijalny), IGMP/multicast-to-unicast realizujemy na AP.
- Bonus: **Priority Mode na portach 1–2** → podłączyć BigFreda pod port 1.
- Gigabit (TL-SG1005P / SG1008P) tylko jeśli w przyszłości pojawi się cięższy ruch — obecnie zbędny.

### 5.4 Czego unikać

- Konsumenckich routerów w trybie AP (słabe zarządzanie high-density, brak strojenia).
- Sprzętu WiFi 5 (brak OFDMA = utrata kluczowego zysku latency dla wielu małych pakietów).

### 5.5 Dodatki

- 3× stabilny statyw / maszt / uchwyt ścienny (2 m).
- Kable Ethernet + osłony/taśma do bezpiecznego prowadzenia w hali z ludźmi.
- UPS dla BigFreda i switcha (opcjonalnie, ale zalecane).
- Zapasowy 1× AP + injector 802.3at (rezerwa awaryjna).

---

## 6. Plan pasm i kanałów

**2.4 GHz (WiFredy — krytyczne), zawsze 20 MHz (NIGDY 40 MHz):**

- Wariant A (3 AP): AP1 = ch 1, AP2 = ch 6, AP3 = ch 11 — komplet nienakładających się kanałów.
- Wariant B (2 AP): AP1 = ch 1, AP2 = ch 11.
- Ostateczne kanały **potwierdzić skanem widma na miejscu** (obce hotspoty mogą wymusić korektę).

**5 GHz (telefony operatorów), 40 MHz, tylko non-DFS (unikamy 52–144):**

- Wariant A (3 AP): ch 36, 149, 44 (lub 157).
- Wariant B (2 AP): ch 36, 149.

**Strategia SSID:**

- `BigFred-Throttle` — **tylko 2.4 GHz**, dedykowany WiFredom (i ewentualnym urządzeniom 2.4-only).
- `BigFred` — **tylko 5 GHz**, dla telefonów operatorów.
- Rozdzielone nazwy → pełna kontrola, telefony nie rywalizują z WiFredami na 2.4 GHz. Oba SSID w tej samej podsieci (dostęp do BigFreda + mDNS).

---

## 7. Konfiguracja AP (tuning pod latency)

- Włączyć **OFDMA (up i down)** oraz **WMM/QoS**.
- **Wyłączyć niskie legacy rates** (1/2/5.5/11 Mb/s); ustawić min basic rate ~6 Mb/s → szybkie beacony/mgmt, mniej zajętego airtime.
- 2.4 GHz: **20 MHz**, TX power **umiarkowane** i dopasowane do 1×1 WiFreda (nie na maksa — inaczej AP „słyszy” daleko, a słaby WiFred nie dosyła ramki uplink → asymetria i retransmisje).
- **Multicast**: IGMP snooping + multicast-to-unicast, ale **zostawić działający mDNS** (discovery WiThrottle/z21).
- **Izolacja klientów**: można włączyć (klient–klient niepotrzebny, mniej ARP/broadcast) — ALE przetestować, czy mDNS z serwera (strona wired) nadal dociera do klientów. Jeśli discovery padnie: wyłączyć izolację lub użyć mDNS reflectora/repeatera.
- **Band steering**: kierować własne telefony na 5 GHz; WiFredy i tak zostaną na 2.4 GHz.
- **Roaming**: włączyć 802.11k/v; operatorzy raczej stacjonarni, więc minimalizować przełączanie — dążyć do jednego dominującego AP na stanowisko.
- **DHCP**: pula ≥ 60 adresów; rezerwacja/statyk dla BigFreda; sensowny czas dzierżawy (unikać burzy odnawiania). BigFred na stałym IP.

---

## 8. Rozmieszczenie AP (montaż max 1,5–2 m, brak sufitu)

- **KRYTYCZNE: montować na 2 m, NIE 1,5 m.** Głowa stojącego człowieka to ~1,7–1,8 m; przy 1,5 m antena jest poniżej linii głów → ciała tłumią tor radiowy do większości klientów. 2 m daje antenie przewagę nad tłumem (LOS ponad głowami).
- **Mocowanie**: stabilne statywy (fotograficzne/głośnikowe), maszty lub uchwyty ścienne na 2 m; kabel PoE poprowadzony bezpiecznie (taśma/osłony — hala z ludźmi).
- **Orientacja anteny**: AP typu „puck” (EAP613) ma główny lobe prostopadle do dysku i dookolny wzór w płaszczyźnie dysku. Montować dysk poziomo na szczycie masztu/statywu (2 m) → „donut” pokrycia dookoła AP na wysokości operatorów. Alternatywnie na ścianie frontem do sali.
- **Geometria**:
  - Wariant A (3 AP): rozstawić w trójkącie / równomiernie wokół pierścienia 40 operatorów (**nie za makietą przy publiczności**), tak by każde stanowisko miało bliski AP.
  - Wariant B (2 AP): po przeciwnych stronach strefy operatorów; komórki nakładają się w środku.
  - Cel: **RSSI > −65 dBm** i jeden dominujący AP na każde stanowisko.
- Trzymać AP z dala od boosterów DCC / zasilaczy / silników (EMI) i od dużych metalowych elementów makiety (odbicia/tłumienie).
- Cel kanałowy: sąsiednie komórki 2.4 GHz (1/6/11) nakładają się zasięgowo, ale nie kanałowo → bezszwowe pokrycie bez co-channel.
- Uwaga pojemnościowa: montaż na 2 m w tłumie mocno kurczy komórki — dlatego **3 AP (1/6/11) są preferowane**. 2 AP to absolutne minimum, akceptowalne tylko jeśli pomiar na miejscu potwierdzi RSSI > −65 dBm na wszystkich stanowiskach.

---

## 9. Adresacja i discovery

- Jedna podsieć L2, np. `10.0.10.0/24` (lub dowolna prywatna). BigFred na stałym IP (np. `10.0.10.1`), podłączony pod port 1 switcha (Priority Mode).
- DHCP na BigFredzie lub na routerze/serwerze pełniącym rolę bramy w tej podsieci; pula ≥ 60 adresów.
- Discovery: BigFred rozgłasza `_withrottle._tcp` / `_z21._udp` przez mDNS. Utrzymać mDNS działający przez AP (nie blokować multicastu link-local 224.0.0.251).
- WiFredy (własny firmware) mogą używać stałego adresu/hosta BigFreda — najbezpieczniejsze, uniezależnia od mDNS i pozwala włączyć izolację klientów.

---

## 10. Słabe punkty i mitigacje

1. **Zatłoczenie 2.4 GHz (200 telefonów, hotspoty, BT) — ryzyko #1.** Mitigacja: telefony na 5 GHz (band steering + osobny SSID), 20 MHz na 2.4, skan i wybór czystych kanałów na miejscu, wyłączone niskie rates, izolacja/limit SSID, prośba do uczestników o wyłączenie hotspotów.
2. **Niski montaż (2 m, brak sufitu) + tłum ~140 osób — największy problem środowiskowy.** AP na wysokości tłumu, ciała tłumią 2.4 GHz, komórki się kurczą, więcej dziur i multipath. Mitigacja: bezwzględnie 2 m (nie 1,5 m), AP ponad głowami na masztach, **preferowany wariant 3 AP (1/6/11)**, pomiar RSSI na miejscu.
3. **Ograniczenia ESP32-C6 (1×1, słaby uplink).** Mitigacja: dobre pokrycie, beamforming (AP jako beamformer, C6 jako beamformee), umiarkowany TX AP, RSSI > −65 dBm w każdym punkcie.
4. **TCP/WebSocket wrażliwe na straty pakietów** (retransmisja = skok latency). Mitigacja: minimalizacja strat (pokrycie, czyste RF), QoS WMM; opcjonalnie po stronie aplikacji: znakowanie DSCP ruchu DCC do kolejki priorytetowej.
5. **Captive portal / brak internetu.** Telefony mogą ostrzegać „brak internetu” lub się odłączać. Mitigacja: odpowiadać sukcesem na sondy (captive.apple.com, connectivitycheck.gstatic.com, msftconnecttest) albo zaakceptować ostrzeżenia; operatorzy łączą się bezpośrednio ze stroną BigFreda.
6. **Roaming ESP32 (podstawowy).** Mitigacja: operatorzy stacjonarni → minimalizować roaming, 802.11k/v, jeden dominujący AP na stanowisko.
7. **Zasilanie/PoE i pojedyncze punkty awarii** (switch, host BigFred, pojedynczy AP). Mitigacja: zapasowy AP + injector, UPS dla switcha i BigFreda.
8. **mDNS pod izolacją/filtrem multicast.** Mitigacja: przetestować discovery przed eventem; ewentualnie stały IP BigFreda w WiFredach.

---

## 11. Budżet latency (orientacyjnie, przy czystym RF)

- Airtime WiFi 2.4 GHz z OFDMA (nieobciążone): ~2–8 ms RTT.
- Switch wired (Fast Ethernet): < 1 ms; serializacja ramki ~0,12 ms.
- Zapas do celu < 25 ms jest duży **pod warunkiem czystego pasma**.
- Ryzyko: zatłoczenie 2.4 GHz potrafi wywindować latency do 50–200 ms — stąd cały nacisk na czystość pasma i odciążenie go z telefonów.

---

## 12. Checklista dnia eventu

**Przed (montaż):**

- [ ] Rozstawić 3 AP na statywach/masztach na 2 m wokół strefy operatorów (nie za makietą).
- [ ] Podłączyć wired backhaul do switcha; BigFred na port 1 (Priority Mode).
- [ ] Skonfigurować SSID: `BigFred-Throttle` (2.4 GHz), `BigFred` (5 GHz) — patrz [instrukcja EAP613](./2026-07-14-eap613-konfiguracja.md)
- [ ] Ustawić kanały 2.4 GHz 1/6/11, 20 MHz; 5 GHz non-DFS.
- [ ] Wyłączyć niskie legacy rates, włączyć OFDMA/WMM, band steering.
- [ ] Ustawić DHCP (pula ≥ 60), stały IP BigFreda.

**Skan widma i walidacja (na miejscu):**

- [ ] Skan 2.4 GHz (np. WiFiAnalyzer / narzędzie Omada) — wybrać najczystsze kanały; skorygować 1/6/11 jeśli trzeba.
- [ ] Pomiar RSSI na kilku stanowiskach operatorów — cel > −65 dBm; jeśli < −70 dBm, przesunąć AP / rozważyć 4. punkt.
- [ ] Test discovery: WiFred i telefon widzą BigFreda (mDNS lub stały IP).
- [ ] Test latency: ping/RTT WiFred↔BigFred < 25 ms; test pod obciążeniem (wiele klientów naraz).
- [ ] Test captive portal na kilku modelach telefonów (Android/iOS) — czy nie odłączają.

**W trakcie:**

- [ ] Monitorować obciążenie/airtime na AP (panel Omada).
- [ ] Prośba do uczestników/publiczności o wyłączenie hotspotów osobistych.
- [ ] Mieć zapasowy AP + injector pod ręką.

---

## 13. Podsumowanie zestawu (Wariant A)

- 3× TP-Link Omada EAP613 (AX1800) — ~840–990 zł.
- 1× TP-Link TL-SF1006P (4× PoE+) — ~130–160 zł.
- 3× statyw/maszt/uchwyt (2 m) + kable Ethernet + osłony.
- Konfiguracja: 2.4 GHz 1/6/11 @ 20 MHz dla WiFredów, 5 GHz non-DFS dla telefonów, płaski L2, wired backhaul, OFDMA + WMM + band steering.
