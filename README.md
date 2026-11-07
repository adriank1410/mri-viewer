# MRI Viewer

**English** · [Polski](README.pl.md)

A simple, fast program for viewing MRI scans (and other DICOM studies, such as CT) copied from the disc you get from the imaging centre. It also has a built-in MCP server, so Claude (in Claude Desktop or Claude Code) can look at the images itself and help you understand them.

> **This is an informational and educational tool, not a medical device.** It is not intended for diagnosis, screening, monitoring of disease or treatment decisions, and its measurements are approximate. Neither the program nor Claude replaces the radiologist's report or a conversation with your doctor.

The program's interface is available in English, French, German, Italian, Polish, Portuguese, Spanish and Ukrainian. It starts in your computer's language (English if that language is not available), and you can change it in the settings.

## Installing on a Mac

1. Open the [Releases](../../releases) tab and download `MRI-Viewer-macOS.zip` from the newest version (marked **Latest**, for example `v0.2.0`).
2. Unzip it (double-click) and drag `MRI Viewer.app` to the **Applications** folder.
3. The program is not signed with an Apple certificate, so macOS blocks it the first time. The easiest fix is to open the **Terminal** app and paste:

   ```
   xattr -dr com.apple.quarantine "/Applications/MRI Viewer.app"
   ```

   Alternatively: try to open the program, then go to **System Settings → Privacy & Security** and click **Open Anyway**.

The Windows (`MRI-Viewer-Windows.zip`) and Linux (`MRI-Viewer-Linux.tar.gz`) versions are in the same place.

## Viewing a study

1. Start the program and click **Open study folder…** (or **File → Open Folder**, ⌘O). Choose the folder copied from the disc (for example `Downloads/MRI`). You can also drag the folder onto the program window. The program searches all subfolders and does not need a DICOMDIR file. Recently opened folders are listed on the start screen.
2. On the left you will see the studies (with their dates) and their **series**: sets of images taken with different settings (for example T1, T2, STIR) and in different planes – axial (slices seen from the feet), sagittal (from the side) and coronal (from the front). Click a series to display it.
3. Basic controls (on Windows and Linux use Ctrl instead of ⌘):
   - **mouse wheel**, arrow keys or the slider at the bottom – next/previous slice; **Home/End** – first and last slice; **Space** or the play button – automatic scrolling (from the first image),
   - the **left mouse button** works according to the tool selected in the toolbar: brightness/contrast, pan, zoom or measurement,
   - **right-button drag** – pan the image, **⌘ + wheel** or pinch – zoom, **double-click** or **R** – back to the initial view,
   - the **brightness ▾** menu – presets (automatic, from the scanner, more/less contrast), **I** – negative, **Rotate** – rotation and flips,
   - **Layout 1 / 2 / 4** – several views side by side, for example T1 and T2, or an old and a new study. Shift-click on a series loads it into the next view. With **Linked** turned on, views in the same plane scroll together, and dashed **reference lines** show where the slice in one view cuts the others,
   - the **Plane** list shows a reconstruction in another plane (works best for 3D series with thin slices).
4. **Measurements.** With the **Distance** tool (M), drag from one point to another and an approximate result appears in millimetres (accuracy depends on pixel size and slice thickness). The **Arrow** tool (T) marks a place on the image with a note. A clicked measurement is selected: drag its middle to move it, drag the handles to change its ends, and remove it with **Delete** or from the right-click menu. **⌘Z / ⇧⌘Z** undo and redo, and **Edit → Delete all measurements** clears everything. The mouse cursor changes depending on the tool and on what is under it.
5. **Saving and export** (**File** menu): the current image (PNG/JPEG), the view with its labels and measurements, a copy of the view to the clipboard, a whole series as separate images, a GIF animation, a thumbnail montage, and the DICOM tags as a text file.
6. **Settings** (⌘,): language, light/dark theme, interface and text size, font, measurement colour, what to show on the image, wheel direction and sensitivity, playback speed, export format, memory cache, and hiding the patient's personal data on screen (for example when showing the images to someone). Saved and copied views leave out the patient's name by default and show "Patient hidden" with sex and age instead.

### Professional mode

The **PRO** switch in the toolbar (or in the settings) shows tools for people who know medical imaging: angle measurement, elliptical and rectangular ROIs with statistics (mean, standard deviation, min/max, area), a pixel-value probe, a 3D cursor that shows the same point in all views, manual window width and level (W/L), colour maps, CT window presets, a panel with full series parameters, all DICOM tags and a histogram, and patient coordinates (LPS) under the cursor.

### Why does a series open in the middle?

When you click a series, the program shows the middle slice, because that is usually where the most important part of the examined area is. Playback, however, starts from the first image if you have not scrolled the series yet. Settings → "When a series opens, show: first image" changes the initial view as well.

### Graphics card

