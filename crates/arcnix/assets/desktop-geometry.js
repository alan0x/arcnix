// Live geometry stays in the WebView until a completed pointer gesture commits it.
const desktopGeometry = (() => {
    const minimumWidth = 480;
    const minimumHeight = 260;
    const controls = 'button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"], .path-location, .explorer-search-options';
    const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, value));
    let canvas, layout, gesture, observedArea;
    let lastDragEnd = -Infinity;
    const pending = new Map();

    function area() {
        const frame = canvas.getBoundingClientRect();
        const dock = canvas.querySelector('.desktop-dock-zone');
        return {
            width: Math.max(1, Math.floor(frame.width)),
            height: Math.max(1, Math.floor(dock?.offsetHeight ? dock.offsetTop - 8 : frame.height)),
        };
    }

    function fit(bounds, limits) {
        const width = clamp(bounds.width, Math.min(minimumWidth, limits.width), limits.width);
        const height = clamp(bounds.height, Math.min(minimumHeight, limits.height), limits.height);
        return {
            width, height,
            x: clamp(bounds.x, 0, limits.width - width),
            y: clamp(bounds.y, 0, limits.height - height),
        };
    }

    function initialBounds(slot, limits) {
        const offset = slot * 26;
        return {
            x: 22 + offset,
            y: 19 + offset,
            width: limits.width - Math.max(150, Math.round(limits.width * .13)),
            height: limits.height - Math.max(136, Math.round(limits.height * .16)),
        };
    }

    function draw(element, bounds) {
        element.setAttribute('data-positioned', '');
        for (const [name, value] of Object.entries(bounds)) {
            element.style.setProperty(`--app-window-${name}`, `${value}px`);
        }
    }

    function windowLayout(element) {
        return layout?.windows?.find(item => element.classList.contains(item.class));
    }

    function drawWindows() {
        if (!layout?.shown || !canvas) return;
        const limits = area();
        for (const item of layout.windows) {
            const element = canvas.querySelector(`.desktop-window.${item.class}:not(.hidden)`);
            if (!element) continue;
            element.style.zIndex = item.order;
            if (gesture?.element === element && gesture.current) continue;
            const local = pending.get(item.id);
            if (local && JSON.stringify(item.bounds) !== local.original) pending.delete(item.id);
            const bounds = pending.get(item.id)?.bounds ?? item.bounds ?? initialBounds(item.slot, limits);
            draw(element, item.maximized ? { x: 0, y: 0, ...limits } : fit(bounds, limits));
        }
    }

    function send(selector, app, fields) {
        const input = canvas?.closest('.server-desktop:not(.hidden)')?.querySelector(selector);
        if (!input) return;
        input.value = JSON.stringify({ epoch: layout.epoch, app, ...fields });
        input.dispatchEvent(new Event('input', { bubbles: true }));
    }

    function raise(element) {
        if (!layout?.shown || element?.classList.contains('active') || element?.closest('[inert]')) return;
        const item = element && windowLayout(element);
        if (item) send('.desktop-focus-input', item.id);
    }

    function finish(commit) {
        const drag = gesture;
        if (!drag) return;
        gesture = null;
        drag.element.removeAttribute('data-window-dragging');
        if (drag.element.hasPointerCapture(drag.pointer)) drag.element.releasePointerCapture(drag.pointer);
        if (!drag.current) return;
        lastDragEnd = performance.now();
        if (commit && !Object.keys(drag.start).every(key => drag.start[key] === drag.current[key])) {
            pending.set(drag.app, { bounds: drag.current, original: drag.original });
            send('.desktop-bounds-input', drag.app, { bounds: drag.current });
        }
        drawWindows();
    }

    const observer = new ResizeObserver(() => {
        if (!canvas || !layout?.shown) return;
        const current = area();
        if (observedArea?.width === current.width && observedArea?.height === current.height) return;
        observedArea = current;
        finish(false);
        drawWindows();
    });

    function sync(next) {
        const nextCanvas = document.querySelector('.desktop-canvas');
        const reset = next?.epoch !== layout?.epoch || nextCanvas !== canvas;
        const changedFrame = reset || !next?.shown || next.fullscreen !== layout?.fullscreen;
        if (changedFrame || (gesture && !next.windows.some(item => item.id === gesture.app && !item.maximized))) {
            finish(false);
        }
        if (reset || !next?.shown) pending.clear();
        else for (const app of pending.keys()) {
            if (!next.windows.some(item => item.id === app)) pending.delete(app);
        }
        if (nextCanvas !== canvas) {
            observer.disconnect();
            observedArea = null;
            if (nextCanvas) {
                observer.observe(nextCanvas);
                const dock = nextCanvas.querySelector('.desktop-dock-zone');
                if (dock) observer.observe(dock);
            }
        }
        canvas = nextCanvas;
        layout = next;
        drawWindows();
    }

    document.addEventListener('focusin', event => {
        raise(event.target.closest?.('.desktop-window:not(.hidden)'));
    }, true);

    document.addEventListener('pointerdown', event => {
        const target = event.target;
        if (!(target instanceof Element) || !event.isPrimary || ![0, 2].includes(event.button)) return;
        const element = target.closest('.desktop-window:not(.hidden)');
        if (!element) return;
        raise(element);
        const item = windowLayout(element);
        if (event.button !== 0 || gesture || !layout?.shown || !item || item.maximized
            || element.closest('[inert]') || target.closest(controls)) return;
        const handle = target.closest('.desktop-resize-handle');
        if (!handle && !target.closest('.desktop-window-titlebar')) return;
        const edge = handle ? [...handle.classList].find(name => name.startsWith('edge-'))?.slice(5) : '';
        const limits = area();
        const start = fit({
            x: element.offsetLeft, y: element.offsetTop,
            width: element.offsetWidth, height: element.offsetHeight,
        }, limits);
        gesture = {
            pointer: event.pointerId, element, edge, limits, start,
            x: event.clientX, y: event.clientY, app: item.id, original: JSON.stringify(item.bounds),
        };
        if (handle) {
            // Edge presses are resize gestures, so the browser must not start a native drag.
            event.preventDefault();
            element.setPointerCapture(event.pointerId);
        }
    }, true);

    document.addEventListener('pointermove', event => {
        const drag = gesture;
        if (!drag || event.pointerId !== drag.pointer) return;
        if (event.buttons === 0) { finish(false); return; }
        const dx = Math.round(event.clientX - drag.x);
        const dy = Math.round(event.clientY - drag.y);
        if (!drag.current && Math.max(Math.abs(dx), Math.abs(dy)) < 4) return;
        if (!drag.current) {
            drag.element.getAnimations().forEach(animation => animation.cancel());
            drag.element.setPointerCapture(event.pointerId);
            drag.element.setAttribute('data-window-dragging', '');
        }
        const { start, edge, limits } = drag;
        if (!edge) {
            drag.current = fit({ ...start, x: start.x + dx, y: start.y + dy }, limits);
        } else {
            const minWidth = Math.min(minimumWidth, limits.width);
            const minHeight = Math.min(minimumHeight, limits.height);
            const right = start.x + start.width;
            const bottom = start.y + start.height;
            const left = edge.includes('w') ? clamp(start.x + dx, 0, right - minWidth) : start.x;
            const top = edge.includes('n') ? clamp(start.y + dy, 0, bottom - minHeight) : start.y;
            const newRight = edge.includes('e') ? clamp(right + dx, left + minWidth, limits.width) : right;
            const newBottom = edge.includes('s') ? clamp(bottom + dy, top + minHeight, limits.height) : bottom;
            drag.current = { x: left, y: top, width: newRight - left, height: newBottom - top };
        }
        draw(drag.element, drag.current);
        event.preventDefault();
    });

    document.addEventListener('pointerup', event => { if (event.pointerId === gesture?.pointer) finish(true); });
    document.addEventListener('pointercancel', event => { if (event.pointerId === gesture?.pointer) finish(false); });
    document.addEventListener('lostpointercapture', event => { if (event.pointerId === gesture?.pointer) finish(false); });
    window.addEventListener('blur', () => finish(false));
    document.addEventListener('keydown', event => {
        if (event.key === 'Escape' && gesture?.current) {
            finish(false);
            event.preventDefault();
            event.stopImmediatePropagation();
        }
    }, true);

    return { sync, movedRecently: () => performance.now() - lastDragEnd < 400 };
})();
