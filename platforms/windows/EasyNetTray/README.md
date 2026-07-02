# EasyNet Tray for Windows

Native Windows companion for local EasyNet operator affordances:

- taskbar tray icon showing whether `easynet-daemon.exe` is running;
- global shortcut `Control + Alt + V` to summon EasyNet clipboard history;
- Windows-clipboard-history-style popup backed by `%USERPROFILE%\.easynet\context\clipboard.jsonl`;
- double-click/Return promotes a history entry to the Windows clipboard so the next `Ctrl + V` uses it.

Build on Windows with .NET 8 SDK:

```powershell
dotnet build .\windows\EasyNetTray\EasyNetTray.csproj -c Release
```

Run:

```powershell
.\windows\EasyNetTray\bin\Release\net8.0-windows\EasyNetTray.exe
```

This app intentionally does not drive the built-in Windows `Win + V` UI. It owns its own popup and writes the selected EasyNet item to the normal Windows clipboard.
