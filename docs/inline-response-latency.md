# Inline response latency

Conversation configuration now accepts nullable `thinkingEnabled`. An absent value preserves provider defaults. Claude applies an explicit value through the process-scoped `--settings` JSON `alwaysThinkingEnabled`; Codex and Claurst report the field unsupported. The setting applies when launching a provider session, so clients change the immutable run selection when changing it after a completed turn.

SQLite schema v25 adds a nullable column to immutable configuration revisions. Existing rows remain unset. The native inline editor uses false for its quick response mode, true for explicit higher efforts, and null for the model default. Ordinary chat does not acquire the override. Models without advertised effort levels still support the separate quick response preference.

A direct Haiku 4.5 probe with the real stream-JSON launch contract measured first text at 4.735 seconds cold and 1.219 seconds reused with thinking off, without thinking deltas. Baseline values were 10.437 and 3.969 seconds. These individual samples are not guarantees or native editor timings.

Validation: nine Claude launch tests, four configuration type tests, three configuration protocol tests, three daemon configuration tests, and eleven schema tests passed on Windows. The app verifies remote false/true/null round trips and inline effort selection. Windows cancellation and MCP scoping remain part of the same development patch.

This work is a local source build. Publishing and signing a matching runtime archive and updating the app bundle remain required for a Store release.
