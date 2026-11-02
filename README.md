# Przeglądarka MRI

Prosty i szybki program do oglądania badań rezonansu magnetycznego (oraz innych badań w formacie DICOM, np. tomografii) skopiowanych z płyty z pracowni. Ma też wbudowany serwer MCP, dzięki któremu Claude (w aplikacji Claude Desktop lub w Claude Code) może sam przeglądać obrazy i pomagać w ich zrozumieniu.

> **To narzędzie informacyjne i edukacyjne, a nie wyrób medyczny.** Nie służy do diagnozy, badań przesiewowych, monitorowania choroby ani podejmowania decyzji o leczeniu, a pomiary są orientacyjne. Ani program, ani Claude nie zastępują opisu radiologa ani rozmowy z lekarzem.

## Instalacja na Macu

1. Wejdź w zakładkę [Releases](../../releases) i pobierz plik `MRI-Viewer-macOS.zip` z wydania „Najnowsza wersja”.
2. Rozpakuj go (dwuklik) i przeciągnij `MRI Viewer.app` do folderu **Programy** (Applications).
3. Program nie jest podpisany certyfikatem Apple, więc przy pierwszym uruchomieniu macOS go zablokuje. Najprościej otworzyć aplikację **Terminal** i wkleić:

   ```
   xattr -dr com.apple.quarantine "/Applications/MRI Viewer.app"
   ```

   Alternatywnie: spróbuj otworzyć program, potem wejdź w **Ustawienia systemowe → Prywatność i ochrona** i kliknij „Otwórz mimo to”.

Wersje dla Windows (`MRI-Viewer-Windows.zip`) i Linuksa (`MRI-Viewer-Linux.tar.gz`) są w tym samym miejscu.

## Jak oglądać badanie

1. Uruchom program i kliknij **„Otwórz folder z badaniem…”** (albo **Plik → Otwórz folder**, ⌘O). Wskaż folder skopiowany z płyty (np. `Pobrane/MRI`). Możesz też przeciągnąć folder na okno programu. Program sam przeszuka podfoldery i nie potrzebuje pliku DICOMDIR. Ostatnio otwierane foldery są na ekranie startowym.
2. Po lewej zobaczysz badania (z datą) i ich **serie**, czyli zestawy zdjęć wykonanych w różnych ustawieniach (np. T1, T2, STIR) i płaszczyznach: poprzecznej (plastry oglądane od stóp), strzałkowej (z boku) i czołowej (od przodu). Kliknij serię, aby ją wyświetlić.
3. Podstawowa obsługa (na Windowsie i Linuksie zamiast ⌘ używaj Ctrl):
   - **kółko myszy**, strzałki albo suwak na dole – kolejne przekroje; **Home/End** – pierwszy i ostatni, **spacja** albo przycisk odtwarzania – automatyczne przewijanie (od pierwszego obrazu),
   - **lewy przycisk** działa zgodnie z wybranym narzędziem na pasku: jasność/kontrast, przesuwanie, lupa albo pomiar,
   - **prawy przycisk** (przeciąganie) – przesuwanie obrazu, **⌘ + kółko** albo gest szczypania – powiększenie, **dwuklik** albo **R** – powrót do widoku początkowego,
   - **Jasność ▾** – gotowe ustawienia (automatyczna, z aparatu, większy/mniejszy kontrast), **I** – negatyw, **Obróć** – obrót i odbicia,
   - **Układ 1 / 2 / 4** – kilka okien obok siebie, np. T1 i T2 albo stare i nowe badanie. Shift + klik na serii wczytuje ją do kolejnego okna. Przy zaznaczonym **„Razem”** okna z tą samą płaszczyzną przewijają się razem, a przerywane **linie odniesienia** pokazują, gdzie przekrój z jednego okna przecina pozostałe,
   - lista **Płaszczyzna** pozwala obejrzeć rekonstrukcję w innej płaszczyźnie (najlepiej działa dla serii 3D z cienkimi warstwami).
4. **Pomiary.** Narzędzie **Odległość** (M): przeciągnij od punktu do punktu, a orientacyjny wynik pojawi się w milimetrach (dokładność zależy od wielkości piksela i grubości warstwy). **Strzałka** (T) służy do opisania miejsca na obrazie. Kliknięty pomiar zostaje zaznaczony: można go przesuwać za środek, zmieniać końce za uchwyty i usunąć klawiszem **Delete** albo z menu pod prawym przyciskiem. **⌘Z / ⇧⌘Z** cofa i ponawia, a **Edycja → Usuń wszystkie pomiary** czyści wszystko. Kursor myszy zmienia się zależnie od narzędzia i tego, co jest pod nim.
5. **Zapisywanie i eksport** (menu **Plik**): bieżący obraz (PNG/JPEG), widok razem z opisami i pomiarami, kopia widoku do schowka, cała seria jako osobne obrazy, animacja GIF, mozaika miniatur i tagi DICOM do pliku tekstowego.
6. **Ustawienia** (⌘,): motyw jasny/ciemny, wielkość interfejsu i opisów, kolor pomiarów, co pokazywać na obrazie, kierunek i czułość kółka, prędkość odtwarzania, format eksportu, pamięć podręczna oraz ukrywanie danych osobowych pacjenta na ekranie (np. przy pokazywaniu komuś zrzutu).

### Tryb profesjonalny

