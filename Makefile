# Tauri CLI resolves the project from crates/spaces_labels_app.
APP_DIR := crates/spaces_labels_app
APP := target/release/bundle/macos/Spaces Labels.app

.PHONY: run debug build install probe check icons clean

run:            ## debug binary with latency/placement diagnostics on stderr
	SPACES_LABELS_DEBUG=1 cargo run -p spaces-labels-app

build:          ## release .app in target/release/bundle/macos
	cd $(APP_DIR) && cargo tauri build

install: build  ## replace /Applications/Spaces Labels.app and launch it
	-pkill -x spaces-labels-app
	rm -rf "/Applications/Spaces Labels.app"
	cp -R "$(APP)" /Applications/
	open "/Applications/Spaces Labels.app"

probe:          ## print Spaces, apps per Space, and call costs
	cargo run --release -p spaces-sys --bin spaces_probe

check:
	cargo clippy --workspace --all-targets -- -D warnings
	cargo fmt --all -- --check

icons:          ## regenerate icons from the SVG sources
	cd $(APP_DIR) && cargo tauri icon icons-src/app-icon.svg -o icons && rm -rf icons/android icons/ios icons/Square*.png icons/StoreLogo.png icons/icon.ico
	cd $(APP_DIR) && cargo tauri icon icons-src/tray.svg -o /tmp/spaces-labels-tray -p 44 && cp /tmp/spaces-labels-tray/44x44.png icons/tray.png

clean:
	cargo clean
