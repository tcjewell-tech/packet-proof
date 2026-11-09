# Game time tracker

Logs which games are running once a minute and charts hours played per day. Windows, Python 3. The command line uses only the standard library; the window and tray icon need two packages.

Games are found from each launcher's own install records: Steam, Battle.net, EA app, Activision, Epic, GOG, Ubisoft Connect and the Xbox app. For anything else (emulators, DRM-free games), add lines to `games.txt` next to the script:

```
RetroArch = C:\RetroArch
```

## Window and tray icon

```bash
pip install -r requirements.txt
```

```bash
pythonw gui.py
```
Opens a small window and puts a dot in the system tray: green while tracking, grey when stopped. Hover it to see the game being tracked. Tracking starts as soon as it opens. Closing the window hides it to the tray; right-click the tray icon for Show, Start/Stop tracking, Open chart and Quit.

## Command line (no extra packages)

```bash
python gametracker.py games
```
Lists the games it can detect.

```bash
pythonw gametracker.py watch
```
Runs in the background with no window and writes `games.db`.

```bash
python gametracker.py plot
```
Writes `report.html` and opens it in your browser.

## Start with Windows

Tick **Start with Windows** in the window (or the tray icon's **Options** menu). The other options there: start minimized to the tray, send close and minimize to the tray, and start tracking on launch; they are saved in `settings.json`. For the CLI without a window, put a shortcut to `pythonw "<full path>\gametracker.py" watch` in `shell:startup`.
