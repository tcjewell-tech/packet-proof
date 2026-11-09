# Game time tracker

Logs which games are running once a minute and charts hours played per day. Python 3 standard library only, Windows.

Games are found from each launcher's own install records: Steam, Battle.net, EA app, Activision, Epic, GOG, Ubisoft Connect and the Xbox app. For anything else (emulators, DRM-free games), add lines to `games.txt` next to the script:

```
RetroArch = C:\RetroArch
```

## Use

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

Press Win+R, type `shell:startup`, and add a shortcut there with the target `pythonw "<full path>\gametracker.py" watch`.
