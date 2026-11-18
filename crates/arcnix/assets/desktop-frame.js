// Modal hosts remain outside the inert workspace. Share the visible desktop
// rectangle with that overlay layer and with viewport-positioned popups.
const desktopFrame = (() => {
    let canvas;
    let dock;

    function sync() {
        const root = document.documentElement;
        if (!canvas || !root) return;
        const box = canvas.getBoundingClientRect();
        if (!box.width || !box.height) return;
        const inset = [box.top, innerWidth - box.right, innerHeight - box.bottom, box.left];
        root.style.setProperty('--desktop-frame-inset', inset.map(n => `${Math.max(0, n)}px`).join(' '));
        root.style.setProperty('--desktop-frame-width', `${box.width}px`);
        root.style.setProperty('--desktop-frame-height', `${box.height}px`);
        const dockTop = dock?.getBoundingClientRect().top ?? box.bottom;
        root.style.setProperty('--desktop-dock-clearance', `${Math.min(box.height / 4, Math.max(0, box.bottom - dockTop))}px`);
    }

    const resize = new ResizeObserver(sync);
    function bind() {
        const next = document.querySelector('.desktop-canvas');
        if (next === canvas) return;
        resize.disconnect();
        canvas = next;
        dock = canvas?.querySelector('.desktop-dock-zone');
        if (!canvas) return;
        resize.observe(canvas);
        if (dock) resize.observe(dock);
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

    return {boundsFor};
})();
