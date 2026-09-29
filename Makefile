# Tauri CLI resolves the project from crates/spaces_labels_app.
APP_DIR := crates/spaces_labels_app
APP := target/release/bundle/macos/Spaces Labels.app

.PHONY: run debug build install probe check icons clean models models-vl4b

run:            ## debug binary with latency/placement diagnostics on stderr
	SPACES_LABELS_DEBUG=1 cargo run -p spaces-labels-app

build:          ## release .app in target/release/bundle/macos
	cd $(APP_DIR) && cargo tauri build

install: build  ## replace /Applications/Spaces Labels.app and launch it
	-pkill -x spaces-labels-app
	@# LaunchServices refuses to open an app that is still shutting down (-600).
	@while pgrep -x spaces-labels-app >/dev/null; do sleep 0.2; done
	rm -rf "/Applications/Spaces Labels.app"
	cp -R "$(APP)" /Applications/
	open "/Applications/Spaces Labels.app"

probe:          ## print Spaces, apps per Space, and call costs
	cargo run --release -p spaces-sys --bin spaces_probe

models:         ## download the default local models (see docs/MODELS.md)
	./scripts/download-models.sh default

models-vl4b:    ## optional: the sharper, slower 4B vision model
	./scripts/download-models.sh vl4b

check:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all -- --check

icons:          ## regenerate icons from the SVG sources
	cd $(APP_DIR) && cargo tauri icon icons-src/app-icon.svg -o icons && rm -rf icons/android icons/ios icons/Square*.png icons/StoreLogo.png icons/icon.ico
	cd $(APP_DIR) && cargo tauri icon icons-src/tray.svg -o /tmp/spaces-labels-tray -p 44 && cp /tmp/spaces-labels-tray/44x44.png icons/tray.png

clean:
	cargo clean