The program uses the integrated graphics, which is more than enough for viewing images. On a Mac with two graphics cards it does not switch on the dedicated one, so the computer does not heat up. On Windows you can change this in **Settings → System → Display → Graphics**, and on Linux by starting the program with `prime-run` or `DRI_PRIME=1`.

## Connecting to Claude (MCP)

The MCP server lets Claude: list the series of a study with their parameters, view a chosen slice, see a whole series as a thumbnail montage, reconstruct an image in another plane, measure an approximate distance, compute brightness statistics in a region, and read DICOM headers.

Claude can also save the images it views (a single slice, a montage or a reconstruction) as PNG files on your computer, for example to put them into a report or show them to a doctor. It is enough to ask, e.g. "save this image to the Desktop"; under the hood the `view_slice`, `view_montage` and `view_reformat` tools accept an optional `save_path` (an absolute path or one starting with `~/`, with `.png` added if missing; folders are created and an existing PNG is overwritten) and `return_image: false`, which only saves the file without sending the image to Claude.

The examples below assume the program is in the Applications folder and the scans are in `Downloads/MRI`. Replace `YOUR_NAME` with your Mac account name (the name of your home folder in Finder).

### Claude Desktop

Open Claude Desktop → **Settings → Developer → Edit Config** and enter (or add to an existing `mcpServers` section):

```json
{
  "mcpServers": {
    "mri": {
      "command": "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer",
      "args": ["mcp", "/Users/YOUR_NAME/Downloads/MRI"]
    }
  }
}
```

Save the file and restart Claude Desktop. An easier way: in the program choose **Help → Connect to Claude**, and the program adds this configuration for you (keeping a backup of the previous one) or shows a ready-made command for Claude Code. Then you can write, for example, "Show me the list of series in my MRI study and describe what is visible on the Sag T2 series".

### Claude Code

```
claude mcp add --scope user mri -- "/Applications/MRI Viewer.app/Contents/MacOS/mri-viewer" mcp /Users/YOUR_NAME/Downloads/MRI
```

### Privacy

Images that Claude views through MCP are sent to Anthropic like any other conversation content. The patient's name, ID number, date of birth, the institution name and the names of physicians are **hidden** from Claude by default (Claude only sees sex and age). Series that may have patient data burned into the pixels (screenshots, scanned documents, annotated images) are not shared either. If you want to share these data, add `--show-patient` to the server arguments or tick that option in the **Connect to Claude** window. In the program itself, which runs only on your computer, patient data are visible unless you hide them in the settings.

## Command line

```
mri-viewer [FOLDER]                        program window
mri-viewer list FOLDER                     list studies and series
mri-viewer mcp [FOLDER] [--show-patient]   MCP server (stdio)
```

## For developers

The program is written in Rust: the window uses [egui/eframe](https://github.com/emilk/egui), DICOM reading uses [dicom-rs](https://github.com/Enet4/dicom-rs) (with JPEG, RLE and JPEG 2000 decoders via OpenJPEG; JPEG-LS is not supported yet), and the MCP server uses [rmcp](https://github.com/modelcontextprotocol/rust-sdk).

```
cargo build --release
./target/release/mri-viewer
```

On Linux the window needs the `libxkbcommon-x11-0` package. Every change on the `main` branch automatically builds packages for macOS (universal: Apple Silicon and Intel), Windows and Linux and publishes them as the **Latest build** pre-release. When the `version` in `Cargo.toml` changes, the same build is also published as a numbered release (for example `v0.2.0`) with notes from [CHANGELOG.md](CHANGELOG.md).

### Translations

Each language is one file in [`locales/`](locales), for example `locales/de.json`, with a flat map from a key to the text. To add a language, copy `locales/en.json`, translate the values (keep the `{placeholders}` unchanged), and add one line to the `LANGS` list in [`src/i18n.rs`](src/i18n.rs). Plural forms use the key suffixes `.one`, `.few`, `.many` and `.other` (CLDR rules). `cargo test` checks that every language file has all keys and the same placeholders as English. Missing texts fall back to English.

## Author and license

Author: Adrian Kwiatkowski ([adriankwiatkowski.eu](https://adriankwiatkowski.eu), contact@adriankwiatkowski.eu).

The program is licensed under either the [MIT](LICENSE-MIT) or the [Apache 2.0](LICENSE-APACHE) license, at your option (SPDX: `MIT OR Apache-2.0`). When you use the code, you choose one of them and follow its terms. The licenses of the libraries it uses (including OpenJPEG under BSD-2-Clause) are collected in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md), generated by `scripts/third_party_licenses.py`. The license files are also included in every download package.

Unless you explicitly state otherwise, any contribution you submit for inclusion in this project is dual-licensed as above, without any additional terms or conditions.

DICOM® is a registered trademark of the National Electrical Manufacturers Association (NEMA). This program is not affiliated with or endorsed by NEMA.
