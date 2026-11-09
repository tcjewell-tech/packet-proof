"""Game Tracker window and system tray icon. Run with: pythonw gui.py

Closing the window hides it to the tray; use the tray icon's Quit to exit.
"""
import json
import os
import sys
import threading
import tkinter as tk
import winreg

import pystray
from PIL import Image, ImageDraw

import gametracker as gt


# Light-theme tokens from the Game Tracker design system (https://claude.ai/artifact/Qa6AgGKyAADf5yJ2VhjZn1)
SURFACE, INK = "#f0f0f0", "#000000"
TRACKING, TRACKING_INK, STOPPED, DOT_OUTLINE = "#2e9d4f", "#1f7a3b", "#808080", "#ffffff"
BUTTON_FACE, BUTTON_BORDER, FOCUS = "#e1e1e1", "#adadad", "#0078d7"
SPACE_1, SPACE_2, SPACE_3 = 6, 12, 18
STATUS_FONT, BUTTON_FONT = ("Segoe UI", 11), ("Segoe UI", 9)


def dot(color):
    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    ImageDraw.Draw(img).ellipse((6, 6, 58, 58), fill=color, outline=DOT_OUTLINE, width=4)
    return img


ICON_ON, ICON_OFF = dot(TRACKING), dot(STOPPED)

SETTINGS = os.path.join(gt.HERE, "settings.json")
DEFAULTS = {"start_minimized": False, "close_to_tray": True, "track_on_launch": True}
RUN_KEY, RUN_NAME = r"Software\Microsoft\Windows\CurrentVersion\Run", "GameTracker"


def load_settings():
    try:
        with open(SETTINGS, encoding="utf-8") as f:
            return {**DEFAULTS, **json.load(f)}
    except (OSError, ValueError):
        return dict(DEFAULTS)


def startup_command():
    pyw = os.path.join(os.path.dirname(sys.executable), "pythonw.exe")
    return f'"{pyw}" "{os.path.abspath(__file__)}"'


def startup_enabled():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as k:
            return winreg.QueryValueEx(k, RUN_NAME)[0] == startup_command()
    except OSError:
        return False


def set_startup(on):
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY, 0, winreg.KEY_SET_VALUE) as k:
        if on:
            winreg.SetValueEx(k, RUN_NAME, 0, winreg.REG_SZ, startup_command())
        else:
            try:
                winreg.DeleteValue(k, RUN_NAME)
            except FileNotFoundError:
                pass


def button(parent, text, command):
    """Square button: button-face, 1px button-border that turns focus-blue on hover and keyboard focus.
    Returns (frame, button); the 1px frame is the border, since Tk on Windows won't draw a colored one."""
    frame = tk.Frame(parent, bg=BUTTON_BORDER, padx=1, pady=1)
    b = tk.Button(frame, text=text, command=command, width=14, font=BUTTON_FONT, fg=INK, bg=BUTTON_FACE,
                  activebackground=BUTTON_FACE, relief="flat", bd=0, pady=2, cursor="hand2")
    b.pack()
    hot = lambda on: frame.config(bg=FOCUS if on or b.focus_get() is b else BUTTON_BORDER)
    for ev, on in (("<Enter>", True), ("<FocusIn>", True), ("<Leave>", False), ("<FocusOut>", False)):
        b.bind(ev, lambda _, on=on: hot(on))
    return frame, b


