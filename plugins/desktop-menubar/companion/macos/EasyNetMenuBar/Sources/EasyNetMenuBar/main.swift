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
import QuartzCore

private let appName = "EasyNet"
private let companionPackageId = "easynet.desktop.menubar"
private let companionPackageVersion = "0.1.0"
private let hotKeySignature = fourCharCode("ENCB")
private let hotKeyIdValue: UInt32 = 1
private let statusIconPointSize = NSSize(width: 18, height: 18)

struct ClipEntry: Codable {
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

struct ClipSummary: Codable {
    let entry: ClipEntry
    var occurrenceCount: Int

    var duplicateCount: Int {
        max(0, occurrenceCount - 1)
    }
}

final class ClipboardHistoryStore {
    private let contextURL: URL

    init(homeURL: URL = FileManager.default.homeDirectoryForCurrentUser) {
        contextURL = homeURL.appendingPathComponent(".easynet/context", isDirectory: true)
    }

    func listSummaries(limit: Int = 200) -> [ClipSummary] {
        let logURL = contextURL.appendingPathComponent("clipboard.jsonl")
        let summaries = parseSummaries(from: logURL)
        return limited(summaries, limit: limit)
    }

    private func parseSummaries(from logURL: URL) -> [ClipSummary] {
        guard let content = try? String(contentsOf: logURL, encoding: .utf8) else {
            return []
        }

        let lines = content.split(whereSeparator: \.isNewline)
        var positions: [String: Int] = [:]
        var summaries: [ClipSummary] = []
        let decoder = JSONDecoder()

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

        return summaries
    }

    func loadImage(for entry: ClipEntry) -> NSImage? {
        guard entry.kind == "image", let imageFile = entry.imageFile else {
            return nil
        }
        let imageURL = contextURL
            .appendingPathComponent("clips", isDirectory: true)
            .appendingPathComponent(imageFile)
        return NSImage(contentsOf: imageURL)
    }

