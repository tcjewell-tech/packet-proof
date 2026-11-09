"""Logs which games are running and plots hours played per day per game.

  python gametracker.py watch [seconds]   leave running (use pythonw for no window)
  python gametracker.py plot              writes report.html and opens it
  python gametracker.py games             lists the games it knows how to detect
  pythonw gui.py                          window + system tray icon (needs pystray, pillow)
"""
import ctypes, glob, html, json, os, re, sqlite3, sys, time, webbrowser, winreg
from collections import defaultdict
from ctypes import wintypes
from datetime import date, datetime, timedelta

HERE = os.path.dirname(os.path.abspath(__file__))
DB = os.path.join(HERE, "games.db")
EXTRAS = os.path.join(HERE, "games.txt")
REPORT = os.path.join(HERE, "report.html")
STEAM = r"C:\Program Files (x86)\Steam"
EPIC_MANIFESTS = r"C:\ProgramData\Epic\EpicGamesLauncher\Data\Manifests\*.item"
XBOX = r"C:\XboxGames"
UNINSTALL = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"
PUBLISHERS = re.compile(r"Blizzard|Electronic Arts|Activision", re.I)
LAUNCHERS = re.compile(r"^(Battle\.net|EA app|EA)$", re.I)


# ---- discovering installed games: {normalised install folder: game name} ----

def norm(path):
    return os.path.normcase(os.path.normpath(path)) + os.sep


def steam_libraries(vdf_text):
    return [p.replace("\\\\", "\\") for p in re.findall(r'"path"\s+"([^"]+)"', vdf_text)]


def steam_app(acf_text):
    name = re.search(r'"name"\s+"([^"]+)"', acf_text)
    folder = re.search(r'"installdir"\s+"([^"]+)"', acf_text)
    return (name.group(1), folder.group(1)) if name and folder else None


def parse_extras(text):
    pairs = (line.split("=", 1) for line in text.splitlines() if "=" in line and not line.lstrip().startswith("#"))
    return {norm(path.strip()): name.strip() for name, path in pairs}


def reg_entries(path, view=0):
    """Yield the values of every subkey of HKLM\\path as a dict."""
    flags = winreg.KEY_READ | view
    try:
        key = winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, path, 0, flags)
    except OSError:
        return
    with key:
        for i in range(winreg.QueryInfoKey(key)[0]):
            try:
                with winreg.OpenKey(key, winreg.EnumKey(key, i), 0, flags) as sub:
                    yield {n: v for n, v, _ in (winreg.EnumValue(sub, j) for j in range(winreg.QueryInfoKey(sub)[1]))}
            except OSError:
                continue


def read(path):
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def discover_games():
    found = {}
    # Steam: every library's appmanifest gives the real name and folder
    for lib in steam_libraries(read(os.path.join(STEAM, "steamapps", "libraryfolders.vdf"))) or [STEAM]:
        for acf in glob.glob(os.path.join(lib, "steamapps", "appmanifest_*.acf")):
            app = steam_app(read(acf))
            # skip redistributables and servers; count modded Terraria as Terraria
            if app and app[1] != "Steamworks Shared" and not app[0].endswith("Dedicated Server"):
                found[norm(os.path.join(lib, "steamapps", "common", app[1]))] = "Terraria" if app[0] == "tModLoader" else app[0]
    # Battle.net / EA app / Activision register normal uninstall entries
    for view in (winreg.KEY_WOW64_64KEY, winreg.KEY_WOW64_32KEY):
        for e in reg_entries(UNINSTALL, view):
            name, loc = e.get("DisplayName"), e.get("InstallLocation")
            if name and loc and PUBLISHERS.search(str(e.get("Publisher", ""))) and not LAUNCHERS.match(name):
                found[norm(loc)] = name
    # Epic
    for item in glob.glob(EPIC_MANIFESTS):
        try:
            m = json.loads(read(item))
            found[norm(m["InstallLocation"])] = m["DisplayName"]
        except (ValueError, KeyError):
            pass
    # GOG and Ubisoft Connect
    for e in reg_entries(r"SOFTWARE\WOW6432Node\GOG.com\Games"):
        if e.get("path") and e.get("gameName"):
            found[norm(e["path"])] = e["gameName"]
    for e in reg_entries(r"SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs"):
        if e.get("InstallDir"):
            found[norm(e["InstallDir"])] = os.path.basename(os.path.normpath(e["InstallDir"]))
    # Xbox app
    for d in glob.glob(os.path.join(XBOX, "*", "")):
        if os.path.basename(os.path.normpath(d)).lower() == "gamesave":  # Xbox app save-data folder
            continue
        found[norm(d)] = os.path.basename(os.path.normpath(d))
    found.update(parse_extras(read(EXTRAS)))
    return found


def game_for(exe, folders):
    """Longest install folder containing exe wins, so a game inside a launcher folder isn't credited to the launcher."""
    p = os.path.normcase(exe)
    best = max((f for f in folders if p.startswith(f)), key=len, default=None)
    return folders[best] if best else None


# ---- watching ----

k32, psapi = ctypes.windll.kernel32, ctypes.windll.psapi
k32.OpenProcess.restype = wintypes.HANDLE


