#!/usr/bin/env python3
"""Real X11 multi-window sentinel for Linux RemoteApp product E2E.

One process owns two independently moving Tk windows. Each surface changes
pixels continuously and the process records X11-delivered pointer, keyboard,
and wheel events. The fixture is an observation target only; it does not mock
capture, input injection, RemoteApp abilities, or WebRTC.
"""

from __future__ import annotations

import ctypes
import hashlib
import json
import os
import pathlib
import time
import tkinter as tk
import uuid


STATE_PATH = pathlib.Path(
    os.environ.get("EASYNET_REMOTEAPP_LINUX_SENTINEL_STATE", "/tmp/remoteapp-linux-sentinel.json")
)
COMMAND_PATH = pathlib.Path(
    os.environ.get(
        "EASYNET_REMOTEAPP_LINUX_SENTINEL_COMMAND",
        str(STATE_PATH.with_suffix(STATE_PATH.suffix + ".command.json")),
    )
)
STARTED_AT_MS = int(time.time() * 1000)
OBSERVER_INSTANCE_ID = str(uuid.uuid4())
APPLICATION_CLASS = os.environ.get(
    "EASYNET_REMOTEAPP_LINUX_SENTINEL_CLASS", "EasyNetRemoteAppSentinel"
)
TITLE_PREFIX = os.environ.get(
    "EASYNET_REMOTEAPP_LINUX_SENTINEL_TITLE_PREFIX", "EasyNet RemoteApp Sentinel"
)
FIXTURE_ROLE = os.environ.get("EASYNET_REMOTEAPP_LINUX_SENTINEL_ROLE", "selected_target")
PRIMARY_GEOMETRY = os.environ.get(
    "EASYNET_REMOTEAPP_LINUX_SENTINEL_PRIMARY_GEOMETRY", "480x320+80+100"
)
SECONDARY_GEOMETRY = os.environ.get(
    "EASYNET_REMOTEAPP_LINUX_SENTINEL_SECONDARY_GEOMETRY", "480x320+620+160"
)


def _read_text(path: pathlib.Path) -> str:
    return path.read_text(encoding="utf-8").strip()


def _process_start_ticks() -> int:
    """Read the kernel identity component that distinguishes PID reuse."""

    stat = _read_text(pathlib.Path("/proc/self/stat"))
    command_end = stat.rfind(")")
    if command_end < 0:
        raise RuntimeError("/proc/self/stat does not contain a process command")
    fields_from_state = stat[command_end + 2 :].split()
    # /proc/<pid>/stat field 22 is starttime. The suffix begins at field 3.
    return int(fields_from_state[19])


def _fixture_sha256() -> str:
    return hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()


OBSERVER_IDENTITY = {
    "instance_id": OBSERVER_INSTANCE_ID,
    "pid": os.getpid(),
    "process_start_ticks": _process_start_ticks(),
    "boot_id": _read_text(pathlib.Path("/proc/sys/kernel/random/boot_id")),
    "display": os.environ.get("DISPLAY", ""),
    "fixture_sha256": _fixture_sha256(),
    "started_at_ms": STARTED_AT_MS,
    "event_source": "target_process_tk_x11_callbacks",
    "application_class": APPLICATION_CLASS,
    "fixture_role": FIXTURE_ROLE,
}