class App:
    def __init__(self):
        self.stop = None
        self.settings = load_settings()
        self.root = root = tk.Tk()
        root.title("Game Tracker")
        root.geometry("320x250")
        root.resizable(False, False)
        root.configure(bg=SURFACE)
        root.protocol("WM_DELETE_WINDOW", self.close)
        root.bind("<Unmap>", lambda e: e.widget is root and root.state() == "iconic" and self.close())  # minimize = hide to tray
        self.status = tk.StringVar(value="Stopped")
        self.status_label = tk.Label(root, textvariable=self.status, font=STATUS_FONT, wraplength=290, bg=SURFACE, fg=INK)
        self.status_label.pack(pady=(SPACE_3, SPACE_2))
        row = tk.Frame(root, bg=SURFACE)
        row.pack()
        toggle_frame, self.toggle_btn = button(row, "Start tracking", self.toggle)
        toggle_frame.pack(side="left", padx=(0, SPACE_1))
        button(row, "Open chart", self.chart)[0].pack(side="left")

        opts = tk.Frame(root, bg=SURFACE)
        opts.pack(pady=(SPACE_3, 0), anchor="w", padx=SPACE_3)
        self.vars = {}
        for key, label in (("startup", "Start with Windows"), ("start_minimized", "Start minimized to tray"),
                           ("close_to_tray", "Close and minimize go to tray"), ("track_on_launch", "Start tracking on launch")):
            v = self.vars[key] = tk.BooleanVar(value=startup_enabled() if key == "startup" else self.settings[key])
            tk.Checkbutton(opts, text=label, variable=v, command=lambda k=key: self.set_option(k, self.vars[k].get()),
                           font=BUTTON_FONT, bg=SURFACE, fg=INK, activebackground=SURFACE, anchor="w").pack(fill="x")

        Item = pystray.MenuItem
        opt = lambda key, label: Item(label, lambda: self.ui(self.set_option, key, not self.vars[key].get()),
                                      checked=lambda _: self.vars[key].get())
        self.tray = pystray.Icon("gametracker", ICON_OFF, "Game Tracker", pystray.Menu(
            Item("Show", lambda: self.ui(self.show), default=True),
            Item(lambda _: "Stop tracking" if self.stop else "Start tracking", lambda: self.ui(self.toggle)),
            Item("Open chart", lambda: self.ui(self.chart)),
            Item("Options", pystray.Menu(
                opt("startup", "Start with Windows"), opt("start_minimized", "Start minimized to tray"),
                opt("close_to_tray", "Close and minimize go to tray"), opt("track_on_launch", "Start tracking on launch"))),
            Item("Quit", lambda: self.ui(self.quit))))
        self.tray.run_detached()
        if self.settings["start_minimized"]:
            root.withdraw()
        if self.settings["track_on_launch"]:
            self.toggle()

    def set_option(self, key, on):
        self.vars[key].set(on)
        if key == "startup":
            set_startup(on)
        else:
            self.settings[key] = on
            with open(SETTINGS, "w", encoding="utf-8") as f:
                json.dump(self.settings, f, indent=1)
        self.tray.update_menu()

    def close(self):
        if self.settings["close_to_tray"]:
            self.root.withdraw()
        elif self.root.state() != "iconic":
            self.quit()

    def ui(self, fn, *args):
        self.root.after(0, fn, *args)  # tray and watcher run on other threads; Tk must be touched from its own

    def show(self):
        self.root.deiconify()
        self.root.lift()

    def toggle(self):
        if self.stop:
            self.stop.set()
            self.stop = None
            self.status.set("Stopped")
        else:
            self.stop = threading.Event()
            self.status.set("Tracking: no game running")
            threading.Thread(target=gt.watch, kwargs={"stop": self.stop, "on_tick": self.tick}, daemon=True).start()
        self.toggle_btn.config(text="Stop tracking" if self.stop else "Start tracking")
        self.status_label.config(fg=TRACKING_INK if self.stop else INK)
        self.tray.icon = ICON_ON if self.stop else ICON_OFF
        self.tray.title = "Game Tracker: no game running" if self.stop else "Game Tracker: Stopped"
        self.tray.update_menu()

    def tick(self, games):
        self.ui(self.set_playing, ", ".join(sorted(games)) or "no game running")

    def set_playing(self, text):
        if self.stop:
            self.status.set(f"Tracking: {text}")
            self.status_label.config(fg=TRACKING_INK)
            self.tray.title = f"Game Tracker: {text}"[:127]  # Windows tooltip limit

    def chart(self):
        if not gt.plot():
            self.status.set("No data yet. Play something while tracking.")
            self.status_label.config(fg=INK)

    def quit(self):
        if self.stop:
            self.stop.set()
        self.tray.stop()
        self.root.destroy()


if __name__ == "__main__":
    App().root.mainloop()
