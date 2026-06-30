# EasyNet Menu Bar

macOS accessory app for local EasyNet operator affordances:

- menu bar icon that reflects whether `easynet-daemon` is running;
- global shortcut `Control + Option + V` to summon EasyNet clipboard history;
- Windows-style clipboard history panel backed by `~/.easynet/context/clipboard.jsonl`;
- double-click/Return promotes a history entry to the macOS pasteboard so the next paste uses it.

Build and run locally:

```sh
scripts/build-macos-menubar.sh
open target/macos/EasyNetMenuBar.app
```

Install as a login LaunchAgent:

```sh
scripts/install-macos-menubar.sh
```

The installed app lives at `~/.easynet/apps/EasyNetMenuBar.app`. The LaunchAgent lives at `~/Library/LaunchAgents/tech.silan.easynet.menubar.plist`.

The bundled icon is copied from `../EasyNet/Frontend/public/logo.png`, resized into a status-bar asset, and loaded as an AppKit template image, so macOS renders it as a status-bar silhouette. The menu bar item also shows `EasyNet` next to the icon to keep it visible in crowded menu bars.