    func imageCacheKey(for entry: ClipEntry) -> String? {
        guard entry.kind == "image", let imageFile = entry.imageFile else {
            return nil
        }
        return imageFile
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

    private func limited(_ summaries: [ClipSummary], limit: Int) -> [ClipSummary] {
        Array(summaries.prefix(max(1, min(limit, 200))))
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

    func write(daemon: DaemonRuntimeStatus, statusItem: CompanionStatusItemSnapshot? = nil) {
        let now = UInt64(Date().timeIntervalSince1970 * 1000)
        var payload: [String: Any] = [
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
        if let statusItem {
            payload["status_item"] = statusItem.asJsonObject()
        }
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

struct CompanionStatusItemSnapshot {
    let title: String
    let hasButton: Bool
    let hasWindow: Bool
    let windowVisible: Bool
    let obscuredByNotch: Bool
    let buttonFrame: NSRect
    let windowFrame: NSRect?
    let screenFrame: NSRect?
    let auxiliaryTopLeftArea: NSRect?
    let auxiliaryTopRightArea: NSRect?

    func asJsonObject() -> [String: Any] {
        var object: [String: Any] = [
            "title": title,
            "has_button": hasButton,
            "has_window": hasWindow,
            "window_visible": windowVisible,
            "obscured_by_notch": obscuredByNotch,
            "button_frame": rectObject(buttonFrame),
        ]
        if let windowFrame {
            object["window_frame"] = rectObject(windowFrame)
        }
        if let screenFrame {
            object["screen_frame"] = rectObject(screenFrame)
        }
        if let auxiliaryTopLeftArea {
            object["auxiliary_top_left_area"] = rectObject(auxiliaryTopLeftArea)
        }
        if let auxiliaryTopRightArea {
            object["auxiliary_top_right_area"] = rectObject(auxiliaryTopRightArea)
        }
        return object
    }

    private func rectObject(_ rect: NSRect) -> [String: Double] {
        [
            "x": rect.origin.x,
            "y": rect.origin.y,
            "width": rect.size.width,
            "height": rect.size.height,
        ]
    }
}

final class ClipboardPanel: NSPanel {
    var onCancel: (() -> Void)?

    override func cancelOperation(_ sender: Any?) {
        onCancel?()
    }
}

enum ClipExpansionState {
    case collapsed
    case expanded
}

final class ClipDetailStore {
    static let shared = ClipDetailStore()
    private var expandedIds: Set<String> = []

    func state(for id: String) -> ClipExpansionState {
        expandedIds.contains(id) ? .expanded : .collapsed
    }

    func toggle(_ id: String) {
        if expandedIds.contains(id) {
            expandedIds.remove(id)
        } else {
            expandedIds.insert(id)
        }
    }

    func collapseAll() {
        expandedIds.removeAll()
    }
}

let previewMaxLines = 3
private let previewFont = NSFont.systemFont(ofSize: 13, weight: .medium)

private struct ExpandedTextHeightKey: Hashable {
    let id: String
    let width: Int
}

final class ExpandedTextHeightMeasurer {
    static let estimatedRowHeight: CGFloat = 272

    private let queue = DispatchQueue(label: "tech.silan.easynet.clipboard-text-height", qos: .userInitiated)
    private var rowHeights: [ExpandedTextHeightKey: CGFloat] = [:]
    private var pending: Set<ExpandedTextHeightKey> = []

    func cachedRowHeight(for entry: ClipEntry, width: CGFloat) -> CGFloat? {
        rowHeights[key(for: entry, width: width)]
    }

    func measure(
        entry: ClipEntry,
        width: CGFloat,
        completion: @escaping (String, CGFloat) -> Void
    ) {
        let key = key(for: entry, width: width)
        if let height = rowHeights[key] {
            completion(entry.id, height)
            return
        }
        guard !pending.contains(key) else {
            return
        }
        pending.insert(key)

        let id = entry.id
        let text = entry.text ?? entry.preview
        queue.async { [weak self] in
            let textHeight = Self.measureTextHeight(text, width: width)
            let rowHeight = max(11 + textHeight + 4 + 14 + 8, ClipCellView.collapsedHeight)
            DispatchQueue.main.async {
                guard let self else {
                    return
                }
                self.pending.remove(key)
                self.rowHeights[key] = rowHeight
                completion(id, rowHeight)
            }
        }
    }

    func reset() {
        rowHeights.removeAll(keepingCapacity: true)
        pending.removeAll(keepingCapacity: true)
    }

    private func key(for entry: ClipEntry, width: CGFloat) -> ExpandedTextHeightKey {
        ExpandedTextHeightKey(id: entry.id, width: Int(width.rounded(.down)))
    }

    private static func measureTextHeight(_ text: String, width: CGFloat) -> CGFloat {
        let attributed = NSAttributedString(
            string: text,
            attributes: [.font: previewFont, .foregroundColor: NSColor.labelColor]
        )
        let bounding = attributed.boundingRect(
            with: NSSize(width: max(1, width), height: .greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]
        )
        return max(ceil(bounding.height) + 8, 20)
    }
}

final class ClipboardImageLoader {
    private let store: ClipboardHistoryStore
    private let cache = NSCache<NSString, NSImage>()
    private let queue = DispatchQueue(label: "tech.silan.easynet.clipboard-images", qos: .userInitiated)

    init(store: ClipboardHistoryStore) {
        self.store = store
        cache.countLimit = 64
    }

    func image(for entry: ClipEntry, completion: @escaping (NSImage?) -> Void) {
        guard let key = store.imageCacheKey(for: entry) else {
            completion(nil)
            return
        }

        let cacheKey = key as NSString
        if let image = cache.object(forKey: cacheKey) {
            completion(image)
            return
        }

        queue.async { [store, cache] in
            let image = store.loadImage(for: entry)
            if let image {
                cache.setObject(image, forKey: cacheKey)
            }
            DispatchQueue.main.async {
                completion(image)
            }
        }
    }
}

final class MetaButton: NSButton {
    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }
}

final class ClipRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { true }
        set {}
    }

    override func drawSelection(in dirtyRect: NSRect) {
        guard selectionHighlightStyle != .none else {
            return
        }
        let selectionRect = bounds.insetBy(dx: 8, dy: 2)
        NSColor.selectedContentBackgroundColor.withAlphaComponent(0.28).setFill()
        NSBezierPath(roundedRect: selectionRect, xRadius: 7, yRadius: 7).fill()
    }
}

final class ClipCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("clip")
    static let collapsedHeight: CGFloat = 88
    static let collapsedPreviewHeight: CGFloat = 51
    static let expandedImageHeight: CGFloat = 220
    static let expandedImageRowHeight: CGFloat = 259

    var onToggleExpand: (() -> Void)?
    static let numberColumnWidth: CGFloat = 22

    private let numberField = NSTextField(labelWithString: "")
    private let previewField = NSTextField(labelWithString: "")
    private let metaField = MetaButton(title: "", target: nil, action: nil)
    private let detailTextField = NSTextField(labelWithString: "")
    private let detailImageView = NSImageView()
    private var summary: ClipSummary?
    private var configuredEntryId: String?
    private var isExpandedText = false
    private var visibleExpansionControl = false
    private var numberBaselineConstraint: NSLayoutConstraint!
    private var previewCollapsedHeightConstraint: NSLayoutConstraint!
    private var metaBelowPreviewConstraint: NSLayoutConstraint!
    private var textTopConstraint: NSLayoutConstraint!
    private var metaBelowTextConstraint: NSLayoutConstraint!
    private var imageTopConstraint: NSLayoutConstraint!
    private var imageHeightConstraint: NSLayoutConstraint!
    private var imageMaxWidthConstraint: NSLayoutConstraint!
    private var metaBelowImageConstraint: NSLayoutConstraint!
    private var metaBottomConstraint: NSLayoutConstraint!

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        identifier = Self.identifier

        numberField.font = NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular)
        numberField.textColor = .tertiaryLabelColor
        numberField.alignment = .right
        numberField.translatesAutoresizingMaskIntoConstraints = false

