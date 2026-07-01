// EasyNet macOS menu bar companion
// =================================
//
// File: macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift
// Description: AppKit status-item companion for EasyNet daemon status
//              and EasyNet clipboard-history promotion.
//
// Protocol Responsibility:
// - Reads local EasyNet context history from ~/.easynet/context.
// - Writes only to the operator's macOS general pasteboard.
//
// Implementation Approach:
// - AppKit accessory app with NSStatusItem, NSPanel, and a Carbon
//   global hot key.
// - Clipboard list uses a newest-to-oldest JSONL scan plus Dictionary
//   lookup, avoiding timestamp sorting.
//
// Usage Contract:
// - Run as the logged-in macOS user that owns the EasyNet state dir.
// - Shortcut is Control + Option + V.
//
// Architectural Position:
// - Local UI facade. It does not own daemon lifecycle, capture, or
//   EasyNet persistence.

import AppKit
import Carbon.HIToolbox
import CryptoKit

private let appName = "EasyNet"
private let hotKeySignature = fourCharCode("ENCB")
private let hotKeyIdValue: UInt32 = 1

struct ClipEntry: Decodable {
    let id: String
    let timestamp: String
    let device: String
    let kind: String
    let text: String?
    let imageFile: String?
    let preview: String

    enum CodingKeys: String, CodingKey {
        case id
        case timestamp
        case device
        case kind
        case text
        case imageFile = "image_file"
        case preview
    }
}

struct ClipSummary {
    let entry: ClipEntry
    var occurrenceCount: Int

    var duplicateCount: Int {
        max(0, occurrenceCount - 1)
    }
}

final class ClipboardHistoryStore {
    private let decoder = JSONDecoder()
    private let contextURL: URL

    init(homeURL: URL = FileManager.default.homeDirectoryForCurrentUser) {
        contextURL = homeURL.appendingPathComponent(".easynet/context", isDirectory: true)
    }

    func listSummaries(limit: Int = 200) -> [ClipSummary] {
        let logURL = contextURL.appendingPathComponent("clipboard.jsonl")
        guard let content = try? String(contentsOf: logURL, encoding: .utf8) else {
            return []
        }

        let lines = content.split(whereSeparator: \.isNewline)
        var positions: [String: Int] = [:]
        var summaries: [ClipSummary] = []

        for line in lines.reversed() {
            guard let data = String(line).data(using: .utf8),
                  let entry = try? decoder.decode(ClipEntry.self, from: data)
            else {
                continue
            }

            let key = contentKey(for: entry)
            if let index = positions[key] {
                summaries[index].occurrenceCount += 1
            } else {
                positions[key] = summaries.count
                summaries.append(ClipSummary(entry: entry, occurrenceCount: 1))
            }
        }

        return Array(summaries.prefix(max(1, min(limit, 200))))
    }

    func applyToPasteboard(_ entry: ClipEntry) -> Bool {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()

        if entry.kind == "text", let text = entry.text {
            return pasteboard.setString(text, forType: .string)
        }

        if entry.kind == "image", let imageFile = entry.imageFile {
            let imageURL = contextURL
                .appendingPathComponent("clips", isDirectory: true)
                .appendingPathComponent(imageFile)
            guard let data = try? Data(contentsOf: imageURL) else {
                return false
            }
            pasteboard.setData(data, forType: NSPasteboard.PasteboardType("public.png"))
            if let image = NSImage(data: data) {
                pasteboard.writeObjects([image])
            }
            return true
        }

        return false
    }

    private func contentKey(for entry: ClipEntry) -> String {
        var hasher = SHA256()
        hasher.update(data: Data(entry.kind.utf8))
        hasher.update(data: Data([0]))
        if let text = entry.text {
            hasher.update(data: Data(text.utf8))
        } else if let imageFile = entry.imageFile {
            hasher.update(data: Data(imageFile.utf8))
        } else {
            hasher.update(data: Data(entry.preview.utf8))
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }
}

final class DaemonStatusProbe {
    func isRunning() -> Bool {
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/bin/pgrep")
        task.arguments = ["-x", "easynet-daemon"]
        task.standardOutput = Pipe()
        task.standardError = Pipe()
        do {
            try task.run()
            task.waitUntilExit()
            return task.terminationStatus == 0
        } catch {
            return false
        }
    }
}

final class ClipboardPanelController: NSWindowController, NSTableViewDataSource, NSTableViewDelegate {
    private let store: ClipboardHistoryStore
    private let tableView = NSTableView()
    private let statusLabel = NSTextField(labelWithString: "")
    private var clips: [ClipSummary] = []

