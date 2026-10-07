APP := sunbeam-weather
BACKEND := weather-scraper
PREFIX ?= /usr/local
BINDIR ?= $(PREFIX)/bin
DATADIR ?= $(PREFIX)/share

.PHONY: all build backend run install clean test

all: build

backend:
	go build -trimpath -ldflags='-s -w' -o $(BACKEND) ./cmd/weather-scraper

build: backend
	cargo build --release
	cp target/release/$(APP) ./$(APP)
	@if command -v patchelf >/dev/null 2>&1 && [ -n "$${NIX_STORE:-}" ]; then \
		patchelf --set-rpath "$${LD_LIBRARY_PATH:-}" ./$(APP); \
		echo "Sunbeam Weather: embedded Nix runtime library path"; \
	fi

run: backend
	cargo run --release

test:
	go test ./...

install: build
	install -Dm755 $(APP) $(BINDIR)/$(APP)
	install -Dm755 $(BACKEND) $(BINDIR)/$(BACKEND)
	install -Dm644 assets/sunbeam-weather.svg $(DATADIR)/icons/hicolor/scalable/apps/sunbeam-weather.svg
	install -Dm644 data/org.sunbeam_weather.desktop $(DATADIR)/applications/org.sunbeam_weather.desktop
	install -Dm644 data/org.sunbeam_weather.appdata.xml $(DATADIR)/metainfo/org.sunbeam_weather.appdata.xml

clean:
	rm -f $(APP) $(BACKEND)
	cargo clean
