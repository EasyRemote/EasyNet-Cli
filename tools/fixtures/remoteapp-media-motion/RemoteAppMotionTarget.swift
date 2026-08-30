import AppKit

final class MotionView: NSView {
    private var phase: Double = 0
    private var frameCounter: UInt64 = 0
    private var timer: Timer?

    override var isOpaque: Bool { true }

    func start() {
        timer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
            guard let self else { return }
            phase += 0.035
            frameCounter &+= 1
            needsDisplay = true
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        let bounds = self.bounds
        NSColor(calibratedHue: phase.truncatingRemainder(dividingBy: 1),
                saturation: 0.78,
                brightness: 0.42,
                alpha: 1).setFill()
        bounds.fill()

        let radius = max(28, min(bounds.width, bounds.height) * 0.08)
        let x = bounds.midX + bounds.width * 0.36 * sin(phase * 2.1)
        let y = bounds.midY + bounds.height * 0.30 * cos(phase * 1.7)
        NSColor.white.setFill()
        NSBezierPath(ovalIn: NSRect(x: x - radius,
                                   y: y - radius,
                                   width: radius * 2,
                                   height: radius * 2)).fill()

        let label = "RemoteApp frame \(frameCounter)"
        label.draw(at: NSPoint(x: bounds.width * 0.06, y: bounds.height * 0.86),
                   withAttributes: [
                       .font: NSFont.monospacedSystemFont(ofSize: 34, weight: .semibold),
                       .foregroundColor: NSColor.white,
                   ])
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let view = MotionView(frame: NSRect(x: 0, y: 0, width: 1280, height: 720))
        let window = NSWindow(
            contentRect: view.frame,
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "RemoteApp Live Media Target"
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        view.start()
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}

let application = NSApplication.shared
let delegate = AppDelegate()
application.setActivationPolicy(.regular)
application.delegate = delegate
application.run()