    init(store: ClipboardHistoryStore) {
        self.store = store

        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 420),
            styleMask: [.titled, .closable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.title = "EasyNet Clipboard"
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.titlebarAppearsTransparent = true

        super.init(window: panel)
        buildContent()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func toggle() {
        guard let window else {
            return
        }
        if window.isVisible {
            window.orderOut(nil)
        } else {
            show()
        }
    }

    func show() {
        reload()
        guard let window else {
            return
        }
        NSApp.activate(ignoringOtherApps: true)
        window.center()
        window.makeKeyAndOrderFront(nil)
    }

    func useLatest() {
        reload()
        guard let latest = clips.first else {
            NSSound.beep()
            return
        }
        apply(latest)
    }

    func numberOfRows(in tableView: NSTableView) -> Int {
        clips.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row >= 0, row < clips.count else {
            return nil
        }
        let summary = clips[row]
        let identifier = tableColumn?.identifier ?? NSUserInterfaceItemIdentifier("clip")
        let cell = tableView.makeView(withIdentifier: identifier, owner: self) as? NSTableCellView
            ?? NSTableCellView()
        cell.identifier = identifier

        let textField = cell.textField ?? NSTextField(labelWithString: "")
        textField.lineBreakMode = .byTruncatingTail
        textField.maximumNumberOfLines = identifier.rawValue == "clip" ? 2 : 1
        textField.font = identifier.rawValue == "clip"
            ? NSFont.systemFont(ofSize: 13, weight: .medium)
            : NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .regular)
        textField.textColor = identifier.rawValue == "clip" ? .labelColor : .secondaryLabelColor
        textField.translatesAutoresizingMaskIntoConstraints = false

        if textField.superview == nil {
            cell.addSubview(textField)
            cell.textField = textField
            NSLayoutConstraint.activate([
                textField.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 8),
                textField.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -8),
                textField.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
        }

        switch identifier.rawValue {
        case "count":
            textField.stringValue = summary.duplicateCount > 0 ? "x\(summary.occurrenceCount)" : ""
        case "kind":
            textField.stringValue = summary.entry.kind
        case "time":
            textField.stringValue = compactTime(summary.entry.timestamp)
        default:
            textField.stringValue = summary.entry.preview
        }
        return cell
    }

    @objc private func useSelected() {
        let selected = tableView.selectedRow
        guard selected >= 0, selected < clips.count else {
            NSSound.beep()
            return
        }
        apply(clips[selected])
    }

    @objc private func refresh() {
        reload()
    }

    private func buildContent() {
        guard let contentView = window?.contentView else {
            return
        }

        let title = NSTextField(labelWithString: "EasyNet Clipboard")
        title.font = NSFont.systemFont(ofSize: 17, weight: .semibold)
        title.translatesAutoresizingMaskIntoConstraints = false

        statusLabel.font = NSFont.systemFont(ofSize: 12)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.translatesAutoresizingMaskIntoConstraints = false

        let refreshButton = NSButton(title: "Refresh", target: self, action: #selector(refresh))
        refreshButton.bezelStyle = .rounded
        refreshButton.translatesAutoresizingMaskIntoConstraints = false

        let useButton = NSButton(title: "Use Selected", target: self, action: #selector(useSelected))
        useButton.keyEquivalent = "\r"
        useButton.bezelStyle = .rounded
        useButton.translatesAutoresizingMaskIntoConstraints = false

        let header = NSStackView(views: [title, NSView(), refreshButton, useButton])
        header.orientation = .horizontal
        header.alignment = .centerY
        header.spacing = 8
        header.translatesAutoresizingMaskIntoConstraints = false

        tableView.headerView = nil
        tableView.rowHeight = 52
        tableView.intercellSpacing = NSSize(width: 0, height: 4)
        tableView.selectionHighlightStyle = .regular
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(useSelected)

        let clipColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("clip"))
        clipColumn.width = 340
        let countColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("count"))
        countColumn.width = 52
        let kindColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("kind"))
        kindColumn.width = 70
        let timeColumn = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("time"))
        timeColumn.width = 92
        [clipColumn, countColumn, kindColumn, timeColumn].forEach(tableView.addTableColumn)

