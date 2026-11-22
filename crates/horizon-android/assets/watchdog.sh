#!/system/bin/sh
# Horizon's watchdog: started by the game, it waits for the game's process to end and starts
# the game again when it ended in a crash (the game left "crashed", or asked for "restart", in
# the folder). A game closed on purpose leaves neither, and the watchdog ends with it.
# Arguments: the game's process id, the folder for the markers, the package name.
app="$1"
dir="$2"
package="$3"
log="$dir/watchdog.log"

echo "$$" > "$dir/watchdog.pid"
while [ -d "/proc/$app" ]; do
    sleep 1
done

if [ -f "$dir/crashed" ] || [ -f "$dir/restart" ]; then
    rm -f "$dir/restart"
    echo "$(date): the game (pid $app) is gone, starting it again" >> "$log"
    /system/bin/am start -n "$package/android.app.NativeActivity" >> "$log" 2>&1
else
    echo "$(date): the game (pid $app) ended on its own" >> "$log"
fi
rm -f "$dir/watchdog.pid"
