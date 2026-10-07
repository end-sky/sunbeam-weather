# Sunbeam Weather

A Linux desktop weather app inspired by the Google-like search layout of `end-sky/weather-metasearch` and the clean, card-based visualization approach of Breezy Weather.

The project is deliberately split into:

- **Rust + egui/eframe** for the desktop interface.
- **Go** for the network/metasearch backend.

The default backend uses public endpoints that do not require an account or API key:

- Open-Meteo for geocoding and the primary forecast.
- wttr.in as a second independent public source.
- MET Norway Locationforecast as a third source.

Open-Meteo explicitly documents API-key-free, no-login access for non-commercial use. wttr.in exposes a JSON API (`format=j1`). MET Norway provides a global Locationforecast API, but it requires clients to identify themselves with a meaningful `User-Agent`; Sunbeam therefore exposes a local Settings field for a site or contact address. The app keeps that value locally and uses it only as the MET Norway User-Agent.

## UI goals

Sunbeam is intentionally search-first. The large search field works from anywhere, clicking the **Sun** in the top-right switches to a **Moon** and dark mode, and clicking the wordmark returns to the home/search view. Results show the current condition, seven-day forecast, next-hours temperature chart, details, and a visible source-agreement section.

## Build on Linux

Prerequisites:

- Rust/Cargo (stable, edition 2024 capable)
- Go 1.23+
- Native dependencies needed by eframe on your distribution (X11/Wayland, XKB, OpenSSL, etc.)

The current eframe release is documented for native Linux and uses wgpu by default. If your distribution is missing X11 development headers, install the equivalents of `libxcb-*`, `libxkbcommon`, and OpenSSL development packages.

Build:

```bash
make build
./sunbeam-weather
```

Or:

```bash
./run.sh
```

Install system-wide:

```bash
sudo make install
```

To use a custom backend executable:

```bash
SUNBEAM_WEATHER_BACKEND=/path/to/weather-scraper ./sunbeam-weather
```

## Privacy and rate limits

No account, API key, telemetry, or analytics are built into the app. Search requests go to Open-Meteo Geocoding. Weather requests are sent directly by the Go process to the three public providers. A five-minute local weather cache reduces repeated requests.

The app should be treated as a small metasearch client, not as a guaranteed-authoritative forecast. The UI makes source failures visible and shows the current-temperature spread between successful providers instead of silently pretending there is one perfect source.

## Attribution

The project was **inspired by and shaped around ideas demonstrated by**:

- `end-sky/weather-metasearch` — GPL-3.0, especially its provider-oriented architecture and search-first presentation.
- Breezy Weather — free/open-source Android weather application using Material 3-style visualizations.

Sunbeam does not copy consumer weather websites or require access to private APIs.


## 0.1.1

This release updates the eframe/egui integration for eframe 0.36.x. The application implements `eframe::App::ui(&mut Ui, &mut Frame)` and uses egui's theme-specific style API (`style_mut_of` / `set_theme`).

## 0.1.3

This release fixes three desktop UI issues: the main forecast is now inside a vertical scroll area, the search field keeps a stable text color while hovered, and weather/UI symbols no longer depend on emoji-capable fonts. Weather conditions are represented by semantic icon codes and rendered as vector shapes by egui.

## NixOS runtime libraries

When building with `nix develop`, the development shell supplies the native Linux
GUI libraries (including `libxkbcommon-x11.so`) through `LD_LIBRARY_PATH`. The
Makefile also embeds that path into the copied release executable when `patchelf`
is available inside the Nix shell, so `./sunbeam-weather` continues to work after
leaving the shell.
