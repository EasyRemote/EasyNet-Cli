# EasyNet Menu Bar

macOS accessory app for local EasyNet operator affordances:

- menu bar icon that reflects whether `easynet-daemon` is running;
- global shortcut `Control + Option + V` to summon EasyNet clipboard history;
- Windows-style clipboard history panel backed by `~/.easynet/context/clipboard.jsonl`;
- double-click/Return promotes a history entry to the macOS pasteboard so the next paste uses it.

Build and run locally:

```sh
plugins/desktop-menubar/scripts/build-macos.sh
open target/macos/EasyNetMenuBar.app
```

Install as a login LaunchAgent:

```sh
plugins/desktop-menubar/scripts/install-macos.sh
```

The installed app lives at `~/.easynet/apps/EasyNetMenuBar.app`. The LaunchAgent lives at `~/Library/LaunchAgents/tech.silan.easynet.menubar.plist`.

The menu bar icon is drawn as an 18 pt AppKit template glyph at runtime. macOS owns the final foreground color for light and dark menu bars, and the status item uses the standard square menu-bar width instead of a text label.
