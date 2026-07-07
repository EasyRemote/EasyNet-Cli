// EasyNet macOS menu bar companion
// =================================
//
// File: plugins/desktop-menubar/companion/macos/EasyNetMenuBar/Sources/EasyNetMenuBar/main.swift
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
import Darwin

private let appName = "EasyNet"
private let companionPackageId = "easynet.desktop.menubar"
private let companionPackageVersion = "0.1.0"
private let hotKeySignature = fourCharCode("ENCB")
private let hotKeyIdValue: UInt32 = 1
private let statusIconPointSize: CGFloat = 18

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

struct DaemonRuntimeStatus {
    let running: Bool
    let runtimeStatus: String
    let controlAccepting: Bool
    let invocationAccepting: Bool

    static let stopped = DaemonRuntimeStatus(
        running: false,
        runtimeStatus: "stopped",
        controlAccepting: false,
        invocationAccepting: false
    )

    static func fromDiscovery(controlAdvertised: Bool, invocationAdvertised: Bool) -> DaemonRuntimeStatus {
        DaemonRuntimeStatus(
            running: true,
            runtimeStatus: "running",
            controlAccepting: controlAdvertised,
            invocationAccepting: invocationAdvertised
        )
    }
}

final class DaemonStatusProbe {
    private let controlURL: URL

    init(homeURL: URL = FileManager.default.homeDirectoryForCurrentUser) {
        controlURL = homeURL
            .appendingPathComponent(".easynet", isDirectory: true)
            .appendingPathComponent("control.json", isDirectory: false)
    }

    func read() -> DaemonRuntimeStatus {
        guard let data = try? Data(contentsOf: controlURL),
              let raw = try? JSONSerialization.jsonObject(with: data),
              let object = raw as? [String: Any],
              let pid = object["pid"] as? Int,
              processIsAlive(pid)
        else {
            return .stopped
        }

        let controlAdvertised = nonEmptyString(object["socket_path"]) || nonEmptyString(object["pipe_name"])
        let invocationAdvertised = nonEmptyString(object["invocation_endpoint"])
        return .fromDiscovery(
            controlAdvertised: controlAdvertised,
            invocationAdvertised: invocationAdvertised
        )
    }

    private func nonEmptyString(_ value: Any?) -> Bool {
        guard let text = value as? String else {
            return false
        }
        return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private func processIsAlive(_ pid: Int) -> Bool {
        if kill(pid_t(pid), 0) == 0 {
            return true
        }
        return errno == EPERM
    }
}

final class CompanionHeartbeatWriter {
    private let statusURL: URL

    init(homeURL: URL = FileManager.default.homeDirectoryForCurrentUser) {
        statusURL = homeURL
            .appendingPathComponent(".easynet/companions", isDirectory: true)
            .appendingPathComponent(companionPackageId, isDirectory: true)
            .appendingPathComponent("status.json", isDirectory: false)
    }

    func write(daemon: DaemonRuntimeStatus) {
        let now = UInt64(Date().timeIntervalSince1970 * 1000)
        let payload: [String: Any] = [
            "schema_version": "1",
            "package_id": companionPackageId,
            "package_version": companionPackageVersion,
            "app": "EasyNetMenuBar",
            "pid": Int(ProcessInfo.processInfo.processIdentifier),
            "started_at_unix_ms": CompanionProcessStart.startedAtUnixMs,
            "last_seen_unix_ms": now,
            "daemon": [
                "runtime_status": daemon.runtimeStatus,
                "control_accepting": daemon.controlAccepting,
                "invocation_accepting": daemon.invocationAccepting,
            ],
        ]
        do {
            try FileManager.default.createDirectory(
                at: statusURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            let data = try JSONSerialization.data(withJSONObject: payload, options: [.prettyPrinted, .sortedKeys])
            let tempURL = statusURL.deletingLastPathComponent()
                .appendingPathComponent(".\(statusURL.lastPathComponent).tmp", isDirectory: false)
            try data.write(to: tempURL, options: [.atomic])
            if FileManager.default.fileExists(atPath: statusURL.path) {
                try FileManager.default.removeItem(at: statusURL)
            }
            try FileManager.default.moveItem(at: tempURL, to: statusURL)
        } catch {
            // Heartbeat failure must not break the local UI process.
        }
    }

    func remove() {
        try? FileManager.default.removeItem(at: statusURL)
    }
}

enum CompanionProcessStart {
    static let startedAtUnixMs = UInt64(Date().timeIntervalSince1970 * 1000)
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
    private let heartbeat = CompanionHeartbeatWriter()
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
        heartbeat.remove()
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
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        statusItem = item

