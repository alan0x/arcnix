// Live geometry stays in the WebView until a completed pointer gesture commits it.
const desktopGeometry = (() => {
    const minimumWidth = 480;
    const minimumHeight = 260;
    const titlebarWidthToKeep = 64;
    const titlebarHeightToKeep = 18;
    const controls = 'button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"], .path-location, .files-search-options, .files-tab';
    const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, value));
    let canvas, layout, gesture, observedArea;
    let lastDragEnd = -Infinity;
    const pending = new Map();
    const openingAreas = new Map();

    function area() {
        const frame = canvas.getBoundingClientRect();
        const [left, right, bottom] = Object.values(desktopFrame.dockInsets(canvas))
            .map(size => size ? size + 8 : 0);
        return {
            left, right, bottom,
            width: Math.max(1, Math.floor(frame.width - left - right)),
            height: Math.max(1, Math.floor(frame.height - bottom)),
        };
    }

    function reachable(limits, titlebarHeight) {
        const width = Math.min(titlebarWidthToKeep, limits.width);
        const height = Math.min(titlebarHeightToKeep, limits.height, titlebarHeight);
        return { width, right: limits.width, top: height - titlebarHeight, bottom: limits.height - height };
    }

    function fit(bounds, range) {
        const widthToKeep = Math.min(range.width, bounds.width);
        return {
            ...bounds,
            x: clamp(bounds.x, widthToKeep - bounds.width, range.right - widthToKeep),
            y: clamp(bounds.y, range.top, range.bottom),
        };
    }

    function initialBounds(item, limits) {
        const offset = item.slot * 26;
        const width = Math.min(limits.width, Math.max(minimumWidth, limits.width - Math.max(150, Math.round(limits.width * .13))));
        const height = Math.min(limits.height, Math.max(minimumHeight, limits.height - Math.max(136, Math.round(limits.height * .16))));
        return {
            x: Math.min(22 + offset, limits.width - width),
            y: Math.min(19 + offset, limits.height - height),
            width,
            height,
        };
    }

    function preferredBounds(item, limits) {
        if (!openingAreas.has(item.class)) openingAreas.set(item.class, {width: limits.width, height: limits.height});
        const opening = openingAreas.get(item.class);
        const bounds = pending.get(item.class)?.bounds ?? item.bounds ?? initialBounds(item, opening);
        return { ...bounds, reference: bounds.reference ?? opening };
    }

    function place(bounds, limits, range) {
        const width = Math.min(bounds.width, limits.width);
        const height = Math.min(bounds.height, limits.height);
        const centerX = (bounds.x + bounds.width / 2) / bounds.reference.width;
        const centerY = (bounds.y + bounds.height / 2) / bounds.reference.height;
        return fit({
            x: Math.round(centerX * limits.width - width / 2),
            y: Math.round(centerY * limits.height - height / 2),
            width,
            height,
        }, range);
    }

    function draw(element, bounds, limits) {
        element.toggleAttribute('data-window-overhang',
            bounds.x < 0 || bounds.y < 0 || bounds.x + bounds.width > limits.width || bounds.y + bounds.height > limits.height);
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
        for (const edge of ['left', 'right', 'bottom', 'width']) {
            const property = `--desktop-work-${edge}`;
            const value = limits[edge] + 'px';
            if (canvas.style.getPropertyValue(property) !== value) canvas.style.setProperty(property, value);
        }
        for (const item of layout.windows) {
            const element = canvas.querySelector(`.desktop-window.${item.class}:not(.hidden)`);
            if (!element) continue;
            element.style.zIndex = item.order;
            if (gesture?.element === element && gesture.current) continue;
            const local = pending.get(item.class);
            if (local && JSON.stringify(item.bounds) !== local.original) pending.delete(item.class);
            const bounds = preferredBounds(item, limits);
            const range = reachable(limits, element.querySelector('.desktop-window-titlebar').offsetHeight);
            draw(element, item.maximized
                ? {x: 0, y: 0, width: limits.width, height: limits.height}
                : place(bounds, limits, range), limits);
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
        if (drag.capture.hasPointerCapture(drag.pointer)) drag.capture.releasePointerCapture(drag.pointer);
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

    function updateArea() {
        if (!canvas || !layout?.shown) return;
        const current = area();
        if (observedArea && Object.keys(current).every(key => current[key] === observedArea[key])) return;
        observedArea = current;
        finish(false);
        drawWindows();
    }
    const observer = new ResizeObserver(updateArea);
    const dockPosition = new MutationObserver(updateArea);

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
            dockPosition.disconnect();
            observedArea = null;
            if (nextCanvas) {
                observer.observe(nextCanvas);
                const dock = nextCanvas.querySelector('.desktop-dock-zone');
                if (dock) {
                    observer.observe(dock);
                    dockPosition.observe(dock, {attributes: true, attributeFilter: ['class']});
                }
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
        if (event.button === 0 && target.closest('.files-inactive-draggable')) return;
        raise(element);
        const item = windowLayout(element);
        if (event.button !== 0 || gesture || !layout?.shown || !item
            || element.closest('[inert]') || target.closest(controls)) return;
        const handle = target.closest('.desktop-resize-handle');
        const titlebar = target.closest('.desktop-window-titlebar');
        if (titlebar) canvas.focus({preventScroll: true});
        if (item.maximized || (!handle && !titlebar)) return;
        const edge = handle ? [...handle.classList].find(name => name.startsWith('edge-'))?.slice(5) : '';
        const limits = area();
        const range = reachable(limits, element.querySelector('.desktop-window-titlebar').offsetHeight);
        const start = {
            x: element.offsetLeft, y: element.offsetTop,
            width: element.offsetWidth, height: element.offsetHeight,
        };
        const capture = handle ? element : titlebar;
        gesture = {
            pointer: event.pointerId, element, capture, edge, limits, range, start, preferred: preferredBounds(item, limits),
            x: event.clientX, y: event.clientY, app: item.class, item, original: JSON.stringify(item.bounds),
        };
        // Capture before a fast move reaches the underlying file or desktop content.
        event.preventDefault();
        capture.setPointerCapture(event.pointerId);
    }, true);
    document.addEventListener('click', event => {
        const row = event.target.closest?.('.files-inactive-draggable');
        if (row) raise(row.closest('.desktop-window:not(.hidden)'));
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
            drag.element.setAttribute('data-window-dragging', '');
        }
        const { start, edge, limits, range } = drag;
        if (!edge) {
            drag.current = fit({ ...start, x: start.x + dx, y: start.y + dy }, range);
        } else {
            const minWidth = Math.min(minimumWidth, limits.width);
            const minHeight = Math.min(minimumHeight, limits.height);
            const right = start.x + start.width;
            const bottom = start.y + start.height;
            const left = edge.includes('w') ? clamp(start.x + dx, right - limits.width, Math.min(right - minWidth, limits.width - range.width)) : start.x;
            const top = edge.includes('n') ? clamp(start.y + dy, Math.max(bottom - limits.height, range.top), Math.min(bottom - minHeight, range.bottom)) : start.y;
            const newRight = edge.includes('e') ? clamp(right + dx, Math.max(left + minWidth, range.width), left + limits.width) : right;
            const newBottom = edge.includes('s') ? clamp(bottom + dy, top + minHeight, top + limits.height) : bottom;
            drag.current = { x: left, y: top, width: newRight - left, height: newBottom - top };
        }
        draw(drag.element, drag.current, limits);
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
