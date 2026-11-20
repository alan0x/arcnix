// Modal hosts remain outside the inert workspace. Share the visible desktop
// rectangle with that overlay layer and with viewport-positioned popups.
const desktopFrame = (() => {
    let canvas;
    let dock;

    function dockInsets(canvas) {
        const zone = canvas.querySelector('.desktop-dock-zone');
        const insets = {left: 0, right: 0, bottom: 0};
        if (zone?.classList.contains('dock-left')) insets.left = zone.offsetWidth;
        else if (zone?.classList.contains('dock-right')) insets.right = zone.offsetWidth;
        else if (zone) insets.bottom = zone.offsetHeight;
        return insets;
    }

    function sync() {
        const root = document.documentElement;
        if (!canvas || !root) return;
        const box = canvas.getBoundingClientRect();
        if (!box.width || !box.height) return;
        const inset = [box.top, innerWidth - box.right, innerHeight - box.bottom, box.left];
        root.style.setProperty('--desktop-frame-inset', inset.map(n => `${Math.max(0, n)}px`).join(' '));
        root.style.setProperty('--desktop-frame-width', `${box.width}px`);
        root.style.setProperty('--desktop-frame-height', `${box.height}px`);
        for (const [edge, size] of Object.entries(dockInsets(canvas))) {
            root.style.setProperty(`--desktop-dock-${edge}`, `${size}px`);
        }
    }

    function sizeDockTiles() {
        if (dock) dock.style.setProperty('--dock-tiles', String(1 + dock.querySelectorAll('.desktop-dock-app').length));
    }
    const dockItems = new MutationObserver(sizeDockTiles);
    const dockPosition = new MutationObserver(sync);
    const resize = new ResizeObserver(sync);
    function bind() {
        const next = document.querySelector('.desktop-canvas');
        if (next === canvas) return;
        resize.disconnect();
        dockItems.disconnect();
        dockPosition.disconnect();
        canvas = next;
        dock = canvas?.querySelector('.desktop-dock-zone');
        if (!canvas) return;
        resize.observe(canvas);
        if (dock) {
            resize.observe(dock);
            dockItems.observe(dock, {childList: true, subtree: true});
            dockPosition.observe(dock, {attributes: true, attributeFilter: ['class']});
            sizeDockTiles();
        }
        sync();
    }
    new MutationObserver(bind).observe(document, {childList: true, subtree: true});
    window.addEventListener('resize', sync);
    bind();

    function boundsFor(element) {
        if (element.closest('.desktop-canvas,.desktop-modal-backdrop') && canvas) {
            const box = canvas.getBoundingClientRect();
            if (box.width && box.height) return box;
        }
        return {left: 0, top: 0, right: innerWidth, bottom: innerHeight};
    }

    return {boundsFor, dockInsets};
})();