def running_exes():
    pids, needed = (wintypes.DWORD * 8192)(), wintypes.DWORD()
    psapi.EnumProcesses(pids, ctypes.sizeof(pids), ctypes.byref(needed))
    buf, exes = ctypes.create_unicode_buffer(1024), []
    for pid in pids[: needed.value // ctypes.sizeof(wintypes.DWORD)]:
        h = k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION also works on elevated games
        if not h:
            continue
        size = wintypes.DWORD(len(buf))
        if k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            exes.append(buf.value)
        k32.CloseHandle(h)
    return exes


def open_db():
    db = sqlite3.connect(DB)
    db.execute("CREATE TABLE IF NOT EXISTS samples(ts TEXT, game TEXT, secs INTEGER)")
    return db


def watch(interval=60, stop=None, on_tick=None):
    """Sample every `interval` s until `stop` (a threading.Event) is set; on_tick(games) after each sample."""
    db, folders, refreshed = open_db(), {}, 0
    while not (stop and stop.is_set()):
        if time.time() - refreshed > 3600:  # pick up newly installed games
            folders, refreshed = discover_games(), time.time()
        games = {g for g in (game_for(e, folders) for e in running_exes()) if g}
        now = datetime.now().isoformat(timespec="seconds")
        with db:
            db.executemany("INSERT INTO samples VALUES (?,?,?)", [(now, g, interval) for g in games])
        if on_tick:
            on_tick(games)
        if stop:
            stop.wait(interval)
        else:
            time.sleep(interval)
    db.close()


# ---- plotting ----

def daily_hours(rows):
    out = defaultdict(lambda: defaultdict(float))
    for ts, game, secs in rows:
        out[ts[:10]][game] += secs / 3600
    return out


def plot():
    by_day = daily_hours(open_db().execute("SELECT ts, game, secs FROM samples").fetchall())
    if not by_day:
        return False
    first, last = date.fromisoformat(min(by_day)), date.fromisoformat(max(by_day))
    days = [str(first + timedelta(i)) for i in range((last - first).days + 1)]
    games = sorted({g for d in by_day.values() for g in d})
    datasets = [{"label": g, "data": [round(by_day.get(d, {}).get(g, 0), 2) for d in days]} for g in games]
    totals = sorted(((sum(by_day[d].get(g, 0) for d in by_day), g) for g in games), reverse=True)
    rows = "".join(f"<tr><td>{html.escape(g)}</td><td>{h:.1f}</td></tr>" for h, g in totals)
    data = json.dumps({"labels": days, "datasets": datasets}).replace("</", "<\\/")
    with open(REPORT, "w", encoding="utf-8") as f:
        # tokens from the Game Tracker design system (https://claude.ai/artifact/Qa6AgGKyAADf5yJ2VhjZn1)
        f.write(f"""<!doctype html><meta charset="utf-8"><title>Game time</title>
<style>
:root{{--surface-raised:#ffffff;--ink:#000000;--ink-muted:#5c5c5c;--button-border:#adadad;--space-4:24px;color-scheme:light}}
@media (prefers-color-scheme:dark){{:root{{--surface-raised:#2b2b2b;--ink:#f2f2f2;--ink-muted:#a8a8a8;--button-border:#5a5a5a;color-scheme:dark}}}}
body{{margin:0;background:var(--surface-raised);color:var(--ink);font:400 14px/20px "Segoe UI",system-ui,sans-serif}}
main{{max-width:1100px;margin:0 auto;padding:var(--space-4)}}
h2{{font-size:21px;line-height:28px;font-weight:700;margin:var(--space-4) 0 12px}}
table{{border-collapse:collapse;font-variant-numeric:tabular-nums}}
th{{color:var(--ink-muted);font-weight:400;text-align:left}}
td,th{{padding:4px 24px 4px 0;border-bottom:1px solid var(--button-border)}}
td:last-child,th:last-child{{text-align:right;padding-right:0}}
</style>
<main><h2>Hours played per day</h2><canvas id="c"></canvas>
<h2>Totals</h2><table><tr><th>Game</th><th>Hours</th></tr>{rows}</table></main>
<script src="https://cdnjs.cloudflare.com/ajax/libs/Chart.js/4.4.1/chart.umd.min.js"></script>
<script>const css = getComputedStyle(document.documentElement), tok = n => css.getPropertyValue(n).trim();
Chart.defaults.color = tok("--ink-muted"); Chart.defaults.borderColor = tok("--button-border");
Chart.defaults.font.family = '"Segoe UI", system-ui, sans-serif'; Chart.defaults.font.size = 14;
new Chart(c, {{type: "bar", data: {data},
  options: {{scales: {{x: {{stacked: true}}, y: {{stacked: true, title: {{display: true, text: "hours"}}}}}}}}}});</script>""")
    webbrowser.open(REPORT)
    return True


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd == "watch":
        watch(int(sys.argv[2]) if len(sys.argv) > 2 else 60)
    elif cmd == "plot":
        plot() or sys.exit("No data yet: run `watch` while you play.")
    elif cmd == "games":
        for folder, name in sorted(discover_games().items(), key=lambda x: x[1].lower()):
            print(f"{name:40} {folder}")
    else:
        print(__doc__)