        previewField.font = previewFont
        previewField.textColor = .labelColor
        previewField.lineBreakMode = .byTruncatingTail
        previewField.maximumNumberOfLines = previewMaxLines
        previewField.cell?.wraps = true
        previewField.cell?.truncatesLastVisibleLine = true
        previewField.cell?.usesSingleLineMode = false
        previewField.cell?.isScrollable = false
        previewField.translatesAutoresizingMaskIntoConstraints = false

        metaField.translatesAutoresizingMaskIntoConstraints = false
        metaField.isBordered = false
        metaField.setButtonType(.momentaryChange)
        metaField.contentTintColor = .secondaryLabelColor
        metaField.alignment = .left
        (metaField.cell as? NSButtonCell)?.imagePosition = .noImage
        metaField.target = self
        metaField.action = #selector(toggleExpand)

        detailTextField.font = previewFont
        detailTextField.textColor = .labelColor
        detailTextField.lineBreakMode = .byWordWrapping
        detailTextField.maximumNumberOfLines = 0
        detailTextField.cell?.wraps = true
        detailTextField.cell?.usesSingleLineMode = false
        detailTextField.cell?.isScrollable = false
        detailTextField.translatesAutoresizingMaskIntoConstraints = false
        detailTextField.isHidden = true

        detailImageView.imageScaling = .scaleProportionallyUpOrDown
        detailImageView.imageAlignment = .alignTopLeft
        detailImageView.translatesAutoresizingMaskIntoConstraints = false
        detailImageView.isHidden = true
        detailImageView.wantsLayer = true
        detailImageView.layer?.cornerRadius = 6
        detailImageView.layer?.masksToBounds = true

        addSubview(numberField)
        addSubview(previewField)
        addSubview(metaField)
        addSubview(detailTextField)
        addSubview(detailImageView)

