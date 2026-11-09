"""Game Tracker window and system tray icon. Run with: pythonw gui.py

Closing the window hides it to the tray; use the tray icon's Quit to exit.
"""
import threading
import tkinter as tk

import pystray
from PIL import Image, ImageDraw

import gametracker as gt


def dot(color):
    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    ImageDraw.Draw(img).ellipse((6, 6, 58, 58), fill=color, outline="white", width=4)
    return img


ICON_ON, ICON_OFF = dot("#2e9d4f"), dot("#808080")


class App:
    def __init__(self):
        self.stop = None
        self.root = root = tk.Tk()
        root.title("Game Tracker")
        root.geometry("320x150")
        root.resizable(False, False)
        root.protocol("WM_DELETE_WINDOW", root.withdraw)  # close = hide to tray
        self.status = tk.StringVar(value="Stopped")
        tk.Label(root, textvariable=self.status, font=("Segoe UI", 11), wraplength=290).pack(pady=(18, 12))
        row = tk.Frame(root)
        row.pack()
        self.toggle_btn = tk.Button(row, text="Start tracking", width=14, command=self.toggle)
        self.toggle_btn.pack(side="left", padx=6)
        tk.Button(row, text="Open chart", width=14, command=self.chart).pack(side="left", padx=6)

        Item = pystray.MenuItem
        self.tray = pystray.Icon("gametracker", ICON_OFF, "Game Tracker", pystray.Menu(
            Item("Show", lambda: self.ui(self.show), default=True),
            Item(lambda _: "Stop tracking" if self.stop else "Start tracking", lambda: self.ui(self.toggle)),
            Item("Open chart", lambda: self.ui(self.chart)),
            Item("Quit", lambda: self.ui(self.quit))))
        self.tray.run_detached()
        self.toggle()  # start tracking on launch

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
        self.tray.icon = ICON_ON if self.stop else ICON_OFF
        self.tray.title = f"Game Tracker: {self.status.get()}"
        self.tray.update_menu()

    def tick(self, games):
        self.ui(self.set_playing, ", ".join(sorted(games)) or "no game running")

    def set_playing(self, text):
        if self.stop:
            self.status.set(f"Tracking: {text}")
            self.tray.title = f"Game Tracker: {text}"[:127]  # Windows tooltip limit

    def chart(self):
        if not gt.plot():
            self.status.set("No data yet. Play something while tracking.")

    def quit(self):
        if self.stop:
            self.stop.set()
        self.tray.stop()
        self.root.destroy()


if __name__ == "__main__":
    App().root.mainloop()
