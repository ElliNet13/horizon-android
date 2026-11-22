#!/system/bin/sh
# Horizon's watchdog: started by the game, it waits for the game's process to end and starts
# the game again when it ended in a crash (the game left "crashed", or asked for "restart", in
# the folder). A game closed on purpose leaves neither, and the watchdog ends with it.
# Arguments: the game's process id, the folder for the markers, the package name.
# Everything it does goes to watchdog.log in that folder.
app="$1"
dir="$2"
package="$3"
log="$dir/watchdog.log"

say() {
    echo "$(date +%T) [$$] $*" >> "$log"
}

echo "$$" > "$dir/watchdog.pid"
say "watching the game (pid $app, package $package)"
while [ -d "/proc/$app" ]; do
    sleep 1
done
say "the game (pid $app) is gone"
ls -la "$dir" >> "$log" 2>&1

if [ -f "$dir/crashed" ] || [ -f "$dir/restart" ]; then
    say "marker found: $(cat "$dir/crashed" "$dir/restart" 2>/dev/null | head -3 | tr '\n' ' ')"
    rm -f "$dir/restart"
    tries=0
    while [ "$tries" -lt 5 ]; do
        tries=$((tries + 1))
        say "starting the game (try $tries)"
        out=$(/system/bin/am start -n "$package/android.app.NativeActivity" 2>&1)
        say "am: $out"
        case "$out" in
            *Error*|*Exception*|*"not started"*|*denied*) ;;
            *) break ;;
        esac
        out=$(/system/bin/monkey -p "$package" -c android.intent.category.LAUNCHER 1 2>&1 | tail -2)
        say "monkey: $out"
        sleep 1
    done
else
    say "no marker: it ended on its own"
fi
rm -f "$dir/watchdog.pid"
say "done"
