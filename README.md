# Przeglądarka MRI

Prosty i szybki program do oglądania badań rezonansu magnetycznego (oraz innych badań w formacie DICOM, np. tomografii) skopiowanych z płyty z pracowni. Ma też wbudowany serwer MCP, dzięki któremu Claude (w aplikacji Claude Desktop lub w Claude Code) może sam przeglądać obrazy i pomagać w ich zrozumieniu.

> **To narzędzie poglądowe, nie diagnostyczne.** Ani program, ani Claude nie zastępują opisu radiologa ani rozmowy z lekarzem.

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

1. Uruchom program i kliknij **„Otwórz folder z badaniem…”**. Wskaż folder skopiowany z płyty (np. `Pobrane/MRI`). Możesz też przeciągnąć folder na okno programu. Program sam przeszuka podfoldery i nie potrzebuje pliku DICOMDIR.
2. Po lewej zobaczysz badania (z datą) i ich **serie**, czyli zestawy zdjęć wykonanych w różnych ustawieniach (np. T1, T2, STIR) i płaszczyznach: poprzecznej (plastry oglądane od stóp), strzałkowej (z boku) i czołowej (od przodu). Kliknij serię, aby ją wyświetlić.
3. Obsługa:
   - **kółko myszy** lub strzałki – kolejne przekroje; suwak na dole pokazuje numer obrazu,
   - **przeciąganie lewym przyciskiem** – jasność (w pionie) i kontrast (w poziomie),
   - **prawy przycisk** – przesuwanie obrazu, **Cmd + kółko** albo gest szczypania – powiększenie,
   - **dwuklik** albo klawisz **R** – powrót do widoku początkowego, **I** – negatyw, **spacja** – automatyczne odtwarzanie,
   - **Pomiar** – kliknij dwa punkty, aby zmierzyć odległość w milimetrach,
   - **Porównanie (2 okna)** – dwa obrazy obok siebie (np. T1 i T2 albo stare i nowe badanie); Shift + klik na serii wczytuje ją do drugiego okna,
   - lista nad obrazem pozwala obejrzeć **rekonstrukcję w innej płaszczyźnie** (najlepiej działa dla serii 3D z cienkimi warstwami),
   - **Zapisz PNG** – zapisuje bieżący obraz, np. żeby wysłać go lekarzowi albo wkleić do rozmowy z Claude.

## Podłączenie do Claude (MCP)

Serwer MCP pozwala Claude samodzielnie: wypisać serie badania z parametrami, obejrzeć wybrany przekrój, zobaczyć całą serię jako mozaikę miniatur, zrobić rekonstrukcję w innej płaszczyźnie, zmierzyć odległość, policzyć statystyki jasności w obszarze i odczytać nagłówki DICOM.

Poniższe przykłady zakładają, że program jest w folderze Programy, a badania w `/Users/adriank1410/Downloads/MRI` – w razie potrzeby zmień ścieżkę.

### Claude Desktop

Otwórz Claude Desktop → **Ustawienia → Developer → Edit Config** i wpisz (albo dopisz do istniejącej sekcji `mcpServers`):

```json
{
  "mcpServers": {
    "mri": {
      "command": "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer",
      "args": ["mcp", "/Users/adriank1410/Downloads/MRI"]
    }
  }
}
```

Zapisz plik i uruchom Claude Desktop ponownie. Potem możesz napisać np. „Pokaż mi listę serii z badania MRI i opisz, co widać na serii Sag T2”.

### Claude Code

```
claude mcp add mri -- "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer" mcp /Users/adriank1410/Downloads/MRI
```

### Prywatność

Obrazy, które Claude ogląda przez MCP, są wysyłane do Anthropic tak jak każda inna treść rozmowy. Imię i nazwisko, PESEL/ID pacjenta, data urodzenia, nazwa placówki i nazwiska lekarzy są domyślnie **ukrywane** przed Claude (widzi tylko płeć i wiek). Jeśli chcesz je przekazać, dodaj `--show-patient` do argumentów serwera.

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
