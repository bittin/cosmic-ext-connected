# Justfile for cosmic-ext-connected
# Install just: cargo install just

# Applet metadata
name := 'cosmic-ext-connected'
export APPID := 'io.github.nwxnw.cosmic-ext-connected'

# Installation paths (overridable via env vars for Flatpak builds)
rootdir := ''
prefix := '/usr'
base-dir := absolute_path(clean(rootdir / prefix))
export INSTALL_DIR := base-dir / 'share'

bin_dir := env_var_or_default("BIN_DIR", base-dir / 'bin')
app_dir := env_var_or_default("APP_DIR", INSTALL_DIR / 'applications')
metainfo_dir := env_var_or_default("METAINFO_DIR", INSTALL_DIR / 'metainfo')
icon_dir := env_var_or_default("ICON_DIR", INSTALL_DIR / 'icons' / 'hicolor' / 'scalable' / 'apps')

# Default recipe - show available commands
default:
    @just --list

# Build debug version
build *args:
    cargo build {{args}}

# Build release version
build-release *args:
    cargo build --release {{args}}

# Run the applet (for testing)
run:
    cargo run -p {{name}}

# Run in standalone window mode (for development)
run-standalone:
    cargo run -p {{name}} -- --standalone

# Run standalone with debug logging
run-debug:
    RUST_LOG=cosmic_ext_connected=debug cargo run -p {{name}} -- --standalone

# Install pre-built applet to system (requires sudo)
# Usage: cargo build --release && sudo just install
install:
    install -Dm0755 target/release/{{name}} {{bin_dir}}/{{name}}
    install -Dm0755 data/{{APPID}}.sh {{bin_dir}}/{{name}}.sh
    install -Dm0644 data/{{APPID}}.desktop {{app_dir}}/{{APPID}}.desktop
    install -Dm0644 data/{{APPID}}.metainfo.xml {{metainfo_dir}}/{{APPID}}.metainfo.xml
    install -Dm0644 data/icons/hicolor/scalable/apps/{{APPID}}.svg {{icon_dir}}/{{APPID}}.svg
    install -Dm0644 data/icons/hicolor/scalable/apps/{{APPID}}-symbolic.svg {{icon_dir}}/{{APPID}}-symbolic.svg
    install -Dm0644 data/icons/hicolor/scalable/apps/{{APPID}}-disconnected-symbolic.svg {{icon_dir}}/{{APPID}}-disconnected-symbolic.svg
    @echo "Installed {{name}} to {{bin_dir}}"
    @echo "Installed {{APPID}}.desktop to {{app_dir}}"
    @echo ""
    @echo "To add the applet to your panel:"
    @echo "  1. Open Settings > Desktop > Panel"
    @echo "  2. Click 'Add Widget' and find 'Connected'"
    @echo ""
    @echo "To reload after changes: killall cosmic-panel"

# Uninstall the applet from the system (requires sudo)
uninstall:
    rm -f {{bin_dir}}/{{name}}
    rm -f {{bin_dir}}/{{name}}.sh
    rm -f {{app_dir}}/{{APPID}}.desktop
    rm -f {{metainfo_dir}}/{{APPID}}.metainfo.xml
    rm -f {{icon_dir}}/{{APPID}}.svg
    rm -f {{icon_dir}}/{{APPID}}-symbolic.svg
    rm -f {{icon_dir}}/{{APPID}}-disconnected-symbolic.svg
    @echo "Uninstalled {{name}}"
    @echo "Restart cosmic-panel to remove from panel: killall cosmic-panel"

# Run tests
test:
    cargo test

# Run tests with output
test-verbose:
    cargo test -- --nocapture

# Check code formatting
fmt-check:
    cargo fmt --check

# Format code
fmt:
    cargo fmt

# Run clippy lints
clippy:
    cargo clippy -- -D warnings

# Run all checks (format, clippy, test)
check: fmt-check clippy test

# Clean build artifacts
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Check if kdeconnectd is running
check-daemon:
    @dbus-send --session --print-reply \
        --dest=org.kde.kdeconnect.daemon \
        /modules/kdeconnect \
        org.kde.kdeconnect.daemon.devices 2>/dev/null \
        && echo "✓ KDE Connect daemon is running" \
        || echo "✗ KDE Connect daemon is NOT running"

# List connected devices via D-Bus
list-devices:
    @dbus-send --session --print-reply \
        --dest=org.kde.kdeconnect.daemon \
        /modules/kdeconnect \
        org.kde.kdeconnect.daemon.devices

# Introspect KDE Connect D-Bus interface
introspect:
    @dbus-send --session --print-reply \
        --dest=org.kde.kdeconnect.daemon \
        /modules/kdeconnect \
        org.freedesktop.DBus.Introspectable.Introspect
