#!/usr/bin/env zsh
set -euo pipefail

[[ $# == 0 || ( $# == 1 && $1 == --reset ) ]] || { print -u2 'Usage: scripts/install-ns7-driver.sh [--reset]'; exit 1; }
reset=${1:-}
project=${0:A:h:h}
kernel=$(uname -r)
module=$project/target/ns7-driver/snd_ns7.ko
destination=/lib/modules/$kernel/updates/omatainer

[[ -d /lib/modules/$kernel/build ]] || { print -u2 'Install headers for the running kernel first.'; exit 1; }
make -C "$project/drivers/ns7" test all
sudo -n mkdir -p "$destination"
sudo -n ln -sfn "$module" "$destination/snd_ns7.ko"
sudo -n depmod -a "$kernel"
if [[ -d /sys/module/snd_ns7 ]]; then
    installed=$(cat /sys/module/snd_ns7/srcversion 2>/dev/null || true)
    built=$(modinfo -F srcversion "$module")
    if [[ -z $installed || $installed != $built ]]; then
        for interface in /sys/bus/usb/drivers/snd_ns7/*:*(N); do
            number=$(cat "$interface/bInterfaceNumber" 2>/dev/null) || continue
            if [[ $number == 00 ]]; then
                print -r -- "${interface:t}" | sudo -n tee /sys/bus/usb/drivers/snd_ns7/unbind >/dev/null
            fi
        done
        for attempt in {1..50}; do
            [[ $(cat /sys/module/snd_ns7/refcnt) == 0 ]] && break
            sleep 0.1
        done
        sudo -n modprobe -r snd_ns7
    fi
fi
sudo -n modprobe snd_ns7
modinfo -F filename snd_ns7
if lsusb -d 15e4:0071 | rg -q .; then
    if [[ $reset == --reset ]]; then
        sudo -n usbreset 15e4:0071
    fi
    for attempt in {1..50}; do
        amidi -l | rg -q 'Numark NS7 MIDI' && break
        sleep 0.1
    done
fi
amidi -l