        if let button = item.button {
            button.image = statusImage()
            button.imagePosition = .imageOnly
            button.imageScaling = .scaleProportionallyDown
            button.alignment = .center
            button.setAccessibilityLabel(appName)
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
        let status = statusProbe.read()
        let running = status.running
        heartbeat.write(daemon: status)
        daemonStatusItem?.title = running ? "Daemon: running" : "Daemon: stopped"
        statusItem?.button?.toolTip = running
            ? "EasyNet is running in the background"
            : "EasyNet daemon is not running"
        statusItem?.button?.alphaValue = running ? 1.0 : 0.38
    }

    private func statusImage() -> NSImage {
        let size = NSSize(width: statusIconPointSize, height: statusIconPointSize)
        let image = NSImage(size: size, flipped: false) { rect in
            Self.drawEasyNetStatusGlyph(in: rect.insetBy(dx: 1.2, dy: 1.4))
            return true
        }
        image.isTemplate = true
        image.size = size
        image.accessibilityDescription = appName
        return image
    }

    private static func drawEasyNetStatusGlyph(in rect: NSRect) {
        NSColor.black.setStroke()
        NSColor.black.setFill()

        let left = rect.minX + 3.1
        let hubX = rect.minX + 5.5
        let topY = rect.maxY - 3.2
        let midY = rect.midY
        let botY = rect.minY + 3.2
        let topEnd = rect.maxX - 3.9
        let midEnd = rect.maxX - 2.2
        let botEnd = rect.maxX - 3.9

        let circuits = NSBezierPath()
        circuits.lineWidth = 1.85
        circuits.lineCapStyle = .round
        circuits.lineJoinStyle = .round
        circuits.move(to: NSPoint(x: left, y: topY))
        circuits.line(to: NSPoint(x: hubX, y: topY))
        circuits.line(to: NSPoint(x: hubX + 2.4, y: midY))
        circuits.line(to: NSPoint(x: hubX, y: botY))
        circuits.line(to: NSPoint(x: left, y: botY))
        circuits.move(to: NSPoint(x: hubX, y: topY))
        circuits.curve(
            to: NSPoint(x: topEnd, y: topY),
            controlPoint1: NSPoint(x: hubX + 3.0, y: topY + 1.2),
            controlPoint2: NSPoint(x: topEnd - 2.1, y: topY + 1.0)
        )
        circuits.move(to: NSPoint(x: hubX + 2.4, y: midY))
        circuits.line(to: NSPoint(x: midEnd, y: midY))
        circuits.move(to: NSPoint(x: hubX, y: botY))
        circuits.curve(
            to: NSPoint(x: botEnd, y: botY),
            controlPoint1: NSPoint(x: hubX + 3.0, y: botY - 1.2),
            controlPoint2: NSPoint(x: botEnd - 2.1, y: botY - 1.0)
        )
        circuits.stroke()

        let nodeRadius: CGFloat = 1.45
        [
            NSPoint(x: topEnd, y: topY),
            NSPoint(x: midEnd, y: midY),
            NSPoint(x: botEnd, y: botY),
        ].forEach { point in
            NSBezierPath(
                ovalIn: NSRect(
                    x: point.x - nodeRadius,
                    y: point.y - nodeRadius,
                    width: nodeRadius * 2,
                    height: nodeRadius * 2
                )
            ).fill()
        }

        let spine = NSBezierPath()
        spine.lineWidth = 1.35
        spine.lineCapStyle = .round
        spine.move(to: NSPoint(x: left, y: botY))
        spine.curve(
            to: NSPoint(x: left, y: topY),
            controlPoint1: NSPoint(x: rect.minX + 0.1, y: rect.minY + 5.4),
            controlPoint2: NSPoint(x: rect.minX + 0.1, y: rect.maxY - 5.4)
        )
        spine.stroke()
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