        numberBaselineConstraint = numberField.firstBaselineAnchor.constraint(equalTo: previewField.firstBaselineAnchor)
        previewCollapsedHeightConstraint = previewField.heightAnchor.constraint(lessThanOrEqualToConstant: Self.collapsedPreviewHeight)
        metaBelowPreviewConstraint = metaField.topAnchor.constraint(equalTo: previewField.bottomAnchor, constant: 4)
        textTopConstraint = detailTextField.topAnchor.constraint(equalTo: topAnchor, constant: 11)
        metaBelowTextConstraint = metaField.topAnchor.constraint(equalTo: detailTextField.bottomAnchor, constant: 4)
        imageTopConstraint = detailImageView.topAnchor.constraint(equalTo: topAnchor, constant: 11)
        imageHeightConstraint = detailImageView.heightAnchor.constraint(equalToConstant: Self.expandedImageHeight)
        imageMaxWidthConstraint = detailImageView.widthAnchor.constraint(lessThanOrEqualToConstant: 400)
        metaBelowImageConstraint = metaField.topAnchor.constraint(equalTo: detailImageView.bottomAnchor, constant: 6)
        metaBottomConstraint = metaField.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8)

        NSLayoutConstraint.activate([
            numberField.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            numberField.widthAnchor.constraint(equalToConstant: Self.numberColumnWidth),
            numberField.topAnchor.constraint(equalTo: topAnchor, constant: 11),
            numberBaselineConstraint,

            previewField.leadingAnchor.constraint(equalTo: numberField.trailingAnchor, constant: 6),
            previewField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            previewField.topAnchor.constraint(equalTo: topAnchor, constant: 11),
            previewCollapsedHeightConstraint,

            metaField.leadingAnchor.constraint(equalTo: previewField.leadingAnchor),
            metaField.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -6),
            metaBelowPreviewConstraint,
            metaBottomConstraint,

            detailTextField.leadingAnchor.constraint(equalTo: previewField.leadingAnchor),
            detailTextField.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),

            detailImageView.leadingAnchor.constraint(equalTo: previewField.leadingAnchor),
            detailImageView.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -6),
            imageMaxWidthConstraint,
        ])
        textTopConstraint.isActive = false
        metaBelowTextConstraint.isActive = false
        imageTopConstraint.isActive = false
        imageHeightConstraint.isActive = false
        metaBelowImageConstraint.isActive = false
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func configure(
        rowNumber: Int,
        with summary: ClipSummary,
        previewWidth: CGFloat,
        imageLoader: (ClipEntry, @escaping (NSImage?) -> Void) -> Void
    ) {
        self.summary = summary
        configuredEntryId = summary.entry.id
        numberField.stringValue = "\(rowNumber)"
        visibleExpansionControl = false
        let expanded = summary.entry.kind != "image"
            && ClipDetailStore.shared.state(for: summary.entry.id) == .expanded
        isExpandedText = expanded
        let isImage = summary.entry.kind == "image"

        setLayout(isImage: isImage, expandedText: expanded && !isImage)
        previewField.isHidden = isImage || expanded
        previewField.maximumNumberOfLines = expanded ? 0 : previewMaxLines
        previewField.lineBreakMode = expanded ? .byWordWrapping : .byTruncatingTail
        previewField.stringValue = isImage ? "" : (summary.entry.text ?? summary.entry.preview)
        previewField.preferredMaxLayoutWidth = previewWidth
        detailTextField.stringValue = expanded && !isImage ? (summary.entry.text ?? summary.entry.preview) : ""
        detailTextField.preferredMaxLayoutWidth = previewWidth
        previewField.invalidateIntrinsicContentSize()
        detailTextField.invalidateIntrinsicContentSize()
        needsLayout = true

        metaField.isEnabled = false
        renderMetaTitle()

        guard isImage else {
            detailImageView.isHidden = true
            detailImageView.image = nil
            return
        }

        detailImageView.isHidden = false
        detailImageView.image = nil
        imageLoader(summary.entry) { [weak self] image in
            guard let self, self.configuredEntryId == summary.entry.id else {
                return
            }
            self.detailImageView.image = image
        }
    }

    @objc private func toggleExpand() {
        if visibleExpansionControl {
            onToggleExpand?()
        }
    }

    private func setLayout(isImage: Bool, expandedText: Bool) {
        previewField.isHidden = isImage || expandedText
        detailTextField.isHidden = !expandedText

        numberBaselineConstraint.isActive = !isImage && !expandedText
        previewCollapsedHeightConstraint.isActive = !isImage && !expandedText
        metaBelowPreviewConstraint.isActive = !isImage && !expandedText
        textTopConstraint.isActive = expandedText
        metaBelowTextConstraint.isActive = expandedText
        imageTopConstraint.isActive = isImage
        imageHeightConstraint.isActive = isImage
        metaBelowImageConstraint.isActive = isImage
    }

    override func layout() {
        super.layout()
        updateExpansionControlFromNativeLayout()
    }

    private func updateExpansionControlFromNativeLayout() {
        guard let summary, summary.entry.kind != "image" else {
            setExpansionControlVisible(false)
            return
        }
        if isExpandedText {
            setExpansionControlVisible(true)
            return
        }
        guard !previewField.bounds.isEmpty else {
            setExpansionControlVisible(false)
            return
        }
        let expansionFrame = previewField.cell?.expansionFrame(
            withFrame: previewField.bounds,
            in: previewField
        ) ?? .zero
        setExpansionControlVisible(!expansionFrame.isEmpty)
    }

    private func setExpansionControlVisible(_ visible: Bool) {
        guard visibleExpansionControl != visible || metaField.isEnabled != visible else {
            return
        }
        visibleExpansionControl = visible
        metaField.isEnabled = visible
        renderMetaTitle()
    }

    private func renderMetaTitle() {
        guard let summary else {
            return
        }
        var meta = compactTime(summary.entry.timestamp)
        if summary.duplicateCount > 0 {
            meta += "  ·  ×\(summary.occurrenceCount)"
        }
        let title = NSMutableAttributedString(
            string: meta,
            attributes: [
                .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .regular),
                .foregroundColor: NSColor.secondaryLabelColor,
            ]
        )
        if visibleExpansionControl {
            title.append(NSAttributedString(
                string: isExpandedText ? "  ·  collapse" : "  ·  expand",
                attributes: [
                    .font: NSFont.monospacedDigitSystemFont(ofSize: 11, weight: .semibold),
                    .foregroundColor: NSColor.linkColor,
                ]
            ))
        }
        metaField.attributedTitle = title
    }
}