class SentinelApplication:
    """Own two real X11 surfaces and an independently observable event log."""

    def __init__(self) -> None:
        self.root = tk.Tk(className=APPLICATION_CLASS)
        self.root.title(f"{TITLE_PREFIX} A")
        self.root.geometry(PRIMARY_GEOMETRY)
        self.tick = 0
        self.events: list[dict[str, object]] = []
        self.event_sequence = 0
        self.observer_error_count = 0
        self.last_observer_error: dict[str, object] | None = None
        self.x11 = ctypes.CDLL("libX11.so.6")
        self.x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x11.XOpenDisplay.restype = ctypes.c_void_p
        self.x11.XQueryTree.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.c_ulong),
            ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)),
            ctypes.POINTER(ctypes.c_uint),
        ]
        self.x11.XQueryTree.restype = ctypes.c_int
        self.x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
        self.x11.XInternAtom.restype = ctypes.c_ulong
        self.x11.XChangeProperty.argtypes = [
            ctypes.c_void_p,
            ctypes.c_ulong,
            ctypes.c_ulong,
            ctypes.c_ulong,
            ctypes.c_int,
            ctypes.c_int,
            ctypes.POINTER(ctypes.c_ubyte),
            ctypes.c_int,
        ]
        self.x11.XChangeProperty.restype = ctypes.c_int
        self.x11.XFlush.argtypes = [ctypes.c_void_p]
        self.x11.XFlush.restype = ctypes.c_int
        self.x11.XFree.argtypes = [ctypes.c_void_p]
        self.x11_display = self.x11.XOpenDisplay(None)
        if not self.x11_display:
            raise RuntimeError("sentinel could not open the configured X11 display")
        self.secondary: tk.Toplevel | None = None
        self.labels: dict[str, tk.Label] = {}
        self.processed_command_id: str | None = None
        self.last_command: dict[str, object] | None = None
        self._install_surface("A", self.root)
        self._reopen_secondary()
        self.root.after(50, self._update)

    def _install_surface(self, surface: str, window: tk.Toplevel | tk.Tk) -> None:
        self.labels[surface] = self._build_surface(window, surface)
        for event_name in (
            "<ButtonPress>",
            "<ButtonRelease>",
            "<Motion>",
            "<KeyPress>",
            "<KeyRelease>",
            "<MouseWheel>",
            "<Button-4>",
            "<Button-5>",
        ):
            window.bind(
                event_name,
                lambda event, bound_surface=surface: self._record_event(event, bound_surface),
                add=True,
            )

    def _reopen_secondary(self) -> None:
        if self.secondary is not None and self.secondary.winfo_exists():
            raise ValueError("secondary surface already exists")
        # Tk normalizes the root's WM_CLASS. Reuse that exact published class
        # so process-owned windows also form one stable application identity.
        self.secondary = tk.Toplevel(self.root, class_=self.root.winfo_class())
        self.secondary.title(f"{TITLE_PREFIX} B")
        self.secondary.geometry(SECONDARY_GEOMETRY)
        self._install_surface("B", self.secondary)

    def _surface_window(self, surface: object) -> tk.Toplevel | tk.Tk:
        if surface == "A":
            return self.root
        if surface == "B" and self.secondary is not None and self.secondary.winfo_exists():
            return self.secondary
        raise ValueError(f"surface {surface!r} is unavailable")

    @staticmethod
    def _required_int(command: dict[str, object], field: str, *, positive: bool = False) -> int:
        value = command.get(field)
        if isinstance(value, bool) or not isinstance(value, int):
            raise ValueError(f"{field} must be an integer")
        if positive and value <= 0:
            raise ValueError(f"{field} must be positive")
        return value

    def _execute_command(self, command: dict[str, object]) -> None:
        action = command.get("action")
        if action == "move":
            window = self._surface_window(command.get("surface"))
            window.geometry(
                f"{window.winfo_width()}x{window.winfo_height()}"
                f"+{self._required_int(command, 'x')}+{self._required_int(command, 'y')}"
            )
        elif action == "resize":
            window = self._surface_window(command.get("surface"))
            window.geometry(
                f"{self._required_int(command, 'width', positive=True)}x"
                f"{self._required_int(command, 'height', positive=True)}"
                f"+{window.winfo_x()}+{window.winfo_y()}"
            )
        elif action == "move_resize":
            window = self._surface_window(command.get("surface"))
            window.geometry(
                f"{self._required_int(command, 'width', positive=True)}x"
                f"{self._required_int(command, 'height', positive=True)}"
                f"+{self._required_int(command, 'x')}+{self._required_int(command, 'y')}"
            )
        elif action == "close_secondary":
            window = self._surface_window("B")
            window.destroy()
            self.secondary = None
            self.labels.pop("B", None)
        elif action == "reopen_secondary":
            self._reopen_secondary()
        elif action == "focus":
            window = self._surface_window(command.get("surface"))
            window.deiconify()
            window.lift()
            window.focus_force()
        else:
            raise ValueError(f"unsupported action {action!r}")

    def _apply_pending_command(self) -> None:
        try:
            command = json.loads(COMMAND_PATH.read_text(encoding="utf-8"))
        except FileNotFoundError:
            return
        except (OSError, json.JSONDecodeError) as error:
            self.last_command = {
                "command_id": None,
                "status": "rejected",
                "detail": f"could not read command: {error}",
                "observed_at_ms": int(time.time() * 1000),
            }
            return
        if not isinstance(command, dict):
            return
        command_id = command.get("command_id")
        if not isinstance(command_id, str) or not command_id or command_id == self.processed_command_id:
            return
        try:
            self._execute_command(command)
            status = "applied"
            detail = None
        except (tk.TclError, ValueError) as error:
            status = "rejected"
            detail = str(error)
        self.processed_command_id = command_id
        self.last_command = {
            "command_id": command_id,
            "action": command.get("action"),
            "status": status,
            "detail": detail,
            "observed_at_ms": int(time.time() * 1000),
        }

    @staticmethod
    def _build_surface(window: tk.Misc, label: str) -> tk.Label:
        surface = tk.Label(
            window,
            text=f"{TITLE_PREFIX} Linux Surface {label}",
            font=("DejaVu Sans", 24, "bold"),
            foreground="#ffffff",
            background="#134e4a" if label == "A" else "#7c2d12",
            borderwidth=12,
            relief="solid",
        )
        surface.pack(fill="both", expand=True)
        surface.focus_set()
        return surface

    def _record_event(self, event: tk.Event, surface: str) -> None:
        def event_int(field: str) -> int:
            """Normalize Tk's platform-dependent missing-value sentinel.

            Tk exposes fields that do not apply to a given event as the string
            ``??`` on X11.  The observer must never drop a real host event just
            because an unrelated field is absent.
            """

            value = getattr(event, field, 0)
            if isinstance(value, bool):
                return int(value)
            if isinstance(value, (int, float)):
                return int(value)
            try:
                return int(str(value))
            except (TypeError, ValueError):
                return 0

        try:
            event_type_code = event_int("type")
            event_name = {
                2: "key_press",
                3: "key_release",
                4: "button_press",
                5: "button_release",
                6: "pointer_motion",
                38: "mouse_wheel",
            }.get(event_type_code, f"x11_event_{event_type_code}")
            kind = (
                "keyboard"
                if event_type_code in (2, 3)
                else "pointer"
                if event_type_code in (4, 5, 6, 38)
                else "other"
            )
            action = {
                2: "down",
                3: "up",
                4: "down",
                5: "up",
                6: "move",
                38: "wheel",
            }.get(event_type_code, "observe")
            observed_at_ms = int(time.time() * 1000)
            self.event_sequence += 1
            self.events.append(
                {
                    "sequence": self.event_sequence,
                    "at_ms": observed_at_ms,
                    "type": event_name,
                    "type_code": event_type_code,
                    "kind": kind,
                    "action": action,
                    "surface": surface,
                    "client_window_id": int(event.widget.winfo_toplevel().winfo_id()),
                    "native_window_id": self._native_window_id(event.widget.winfo_toplevel()),
                    "x": event_int("x"),
                    "y": event_int("y"),
                    "button": event_int("num"),
                    "keycode": event_int("keycode"),
                    "keysym": str(getattr(event, "keysym", "")),
                    "delta": event_int("delta"),
                }
            )
            del self.events[:-256]
        except Exception as error:  # The observer must report, never hide, callback failure.
            self.observer_error_count += 1
            self.last_observer_error = {
                "at_ms": int(time.time() * 1000),
                "type": type(error).__name__,
                "message": str(error),
            }

    def _native_window_id(self, window: tk.Toplevel | tk.Tk) -> int:
        """Return the WM-facing client XID used by xcap Resource identity."""

        client_window_id = int(window.winfo_id())
        root = ctypes.c_ulong()
        parent = ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        child_count = ctypes.c_uint()
        queried = self.x11.XQueryTree(
            self.x11_display,
            client_window_id,
            ctypes.byref(root),
            ctypes.byref(parent),
            ctypes.byref(children),
            ctypes.byref(child_count),
        )
        if children:
            self.x11.XFree(children)
        if queried == 0 or parent.value == 0:
            raise RuntimeError(f"XQueryTree failed for Tk client XID {client_window_id}")
        return int(parent.value)

    def _window_state(self, window: tk.Toplevel | tk.Tk, surface: str) -> dict[str, object]:
        client_window_id = int(window.winfo_id())
        native_window_id = self._native_window_id(window)
        self._publish_process_identity(native_window_id)
        return {
            "surface": surface,
            "title": window.title(),
            "client_window_id": client_window_id,
            "native_window_id": native_window_id,
            "x": int(window.winfo_x()),
            "y": int(window.winfo_y()),
            "width": int(window.winfo_width()),
            "height": int(window.winfo_height()),
            "viewable": bool(window.winfo_viewable()),
        }

    def _publish_process_identity(self, native_window_id: int) -> None:
        """Publish the EWMH PID used by real Linux window discovery APIs."""

        pid_atom = self.x11.XInternAtom(self.x11_display, b"_NET_WM_PID", 0)
        cardinal_atom = self.x11.XInternAtom(self.x11_display, b"CARDINAL", 0)
        if pid_atom == 0 or cardinal_atom == 0:
            raise RuntimeError("could not resolve X11 atoms for _NET_WM_PID")
        pid = ctypes.c_ulong(os.getpid())
        status = self.x11.XChangeProperty(
            self.x11_display,
            native_window_id,
            pid_atom,
            cardinal_atom,
            32,
            0,
            ctypes.cast(ctypes.pointer(pid), ctypes.POINTER(ctypes.c_ubyte)),
            1,
        )
        if status == 0:
            raise RuntimeError(
                f"XChangeProperty failed for native window {native_window_id}"
            )
        self.x11.XFlush(self.x11_display)

    def _write_state(self) -> None:
        windows = [self._window_state(self.root, "A")]
        if self.secondary is not None and self.secondary.winfo_exists():
            windows.append(self._window_state(self.secondary, "B"))
        state = {
            "schema": "easynet.remoteapp.linux-x11-sentinel.v1",
            "fixture_role": FIXTURE_ROLE,
            "pid": os.getpid(),
            "display": os.environ.get("DISPLAY", ""),
            "observer_identity": OBSERVER_IDENTITY,
            "command_path": str(COMMAND_PATH),
            "started_at_ms": STARTED_AT_MS,
            "observed_at_ms": int(time.time() * 1000),
            "tick": self.tick,
            "windows": windows,
            "events": list(self.events),
            "observer_health": {
                "status": "healthy" if self.observer_error_count == 0 else "degraded",
                "callback_error_count": self.observer_error_count,
                "event_count": self.event_sequence,
                "last_event_at_ms": self.events[-1]["at_ms"] if self.events else None,
                "last_error": self.last_observer_error,
            },
            "last_command": self.last_command,
        }
        STATE_PATH.parent.mkdir(parents=True, exist_ok=True)
        temporary = STATE_PATH.with_suffix(STATE_PATH.suffix + ".tmp")
        temporary.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        temporary.replace(STATE_PATH)

    def _update(self) -> None:
        self._apply_pending_command()
        self.tick += 1
        phase = self.tick % 240
        colors = (
            ("#134e4a", "#7c2d12")
            if phase < 120
            else ("#1d4ed8", "#6b21a8")
        )
        for index, surface in enumerate(("A", "B")):
            label = self.labels.get(surface)
            if label is None:
                continue
            label.configure(
                background=colors[index],
                text=(
                    f"{TITLE_PREFIX} Linux Surface {surface}\n"
                    f"PID {os.getpid()} · frame {self.tick}"
                ),
            )
        self._write_state()
        self.root.after(50, self._update)

    def run(self) -> None:
        self.root.mainloop()


if __name__ == "__main__":
    SentinelApplication().run()
