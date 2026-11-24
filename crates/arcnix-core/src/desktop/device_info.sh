# Read the hardware and operating system visible to the current desktop session.
export LC_ALL=C
field() { printf '%s\t%s\n' "$1" "$2"; }
field arch "$(uname -m)"
case "$(uname -s)" in
Linux)
    release=/etc/os-release
    [ -r "$release" ] || release=/usr/lib/os-release
    # Treat the release file as data, including on a read-only local desktop.
    release_value() {
        sed -n "s/^$1=//p" "$release" 2>/dev/null | head -n 1 | sed "s/^[\"']//;s/[\"']$//"
    }
    name=$(release_value NAME)
    field os "${name:-Linux}"
    field version "$(release_value VERSION_ID)"
    field kernel "$(uname -r)"
    model=$(awk -F ': *' '/^model name[[:space:]]*:/ {print $2; exit}' /proc/cpuinfo)
    if [ -z "$model" ]; then
        model=$(lscpu 2>/dev/null | sed -n 's/^Model name:[[:space:]]*//p' | head -n 1)
    fi
    field cpu "$model"
    # Logical CPUs on the same core report the same list of sibling CPUs.
    awk 'NF {cores[$0]=1} END {for (c in cores) n++; if (n) print "cores\t" n}' \
        /sys/devices/system/cpu/cpu[0-9]*/topology/thread_siblings_list 2>/dev/null
    field logical "$(getconf _NPROCESSORS_ONLN 2>/dev/null || grep -c '^processor[[:space:]]*:' /proc/cpuinfo)"
    awk '/^MemTotal:/ {printf "memory\t%.0f\n", $2 * 1024}' /proc/meminfo
    if command -v lspci >/dev/null 2>&1; then
        lspci -nn | sed -n 's/^.* \[03[0-9a-fA-F][0-9a-fA-F]\]: /gpu	/p'
    else
        for slot in /sys/bus/pci/devices/*; do
            [ -r "$slot/class" ] && read -r class < "$slot/class" || continue
            case "$class" in 0x03*) ;; *) continue ;; esac
            model=$(udevadm info -q property -p "$slot" 2>/dev/null | sed -n 's/^ID_MODEL_FROM_DATABASE=//p')
            if [ -z "$model" ]; then
                read -r vendor < "$slot/vendor" && read -r device < "$slot/device" || continue
                model="PCI ${vendor#0x}:${device#0x}"
            fi
            field gpu "$model"
        done
    fi
    ;;
Darwin)
    field os "$(sw_vers -productName)"
    field version "$(sw_vers -productVersion)"
    field kernel "$(uname -r)"
    field cpu "$(sysctl -n machdep.cpu.brand_string)"
    field cores "$(sysctl -n hw.physicalcpu)"
    field logical "$(sysctl -n hw.logicalcpu)"
    field memory "$(sysctl -n hw.memsize)"
    /usr/sbin/system_profiler SPDisplaysDataType | sed -n 's/^[[:space:]]*Chipset Model: /gpu	/p'
    ;;
*) exit 1 ;;
esac
