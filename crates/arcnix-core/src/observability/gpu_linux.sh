export LC_ALL=C
for card in /sys/class/drm/card[0-9]*; do
    case "$card" in *-*) continue;; esac
    dev=$card/device
    [ -r "$dev/vendor" ] || continue
    slot=$(basename "$(readlink -f "$dev")")
    driver=$(basename "$(readlink -f "$dev/driver")")
    model=$(cat "$dev/product_name" 2>/dev/null)
    if [ -z "$model" ] && command -v lspci >/dev/null; then
        model=$(lspci -s "$slot" 2>/dev/null | sed 's/^[^ ]* [^:]*: //')
    fi
    [ -n "$model" ] || model="$driver ($slot)"
    temp=
    for sensor in "$dev"/hwmon/hwmon*/temp1_input; do
        [ -r "$sensor" ] && { temp=$(cat "$sensor"); break; }
    done
    printf 'DRM\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$slot" "$model" "$driver" \
        "$(cat "$dev/gpu_busy_percent" 2>/dev/null)" "$(cat "$dev/mem_info_vram_used" 2>/dev/null)" \
        "$(cat "$dev/mem_info_vram_total" 2>/dev/null)" "$temp"
done
if command -v nvidia-smi >/dev/null; then
    nvidia-smi --query-gpu=pci.bus_id,name,driver_version,utilization.gpu,utilization.encoder,utilization.decoder,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits 2>/dev/null | sed 's/^/NV	/'
fi
