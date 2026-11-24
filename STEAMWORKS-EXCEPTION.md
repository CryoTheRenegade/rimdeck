# Steamworks linking exception

rimdeck is licensed under the GNU General Public License version 3 only (see `LICENSE`),
with the following additional permission.

## Additional permission under GNU GPL version 3 section 7

If you modify this Program, or any covered work, by linking or combining it with the
Steamworks API library distributed by Valve Corporation (`steam_api64.dll`, `steam_api.dll`,
`libsteam_api.so` or `libsteam_api.dylib`, or a modified version of that library), containing
parts covered by the terms of the Steamworks SDK Access Agreement, the licensors of this
Program grant you additional permission to convey the resulting work.

---

In rimdeck, only the `rimdeck-steam` helper (`crates/steam-helper`) links that library. The
GUI and TUI do not. Redistributing Valve's library itself is governed by Valve's Steamworks
SDK terms, not by this permission.
