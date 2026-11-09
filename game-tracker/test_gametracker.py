import unittest

import gametracker as gt


class GameTrackerTest(unittest.TestCase):
    def test_game_for_longest_prefix_case_insensitive(self):
        folders = {gt.norm(r"M:\Battle.net"): "Battle.net", gt.norm(r"M:\Battle.net\Overwatch"): "Overwatch",
                   gt.norm(r"M:\StarCraft II"): "StarCraft II"}
        self.assertEqual(gt.game_for(r"m:\battle.net\overwatch\Overwatch.exe", folders), "Overwatch")
        self.assertEqual(gt.game_for(r"M:\StarCraft II\Versions\SC2_x64.exe", folders), "StarCraft II")
        self.assertIsNone(gt.game_for(r"M:\StarCraft II Editor\x.exe", folders))  # prefix of name, not folder
        self.assertIsNone(gt.game_for(r"C:\Windows\explorer.exe", folders))

    def test_steam_parsers(self):
        vdf = '"libraryfolders" { "0" { "path" "C:\\\\Program Files (x86)\\\\Steam" } "1" { "path" "D:\\\\SteamLibrary" } }'
        self.assertEqual(gt.steam_libraries(vdf), [r"C:\Program Files (x86)\Steam", r"D:\SteamLibrary"])
        acf = '"AppState" { "appid" "730" "name" "Counter-Strike 2" "installdir" "Counter-Strike Global Offensive" }'
        self.assertEqual(gt.steam_app(acf), ("Counter-Strike 2", "Counter-Strike Global Offensive"))

    def test_parse_extras(self):
        self.assertEqual(gt.parse_extras("# comment\nRetroArch = C:\\RetroArch\n\n"), {gt.norm(r"C:\RetroArch"): "RetroArch"})

    def test_daily_hours(self):
        rows = [("2026-11-09T20:00:00", "A", 1800), ("2026-11-09T20:30:00", "A", 1800), ("2026-11-10T01:00:00", "B", 60)]
        out = gt.daily_hours(rows)
        self.assertEqual(out["2026-11-09"]["A"], 1.0)
        self.assertAlmostEqual(out["2026-11-10"]["B"], 1 / 60)

    def test_watch_stops_and_records(self):
        import os, tempfile, threading
        from unittest import mock
        stop, ticks = threading.Event(), []
        with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as d, \
                mock.patch.object(gt, "DB", os.path.join(d, "t.db")), \
                mock.patch.object(gt, "discover_games", lambda: {gt.norm(r"M:\StarCraft II"): "StarCraft II"}), \
                mock.patch.object(gt, "running_exes", lambda: [r"M:\StarCraft II\SC2.exe"] * 2):
            gt.watch(5, stop, lambda g: (ticks.append(g), stop.set()))
            db = gt.open_db()
            rows = db.execute("SELECT game, secs FROM samples").fetchall()
            db.close()
        self.assertEqual(ticks, [{"StarCraft II"}])
        self.assertEqual(rows, [("StarCraft II", 5)])  # two processes, one sample


if __name__ == "__main__":
    unittest.main()
