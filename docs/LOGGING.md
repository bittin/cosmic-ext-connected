# Logging and diagnostics

Connected emits structured tracing events directly to the user's systemd journal via the `tracing_journald` layer in `src/main.rs`.

**Live tail:**

```bash
journalctl --user SYSLOG_IDENTIFIER=cosmic-ext-connected -f
```

**Filter by level or message:**

```bash
journalctl --user SYSLOG_IDENTIFIER=cosmic-ext-connected -p warning      # WARN+
journalctl --user SYSLOG_IDENTIFIER=cosmic-ext-connected --grep "<text>"
journalctl --user SYSLOG_IDENTIFIER=cosmic-ext-connected _PID=<pid>      # one process at a time
```

The default filter directive is `cosmic_ext_connected=info` — events at INFO and above from Connected, plus ERROR-level events from any crate routed through our subscriber (so libcosmic warnings and errors also surface). Override with `RUST_LOG`:

```bash
RUST_LOG=cosmic_ext_connected=debug cargo run -p cosmic-ext-connected
```

**Why direct routing:** cosmic-panel pipes applet stdout/stderr and re-emits each line through its own tracing tree, then drops INFO under its default `warn` filter. The journald layer bypasses this, preserving each event's original level under our own `SYSLOG_IDENTIFIER`. Inside Flatpak the layer may fail to construct (sandboxed journal socket) and silently falls back to fmt-only — see `CLAUDE.md` "Flatpak Debug Logging" for the file-based alternative.

**Adding diagnostics:** use `tracing::info!`/`warn!`/`error!` macros — they route through both layers automatically. For structured fields, prefer `tracing::info!(thread_id = %tid, "loaded")` over format-string interpolation; structured fields render as separate journald entries when the layer supports them.