        let scrollView = NSScrollView()
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        contentView.addSubview(header)
        contentView.addSubview(statusLabel)
        contentView.addSubview(scrollView)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 18),
            header.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            header.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),

            statusLabel.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 4),
            statusLabel.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            statusLabel.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),

            scrollView.topAnchor.constraint(equalTo: statusLabel.bottomAnchor, constant: 12),
            scrollView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 8),
            scrollView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -8),
            scrollView.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -8),
        ])
    }

    private func reload() {
        clips = store.listSummaries()
        statusLabel.stringValue = clips.isEmpty
            ? "No EasyNet clipboard history yet."
            : "\(clips.count) unique items. Double-click or press Return to move one to the system clipboard."
        tableView.reloadData()
        if !clips.isEmpty && tableView.selectedRow < 0 {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
    }

    private func apply(_ summary: ClipSummary) {
        if store.applyToPasteboard(summary.entry) {
            window?.orderOut(nil)
        } else {
            NSSound.beep()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private let store = ClipboardHistoryStore()
    private let statusProbe = DaemonStatusProbe()
    private var panelController: ClipboardPanelController?
    private var statusItem: NSStatusItem?
    private var statusMenu: NSMenu?
    private var daemonStatusItem: NSMenuItem?
    private var hotKeyRef: EventHotKeyRef?
    private var statusTimer: Timer?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.setActivationPolicy(.accessory)

        panelController = ClipboardPanelController(store: store)
        installStatusItem()
        installHotKey()
        updateDaemonStatus()

        statusTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
            self?.updateDaemonStatus()
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        if let hotKeyRef {
            UnregisterEventHotKey(hotKeyRef)
        }
    }

    @objc private func showClipboardHistory() {
        panelController?.show()
    }

    @objc private func toggleClipboardHistory() {
        panelController?.toggle()
    }

    @objc private func useLatestClip() {
        panelController?.useLatest()
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: 92)
        statusItem = item

        if let button = item.button {
            button.image = statusImage()
            button.title = appName
            button.imagePosition = .imageLeading
            button.font = NSFont.monospacedSystemFont(ofSize: 12, weight: .semibold)
            button.alignment = .center
        }

        let menu = NSMenu()
        daemonStatusItem = NSMenuItem(title: "Daemon: checking...", action: nil, keyEquivalent: "")
        menu.addItem(daemonStatusItem!)
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(
            title: "Show Clipboard History",
            action: #selector(showClipboardHistory),
            keyEquivalent: ""
        ))
        menu.addItem(NSMenuItem(
            title: "Use Latest Clip",
            action: #selector(useLatestClip),
            keyEquivalent: ""
        ))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Shortcut: Control + Option + V", action: nil, keyEquivalent: ""))
        menu.addItem(NSMenuItem.separator())
        menu.addItem(NSMenuItem(title: "Quit EasyNet Menu Bar", action: #selector(quit), keyEquivalent: "q"))
        statusMenu = menu
        item.menu = menu
    }

    private func installHotKey() {
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )
        InstallEventHandler(
            GetApplicationEventTarget(),
            { _, eventRef, userData in
                guard let eventRef, let userData else {
                    return noErr
                }
                var hotKeyId = EventHotKeyID()
                let status = GetEventParameter(
                    eventRef,
                    EventParamName(kEventParamDirectObject),
                    EventParamType(typeEventHotKeyID),
                    nil,
                    MemoryLayout<EventHotKeyID>.size,
                    nil,
                    &hotKeyId
                )
                guard status == noErr,
                      hotKeyId.signature == hotKeySignature,
                      hotKeyId.id == hotKeyIdValue
                else {
                    return noErr
                }
                let appDelegate = Unmanaged<AppDelegate>
                    .fromOpaque(userData)
                    .takeUnretainedValue()
                DispatchQueue.main.async {
                    appDelegate.toggleClipboardHistory()
                }
                return noErr
            },
            1,
            &eventType,
            Unmanaged.passUnretained(self).toOpaque(),
            nil
        )

        let carbonHotKeyId = EventHotKeyID(signature: hotKeySignature, id: hotKeyIdValue)
        let modifiers = UInt32(controlKey | optionKey)
        RegisterEventHotKey(
            UInt32(kVK_ANSI_V),
            modifiers,
            carbonHotKeyId,
            GetApplicationEventTarget(),
            0,
            &hotKeyRef
        )
    }

    private func updateDaemonStatus() {
        let running = statusProbe.isRunning()
        daemonStatusItem?.title = running ? "Daemon: running" : "Daemon: stopped"
        statusItem?.button?.toolTip = running
            ? "EasyNet is running in the background"
            : "EasyNet daemon is not running"
        statusItem?.button?.alphaValue = running ? 1.0 : 0.38
    }

    private func statusImage() -> NSImage? {
        let bundleImage = Bundle.main.url(forResource: "easynet-status", withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
            ?? Bundle.main.url(forResource: "easynet-template", withExtension: "png")
            .flatMap { NSImage(contentsOf: $0) }
        let fallback = NSImage(
            systemSymbolName: "bolt.horizontal.circle",
            accessibilityDescription: appName
        )
        let image = bundleImage ?? fallback
        image?.isTemplate = true
        image?.size = NSSize(width: 18, height: 18)
        return image
    }
}

private func compactTime(_ raw: String) -> String {
    let formatter = ISO8601DateFormatter()
    guard let date = formatter.date(from: raw) else {
        return raw
    }
    let out = DateFormatter()
    out.dateFormat = Calendar.current.isDateInToday(date) ? "HH:mm:ss" : "MM-dd HH:mm"
    return out.string(from: date)
}

private func fourCharCode(_ string: String) -> FourCharCode {
    var result: FourCharCode = 0
    for scalar in string.unicodeScalars.prefix(4) {
        result = (result << 8) + FourCharCode(scalar.value)
    }
    return result
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
