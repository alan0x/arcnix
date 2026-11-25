# arcnix:gpu:macos
# Only the adapter identity and selected counters cross the SSH connection.
# Foundation can read the registry plist even when unrelated keys contain binary data.
/usr/sbin/ioreg -a -r -d 1 -c IOAccelerator | /usr/bin/osascript -l JavaScript -e '
ObjC.import("Foundation");

function sample() {
    const data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
    const entries = $.NSPropertyListSerialization.propertyListWithDataOptionsFormatError(data, 0, null, null);
    const cards = new Map();
    const discrete = new Set();
    try {
        ObjC.import("Metal");
        if (typeof $.MTLCopyAllDevices !== "function") {
            ObjC.bindFunction("MTLCopyAllDevices", ["id", []]);
        }
        const devices = $.MTLCopyAllDevices();
        for (let i = 0; i < devices.count; ++i) {
            const device = devices.objectAtIndex(i);
            const id = String(device.registryID);
            cards.set(id, {id: id, name: ObjC.unwrap(device.name)});
            if (device.respondsToSelector("hasUnifiedMemory") && !device.hasUnifiedMemory) discrete.add(id);
        }
    } catch (_) {
        // Drivers without Metal can still publish IOAccelerator counters.
    }

    function number(stats, key, max) {
        const item = stats.objectForKey(key);
        if (!item || item.isNil()) return undefined;
        const value = ObjC.unwrap(item);
        return Number.isFinite(value) && value >= 0 && value <= max ? value : undefined;
    }
    for (let i = 0; i < entries.count; ++i) {
        const entry = entries.objectAtIndex(i);
        const id = ObjC.unwrap(entry.objectForKey("IORegistryEntryID"));
        if (!Number.isSafeInteger(id)) continue;
        const key = String(id);
        const card = cards.get(key) || {id: key, name: ObjC.unwrap(entry.objectForKey("IORegistryEntryName"))};
        const stats = entry.objectForKey("PerformanceStatistics");
        if (stats && !stats.isNil()) {
            card.utilization = number(stats, "Device Utilization %", 100);
            if (card.utilization === undefined) card.utilization = number(stats, "GPU Activity(%)", 100);
            card.renderer = number(stats, "Renderer Utilization %", 100);
            card.tiler = number(stats, "Tiler Utilization %", 100);
            card.shared = number(stats, "In use system memory", Number.MAX_SAFE_INTEGER);
            // VRAM counters are also exposed by some integrated GPUs. Only label
            // these bytes dedicated if the Metal device reports discrete memory.
            if (discrete.has(key)) {
                card.dedicated = number(stats, "vramUsedBytes", Number.MAX_SAFE_INTEGER);
            }
        }
        cards.set(key, card);
    }
    if (cards.size === 0) throw new Error("No accelerator inventory");
    return Array.from(cards.values(), card => "GPU\t" + JSON.stringify(card)).join("\n");
}
sample()
' || /usr/sbin/system_profiler SPDisplaysDataType -json
