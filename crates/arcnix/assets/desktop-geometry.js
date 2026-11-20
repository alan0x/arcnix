// Live geometry stays in the WebView until a completed pointer gesture commits it.
const desktopGeometry = (() => {
    const minimumWidth = 480;
    const minimumHeight = 260;
    const controls = 'button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"], .path-location, .explorer-search-options, .files-tab';
    const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, value));
    let canvas, layout, gesture, observedArea;
    let lastDragEnd = -Infinity;
    const pending = new Map();
    const openingAreas = new Map();

    function area() {
        const frame = canvas.getBoundingClientRect();
        const dock = canvas.querySelector('.desktop-dock-zone');
        return {
            width: Math.max(1, Math.floor(frame.width)),
            height: Math.max(1, Math.floor(dock?.offsetHeight ? dock.offsetTop - 8 : frame.height)),
        };
    }

    function fit(bounds, limits) {
        return {
            ...bounds,
            x: clamp(bounds.x, 0, Math.max(0, limits.width - bounds.width)),
            y: clamp(bounds.y, 0, Math.max(0, limits.height - bounds.height)),
        };
    }

    function initialBounds(item, limits) {
        const offset = item.slot * 26;
        return fit({
            x: 22 + offset,
            y: 19 + offset,
            width: Math.min(limits.width, Math.max(minimumWidth, limits.width - Math.max(150, Math.round(limits.width * .13)))),
            height: Math.min(limits.height, Math.max(minimumHeight, limits.height - Math.max(136, Math.round(limits.height * .16)))),
        }, limits);
    }

    function preferredBounds(item, limits) {
        if (!openingAreas.has(item.class)) openingAreas.set(item.class, limits);
        const opening = openingAreas.get(item.class);
        const bounds = pending.get(item.class)?.bounds ?? item.bounds ?? initialBounds(item, opening);
        return { ...bounds, reference: bounds.reference ?? opening };
    }

    function place(bounds, limits) {
        const width = Math.min(bounds.width, limits.width);
        const height = Math.min(bounds.height, limits.height);
        const centerX = (bounds.x + bounds.width / 2) / bounds.reference.width;
        const centerY = (bounds.y + bounds.height / 2) / bounds.reference.height;
        return fit({
            x: Math.round(centerX * limits.width - width / 2),
            y: Math.round(centerY * limits.height - height / 2),
            width,
            height,
        }, limits);
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
        if (!layout?.shown || !canvas?.offsetWidth || !canvas.offsetHeight) return;
        const limits = area();
        const workWidth = limits.width + 'px';
        if (canvas.style.getPropertyValue('--desktop-work-width') !== workWidth) {
            canvas.style.setProperty('--desktop-work-width', workWidth);
        }
        for (const item of layout.windows) {
            const element = canvas.querySelector(`.desktop-window.${item.class}:not(.hidden)`);
            if (!element) continue;
            element.style.zIndex = item.order;
            if (gesture?.element === element && gesture.current) continue;
            const local = pending.get(item.class);
            if (local && JSON.stringify(item.bounds) !== local.original) pending.delete(item.class);
            const bounds = preferredBounds(item, limits);
            draw(element, item.maximized ? { x: 0, y: 0, ...limits } : place(bounds, limits));
        }
    }

    function send(selector, item, fields) {
        const input = canvas?.closest('.server-desktop:not(.hidden)')?.querySelector(selector);
        if (!input) return;
        input.value = JSON.stringify({ epoch: layout.epoch, app: item.id, window: item.window, ...fields });
        input.dispatchEvent(new Event('input', { bubbles: true }));
    }

    function raise(element) {
        if (!layout?.shown || element?.classList.contains('active') || element?.closest('[inert]')) return;
        const item = element && windowLayout(element);
        if (item) send('.desktop-focus-input', item);
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
            // Only an explicitly moved or resized axis replaces its preferred geometry.
            const bounds = { ...drag.preferred, reference: { ...drag.preferred.reference } };
            for (const [position, size] of [['x', 'width'], ['y', 'height']]) {
                if (drag.current[position] !== drag.start[position] || drag.current[size] !== drag.start[size]) {
                    bounds[position] = drag.current[position];
                    bounds[size] = drag.current[size];
                    bounds.reference[size] = drag.limits[size];
                }
            }
            pending.set(drag.app, { bounds, original: drag.original });
            send('.desktop-bounds-input', drag.item, { bounds });
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
        if (next?.epoch !== layout?.epoch) openingAreas.clear();
        const nextCanvas = document.querySelector('.desktop-canvas');
        const reset = next?.epoch !== layout?.epoch || nextCanvas !== canvas;
        const changedFrame = reset || !next?.shown || next.fullscreen !== layout?.fullscreen;
        if (changedFrame || (gesture && !next.windows.some(item => item.class === gesture.app && !item.maximized))) {
            finish(false);
        }
        if (reset || !next?.shown) pending.clear();
        else for (const app of pending.keys()) {
            if (!next.windows.some(item => item.class === app)) pending.delete(app);
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
            pointer: event.pointerId, element, edge, limits, start, preferred: preferredBounds(item, limits),
            x: event.clientX, y: event.clientY, app: item.class, item, original: JSON.stringify(item.bounds),
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