Przełącznik **Tryb PRO** na pasku (albo w ustawieniach) odsłania narzędzia dla osób, które znają się na obrazowaniu: pomiar kąta, owalny i prostokątny obszar ROI ze statystykami (średnia, odchylenie, min/max, pole), sondę wartości piksela, wskaźnik 3D pokazujący ten sam punkt we wszystkich oknach, ręczne wpisanie szerokości i poziomu okna (W/L), mapy kolorów, ustawienia okien dla tomografii, panel z pełnymi parametrami serii, wszystkimi tagami DICOM i histogramem oraz współrzędne pacjenta (LPS) pod kursorem.

### Dlaczego odtwarzanie i otwarcie serii zaczyna się od środka?

Po kliknięciu serii program pokazuje środkowy przekrój, bo zwykle tam jest najważniejszy fragment badanego narządu. Odtwarzanie rusza jednak od pierwszego obrazu, jeśli serii jeszcze nie przewijałeś. Ustawienia → „Po otwarciu serii pokaż: pierwszy obraz” zmienia też widok początkowy.

### Karta graficzna

Program korzysta ze zintegrowanej karty graficznej, która w zupełności wystarcza do oglądania zdjęć. Na Macu z dwiema kartami nie włącza dedykowanej, więc komputer się nie grzeje. Na Windowsie można to zmienić w **Ustawienia → System → Ekran → Grafika**, a na Linuksie uruchamiając program przez `prime-run` lub z `DRI_PRIME=1`.

## Podłączenie do Claude (MCP)

Serwer MCP pozwala Claude samodzielnie: wypisać serie badania z parametrami, obejrzeć wybrany przekrój, zobaczyć całą serię jako mozaikę miniatur, zrobić rekonstrukcję w innej płaszczyźnie, orientacyjnie zmierzyć odległość, policzyć statystyki jasności w obszarze i odczytać nagłówki DICOM.

Poniższe przykłady zakładają, że program jest w folderze Programy, a badania w folderze `Pobrane/MRI`. Zamiast `TWOJA_NAZWA` wpisz nazwę swojego konta na Macu (widać ją w Finderze jako nazwę folderu domowego).

### Claude Desktop

Otwórz Claude Desktop → **Ustawienia → Developer → Edit Config** i wpisz (albo dopisz do istniejącej sekcji `mcpServers`):

```json
{
  "mcpServers": {
    "mri": {
      "command": "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer",
      "args": ["mcp", "/Users/TWOJA_NAZWA/Downloads/MRI"]
    }
  }
}
```

Zapisz plik i uruchom Claude Desktop ponownie. Prościej: w programie wybierz **Pomoc → Podłącz do Claude**, a program sam dopisze tę konfigurację (z kopią zapasową dotychczasowej) albo pokaże gotowe polecenie dla Claude Code. Potem możesz napisać np. „Pokaż mi listę serii z badania MRI i opisz, co widać na serii Sag T2”.

### Claude Code

```
claude mcp add --scope user mri -- "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer" mcp /Users/TWOJA_NAZWA/Downloads/MRI
```

### Prywatność

Obrazy, które Claude ogląda przez MCP, są wysyłane do Anthropic tak jak każda inna treść rozmowy. Imię i nazwisko, PESEL/ID pacjenta, data urodzenia, nazwa placówki i nazwiska lekarzy są domyślnie **ukrywane** przed Claude (widzi tylko płeć i wiek). Jeśli chcesz je przekazać, dodaj `--show-patient` do argumentów serwera albo zaznacz tę opcję w oknie **Podłącz do Claude**. W samym programie, który działa tylko na Twoim komputerze, dane pacjenta są widoczne, chyba że wyłączysz je w ustawieniach.

## Wiersz poleceń

```
mri-viewer [FOLDER]                        okno programu
mri-viewer list FOLDER                     lista badań i serii
mri-viewer mcp [FOLDER] [--show-patient]   serwer MCP (stdio)
```

## Dla programistów

Program jest napisany w Ruście: okno w [egui/eframe](https://github.com/emilk/egui), odczyt DICOM w [dicom-rs](https://github.com/Enet4/dicom-rs) (z dekoderami JPEG, RLE i JPEG 2000 przez OpenJPEG; JPEG-LS nie jest jeszcze obsługiwany), serwer MCP w [rmcp](https://github.com/modelcontextprotocol/rust-sdk).

```
cargo build --release
./target/release/mri-viewer
```

Na Linuksie do działania okna potrzebny jest pakiet `libxkbcommon-x11-0`. Każda zmiana na gałęzi `main` automatycznie buduje paczki dla macOS (uniwersalna: Apple Silicon i Intel), Windows i Linuksa i publikuje je jako wydanie „Najnowsza wersja”.

## Autor i licencja

Autor: Adrian Kwiatkowski ([adriankwiatkowski.eu](https://adriankwiatkowski.eu), kontakt@adriankwiatkowski.eu).

Program jest udostępniany na zasadach licencji [MIT](LICENSE-MIT) lub [Apache 2.0](LICENSE-APACHE), do wyboru. Licencje bibliotek, z których korzysta (w tym OpenJPEG na licencji BSD-2-Clause), są zebrane w pliku [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md), generowanym skryptem `scripts/third_party_licenses.py`. Pliki licencji są też dołączone do każdej paczki do pobrania.

Wnosząc zmiany do repozytorium, zgadzasz się na udostępnienie ich na tych samych dwóch licencjach.

DICOM® jest zastrzeżonym znakiem towarowym National Electrical Manufacturers Association (NEMA). Program nie jest powiązany z NEMA ani przez nią zatwierdzany.