final class ClipboardPanelController: NSWindowController, NSTableViewDataSource, NSTableViewDelegate {
    private let store: ClipboardHistoryStore
    private let imageLoader: ClipboardImageLoader
    private let textHeightMeasurer = ExpandedTextHeightMeasurer()
    private let reloadQueue = DispatchQueue(label: "tech.silan.easynet.clipboard-history.reload", qos: .userInitiated)
    private let tableView = NSTableView()
    private let statusLabel = NSTextField(labelWithString: "")
    private let emptyLabel = NSTextField(labelWithString: "No EasyNet clipboard history yet.")
    private var clips: [ClipSummary] = []
    private var reloadGeneration = 0

    init(store: ClipboardHistoryStore) {
        self.store = store
        imageLoader = ClipboardImageLoader(store: store)

        let panel = ClipboardPanel(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 440),
            styleMask: [.titled, .closable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.title = "EasyNet Clipboard"
        panel.titleVisibility = .hidden
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true

        let content = NSView()
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        panel.contentView = content

        super.init(window: panel)
        panel.onCancel = { [weak self] in
            self?.dismiss()
        }
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
            dismiss()
        } else {
            show()
        }
    }

    func show() {
        guard let window else {
            return
        }
        NSApp.activate(ignoringOtherApps: true)
        reloadAsync()
        if window.isVisible {
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(tableView)
            return
        }
        window.center()
        let target = window.frame
        window.setFrame(target.offsetBy(dx: 0, dy: -14), display: false)
        window.alphaValue = 0
        window.makeKeyAndOrderFront(nil)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0.22
            context.timingFunction = CAMediaTimingFunction(name: .easeOut)
            window.animator().alphaValue = 1
            window.animator().setFrame(target, display: true)
        }
        window.makeFirstResponder(tableView)
    }

    func preload() {
        reloadAsync(showLoadingState: false)
    }

    func dismiss() {
        guard let window, window.isVisible else {
            return
        }
        NSAnimationContext.runAnimationGroup({ context in
            context.duration = 0.16
            context.timingFunction = CAMediaTimingFunction(name: .easeIn)
            window.animator().alphaValue = 0
        }, completionHandler: {
            window.orderOut(nil)
            window.alphaValue = 1
        })
    }

    func useLatest() {
        statusLabel.stringValue = "Loading latest clipboard item..."
        reloadQueue.async { [store] in
            let latest = store.listSummaries(limit: 1).first
            DispatchQueue.main.async { [weak self] in
                guard let self else {
                    return
                }
                guard let latest else {
                    NSSound.beep()
                    return
                }
                self.apply(latest)
            }
        }
    }

    func numberOfRows(in tableView: NSTableView) -> Int {
        clips.count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        guard row >= 0, row < clips.count else {
            return nil
        }
        let cell = tableView.makeView(withIdentifier: ClipCellView.identifier, owner: self) as? ClipCellView
            ?? ClipCellView(frame: .zero)
        let entry = clips[row].entry
        cell.configure(
            rowNumber: row + 1,
            with: clips[row],
            previewWidth: previewWidth(for: tableView),
            imageLoader: { [weak self] entry, completion in
                self?.imageLoader.image(for: entry, completion: completion)
            }
        )
        cell.onToggleExpand = { [weak self] in
            self?.toggleExpansion(for: entry.id, row: row)
        }
        return cell
    }

    private func toggleExpansion(for id: String, row: Int) {
        guard row >= 0, row < clips.count else {
            return
        }
        ClipDetailStore.shared.toggle(id)
        let rowIndexes = IndexSet(integer: row)
        let columnIndexes = IndexSet(integer: 0)
        tableView.beginUpdates()
        tableView.reloadData(forRowIndexes: rowIndexes, columnIndexes: columnIndexes)
        tableView.layoutSubtreeIfNeeded()
        tableView.noteHeightOfRows(withIndexesChanged: rowIndexes)
        tableView.endUpdates()
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        ClipRowView()
    }

    private func previewWidth(for tableView: NSTableView) -> CGFloat {
        let columnWidth = tableView.tableColumns.first?.width ?? 500
        return columnWidth - 6 - ClipCellView.numberColumnWidth - 6 - 6
    }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        guard row >= 0, row < clips.count else {
            return ClipCellView.collapsedHeight
        }
        let entry = clips[row].entry
        if entry.kind == "image" {
            return ClipCellView.expandedImageRowHeight
        }
        guard ClipDetailStore.shared.state(for: entry.id) == .expanded else {
            return ClipCellView.collapsedHeight
        }
        let width = previewWidth(for: tableView)
        if let cached = textHeightMeasurer.cachedRowHeight(for: entry, width: width) {
            return cached
        }
        textHeightMeasurer.measure(entry: entry, width: width) { [weak self] id, _ in
            self?.applyMeasuredTextHeight(for: id)
        }
        return ExpandedTextHeightMeasurer.estimatedRowHeight
    }

    private func applyMeasuredTextHeight(for id: String) {
        guard let row = clips.firstIndex(where: { $0.entry.id == id }),
              ClipDetailStore.shared.state(for: id) == .expanded
        else {
            return
        }
        tableView.beginUpdates()
        tableView.noteHeightOfRows(withIndexesChanged: IndexSet(integer: row))
        tableView.endUpdates()
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
        reloadAsync()
    }

    private func buildContent() {
        guard let contentView = window?.contentView else {
            return
        }

        let title = NSTextField(labelWithString: "EasyNet Clipboard")
        title.font = NSFont.systemFont(ofSize: 15, weight: .semibold)

        let refreshButton: NSButton
        if let icon = NSImage(systemSymbolName: "arrow.clockwise", accessibilityDescription: "Refresh") {
            refreshButton = NSButton(image: icon, target: self, action: #selector(refresh))
            refreshButton.isBordered = false
            refreshButton.contentTintColor = .secondaryLabelColor
        } else {
            refreshButton = NSButton(title: "Refresh", target: self, action: #selector(refresh))
            refreshButton.bezelStyle = .rounded
        }

        let header = NSStackView(views: [title, NSView(), refreshButton])
        header.orientation = .horizontal
        header.alignment = .centerY
        header.spacing = 8
        header.translatesAutoresizingMaskIntoConstraints = false

        tableView.headerView = nil
        tableView.rowHeight = ClipCellView.collapsedHeight
        tableView.style = .inset
        tableView.backgroundColor = .clear
        tableView.intercellSpacing = NSSize(width: 0, height: 2)
        tableView.selectionHighlightStyle = .regular
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.doubleAction = #selector(useSelected)
        tableView.columnAutoresizingStyle = .uniformColumnAutoresizingStyle

        let clipColumn = NSTableColumn(identifier: ClipCellView.identifier)
        clipColumn.width = 520
        tableView.addTableColumn(clipColumn)

        let scrollView = NSScrollView()
        scrollView.documentView = tableView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        emptyLabel.font = NSFont.systemFont(ofSize: 13)
        emptyLabel.textColor = .secondaryLabelColor
        emptyLabel.translatesAutoresizingMaskIntoConstraints = false

        let separator = NSBox()
        separator.boxType = .separator
        separator.translatesAutoresizingMaskIntoConstraints = false

        statusLabel.font = NSFont.systemFont(ofSize: 11)
        statusLabel.textColor = .secondaryLabelColor
        statusLabel.lineBreakMode = .byTruncatingTail

        let useButton = NSButton(title: "Use Selected", target: self, action: #selector(useSelected))
        useButton.keyEquivalent = "\r"
        useButton.bezelStyle = .rounded

        let footer = NSStackView(views: [statusLabel, NSView(), useButton])
        footer.orientation = .horizontal
        footer.alignment = .centerY
        footer.spacing = 8
        footer.translatesAutoresizingMaskIntoConstraints = false

        contentView.addSubview(header)
        contentView.addSubview(scrollView)
        contentView.addSubview(emptyLabel)
        contentView.addSubview(separator)
        contentView.addSubview(footer)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 12),
            header.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 44),
            header.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),

            scrollView.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 10),
            scrollView.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: separator.topAnchor, constant: -6),

            emptyLabel.centerXAnchor.constraint(equalTo: scrollView.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(equalTo: scrollView.centerYAnchor),

            separator.leadingAnchor.constraint(equalTo: contentView.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: contentView.trailingAnchor),

            footer.topAnchor.constraint(equalTo: separator.bottomAnchor, constant: 9),
            footer.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 16),
            footer.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -16),
            footer.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -10),
        ])
    }

    private func reloadAsync(showLoadingState: Bool = true) {
        reloadGeneration += 1
        let generation = reloadGeneration
        if showLoadingState && clips.isEmpty {
            emptyLabel.stringValue = "Loading clipboard history..."
            emptyLabel.isHidden = false
        }
        if showLoadingState {
            statusLabel.stringValue = "Loading clipboard history..."
        }

        reloadQueue.async { [store] in
            let summaries = store.listSummaries()
            DispatchQueue.main.async { [weak self] in
                guard let self, generation == self.reloadGeneration else {
                    return
                }
                self.applyReloadedSummaries(summaries)
            }
        }
    }

    private func applyReloadedSummaries(_ summaries: [ClipSummary]) {
        if sameSummaryProjection(clips, summaries) {
            updateFooter()
            return
        }
        clips = summaries
        textHeightMeasurer.reset()
        updateFooter()
        tableView.reloadData()
        if !clips.isEmpty && tableView.selectedRow < 0 {
            tableView.selectRowIndexes(IndexSet(integer: 0), byExtendingSelection: false)
        }
    }

    private func updateFooter() {
        emptyLabel.stringValue = "No EasyNet clipboard history yet."
        emptyLabel.isHidden = !clips.isEmpty
        statusLabel.stringValue = clips.isEmpty
            ? ""
            : "\(clips.count) unique items · double-click or ⏎ to copy"
    }

    private func sameSummaryProjection(_ lhs: [ClipSummary], _ rhs: [ClipSummary]) -> Bool {
        guard lhs.count == rhs.count else {
            return false
        }
        for (left, right) in zip(lhs, rhs) {
            if left.entry.id != right.entry.id || left.occurrenceCount != right.occurrenceCount {
                return false
            }
        }
        return true
    }

    private func apply(_ summary: ClipSummary) {
        if store.applyToPasteboard(summary.entry) {
            dismiss()
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
        panelController = ClipboardPanelController(store: store)
        installStatusItem()
        installHotKey()
        updateDaemonStatus()
        panelController?.preload()

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
            let image = statusImage()
            button.image = image
            button.alternateImage = image
            button.title = ""
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

    private func statusImage() -> NSImage {
        if let url = Bundle.main.url(forResource: "easynet-template", withExtension: "png"),
           let image = NSImage(contentsOf: url)
        {
            return whiteStatusImage(from: image)
        }
        let image = NSImage(size: statusIconPointSize, flipped: false) { rect in
            NSColor.white.setFill()
            "E".draw(
                in: rect.insetBy(dx: 3, dy: 1),
                withAttributes: [
                    .font: NSFont.monospacedSystemFont(ofSize: 13, weight: .semibold),
                    .foregroundColor: NSColor.white,
                ]
            )
            return true
        }
        image.isTemplate = false
        image.accessibilityDescription = appName
        return image
    }

    private func whiteStatusImage(from source: NSImage) -> NSImage {
        let target = NSImage(size: statusIconPointSize)
        target.lockFocus()
        defer { target.unlockFocus() }

        let rect = NSRect(origin: .zero, size: statusIconPointSize)
        NSGraphicsContext.current?.imageInterpolation = .high
        source.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
        NSColor.white.setFill()
        rect.fill(using: .sourceIn)

        target.isTemplate = false
        target.accessibilityDescription = appName
        return target
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
        heartbeat.write(daemon: status, statusItem: statusItemSnapshot())
        daemonStatusItem?.title = running ? "Daemon: running" : "Daemon: stopped"
        statusItem?.button?.toolTip = running
            ? "EasyNet is running in the background"
            : "EasyNet daemon is not running"
        statusItem?.button?.alphaValue = running ? 1.0 : 0.38
    }

    private func statusItemSnapshot() -> CompanionStatusItemSnapshot {
        guard let button = statusItem?.button else {
            return CompanionStatusItemSnapshot(
                title: "",
                hasButton: false,
                hasWindow: false,
                windowVisible: false,
                obscuredByNotch: false,
                buttonFrame: .zero,
                windowFrame: nil,
                screenFrame: nil,
                auxiliaryTopLeftArea: nil,
                auxiliaryTopRightArea: nil
            )
        }
        let window = button.window
        let notch = Self.notchObservation(for: window?.frame)
        return CompanionStatusItemSnapshot(
            title: button.title,
            hasButton: true,
            hasWindow: window != nil,
            windowVisible: window?.isVisible ?? false,
            obscuredByNotch: notch.obscured,
            buttonFrame: button.frame,
            windowFrame: window?.frame,
            screenFrame: notch.screenFrame,
            auxiliaryTopLeftArea: notch.leftArea,
            auxiliaryTopRightArea: notch.rightArea
        )
    }

    private static func notchObservation(
        for frame: NSRect?
    ) -> (obscured: Bool, screenFrame: NSRect?, leftArea: NSRect?, rightArea: NSRect?) {
        guard let frame,
              let screen = NSScreen.screens.first(where: { $0.frame.intersects(frame) })
        else {
            return (false, nil, nil, nil)
        }
        guard #available(macOS 12.0, *) else {
            return (false, screen.frame, nil, nil)
        }
        let left = screen.auxiliaryTopLeftArea ?? .zero
        let right = screen.auxiliaryTopRightArea ?? .zero
        let hasNotch = screen.safeAreaInsets.top > 0 && (!left.isEmpty || !right.isEmpty)
        let inVisibleMenuArea = left.intersects(frame) || right.intersects(frame)
        return (
            hasNotch && !inVisibleMenuArea,
            screen.frame,
            left.isEmpty ? nil : left,
            right.isEmpty ? nil : right
        )
    }

}

private let isoFractionalFormatter: ISO8601DateFormatter = {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return formatter
}()

private let isoFormatter = ISO8601DateFormatter()

private func compactTime(_ raw: String) -> String {
    guard let date = isoFractionalFormatter.date(from: raw) ?? isoFormatter.date(from: raw) else {
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
app.setActivationPolicy(.accessory)
app.delegate = delegate
app.run()
