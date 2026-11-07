# Changelog

A new version is published automatically on GitHub when the `version` in `Cargo.toml` changes on the `main` branch. The release notes are taken from the matching section below.

MRI Viewer is an informational and educational image viewer, not a medical device. It is not intended for diagnosis, and its measurements are approximate.

## Unreleased

**Claude (MCP)**

- `view_slice`, `view_montage` and `view_reformat` can save the rendered PNG to a file (`save_path`, absolute or starting with `~/`). With `return_image: false` the image is only saved and not sent back, which makes exporting many images cheap. No separate script is needed any more.

## 0.2.0

First public release.

**Viewing**

- Open a folder copied from an imaging CD (no DICOMDIR needed) and browse studies and series with thumbnails. JPEG, RLE and JPEG 2000 compressed images are supported.
- 1, 2 or 4 views side by side, with linked scrolling, reference lines and a 3D cursor (only between series in the same frame of reference).
- Reconstruction in other planes (MPR), brightness/contrast presets, negative, rotation and flips, and playback (loop or back and forth).
- Multi-echo, diffusion and dynamic series are split into separate stacks.

**Measurements**

- Distance and arrow notes, which you can select, move, edit and delete, with undo and redo.
- PRO mode adds angles, elliptical and rectangular ROIs with statistics, a pixel probe, manual window/level, colour maps, CT presets, a DICOM tag panel and a histogram.

**Interface**

- Native macOS menu, and the same menu inside the window on Windows and Linux.
- Settings for theme, interface and text size, font, colours, overlays, mouse wheel, playback and export.
- The interface is in English, French, German, Italian, Polish, Portuguese, Spanish and Ukrainian. It starts in the system language and can be changed in the settings.
- The toolbar always fits on one line and folds tools into menus in narrow windows.
- On Macs with two graphics cards, the app uses the integrated one.

**Export**

- Current image (PNG/JPEG), view with labels and measurements, copy to clipboard, whole series, GIF animation, thumbnail montage and DICOM tags as text.

**Claude (MCP)**

- Built-in MCP server (`mri-viewer mcp FOLDER`) with tools to list series, view slices, montages and reconstructions, measure distances, compute region statistics and read DICOM tags.
- **Help → Connect to Claude** adds the server to Claude Desktop or shows the command for Claude Code.

**Privacy**

- By default the patient's name, ID, date of birth, institution and physicians are hidden from Claude, together with folder paths and series that may have patient data burned into the pixels. `--show-patient` turns this off.
- Saved and copied views leave out the patient's name by default.

**License**

- MIT or Apache-2.0, at your option. Third-party licenses are in `THIRD_PARTY_LICENSES.md`, which is included in every package.

## 0.1.0

First test version (not published as a release): a simple viewer with an MCP server.
