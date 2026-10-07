#!/bin/sh
# Starts Open KF inside the Flatpak sandbox.
# The game writes logs/, settings/ and work/ in the folder it runs in, so it
# runs in the app's own data folder (~/.var/app/io.github.scronkfinkle.OpenKF/data).
cd "$XDG_DATA_HOME" || exit 1
exec /app/bin/open-kf "$@"
