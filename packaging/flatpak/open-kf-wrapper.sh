#!/bin/sh
# Starts Open KF inside the Flatpak sandbox.
# The game writes logs/, settings/ and work/ in the folder it runs in, so it
# runs in the app's own data folder (~/.var/app/io.github.scronkfinkle.OpenKF/data).
cd "$XDG_DATA_HOME" || exit 1
# Killing Floor installed by the Steam Flatpak: the game does not look there
# by itself, so point it there (unless the player chose a folder).
if [ -z "$KF_ROOT" ]; then
    flatpak_steam="$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common/KillingFloor"
    [ -d "$flatpak_steam" ] && export KF_ROOT="$flatpak_steam"
fi
exec /app/bin/open-kf "$@"
