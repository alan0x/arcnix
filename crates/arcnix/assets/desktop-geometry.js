// Live geometry stays in the WebView until a completed pointer gesture commits it.
const desktopGeometry = (() => {
    const minimumWidth = 480;
    const minimumHeight = 260;
    const titlebarWidthToKeep = 64;
    const titlebarHeightToKeep = 18;
    const controls = 'button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"], .path-location, .files-search-options, .files-tab';
    const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, value));
    const moveWithin = (start, delta, minimum, maximum) =>
        clamp(start + delta, Math.min(start, minimum), Math.max(start, maximum));
    let canvas, layout, gesture, observedArea, areaFrame;
    let lastDragEnd = -Infinity;
    const pending = new Map();
    const openingAreas = new Map();

    function reachable(limits, titlebarHeight) {
        const width = Math.min(titlebarWidthToKeep, limits.width);
        const height = Math.min(titlebarHeightToKeep, limits.height, titlebarHeight);
        return { width, right: limits.width, top: height - titlebarHeight, bottom: limits.height - height };
    }

    function positionRange(bounds, range) {
        const widthToKeep = Math.min(range.width, bounds.width);
        return {
            left: widthToKeep - bounds.width,
            right: range.right - widthToKeep,
            top: range.top,
            bottom: range.bottom,
        };
    }

    function fit(bounds, range) {
        const position = positionRange(bounds, range);
        return {
            ...bounds,
            x: clamp(bounds.x, position.left, position.right),
            y: clamp(bounds.y, position.top, position.bottom),
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
        return { bounds, reference: bounds.reference ?? opening };
    }

    function projectAxis(position, size, reference, extent, hasSnapshot) {
        if (extent < size) return 0;
        const space = extent - size;
        const originalSpace = reference - size;
        if (hasSnapshot && originalSpace >= 0 && (position < 0 || position > originalSpace)) {
            if (extent < reference) return Math.round(position * space / originalSpace);

            // Reveal a deliberately clipped edge before following the saved center.
            const revealed = position < 0 ? Math.min(0, position + extent - reference) : position;
            if (revealed < 0 || revealed > space) return revealed;
        }
        const center = (position + size / 2) / reference;
        return clamp(Math.round(center * extent - size / 2), 0, space);
    }

    function place({bounds, reference}, limits, range) {
        const width = Math.min(bounds.width, limits.width);
        const height = Math.min(bounds.height, limits.height);
        const hasSnapshot = Boolean(bounds.reference);
        return fit({
            x: projectAxis(bounds.x, bounds.width, reference.width, limits.width, hasSnapshot),
            y: projectAxis(bounds.y, bounds.height, reference.height, limits.height, hasSnapshot),
            width,
            height,
        }, range);
    }

    function draw(element, bounds, limits) {
        // Keep a draggable end when only the minimum titlebar sliver remains.
        // Its hit area must not reflow the controls during a drag.
        const right = bounds.x + bounds.width;
        if (bounds.x < 0 && right <= titlebarWidthToKeep) element.dataset.titlebarGrip = 'right';
        else if (right > limits.width && limits.width - bounds.x <= titlebarWidthToKeep) element.dataset.titlebarGrip = 'left';
        else element.removeAttribute('data-titlebar-grip');
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
        const limits = desktopFrame.workArea(canvas);
        for (const edge of ['left', 'right', 'bottom', 'width']) {
            const property = `--desktop-work-${edge}`;
            const value = limits[edge] + 'px';
            if (canvas.style.getPropertyValue(property) !== value) canvas.style.setProperty(property, value);
        }
        if (layout.overviewCount) {
            const count = layout.overviewCount;
            const columns = Math.max(1, Math.min(count, Math.floor(limits.width / 230),
                Math.ceil(Math.sqrt(count * limits.width / (1.55 * limits.height)))));
            canvas.style.setProperty('--overview-columns', String(columns));
            canvas.style.setProperty('--overview-rows', String(Math.ceil(count / columns)));
            return;
        }
        for (const item of layout.windows) {
            const element = canvas.querySelector(`.desktop-window.${item.class}:not(.hidden)`);
            if (!element) continue;
            element.style.zIndex = item.order;
            if (gesture?.element === element) continue;
            const local = pending.get(item.class);
            if (local) {
                const recorded = JSON.stringify(item.bounds);
                if (recorded === local.saved || !local.previous.has(recorded)) pending.delete(item.class);
            }
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
        if (!layout?.shown || layout.overviewCount || element?.classList.contains('active') || element?.closest('[inert]')) return;
        const item = element && windowLayout(element);
        if (item) send('.desktop-focus-input', item);
    }

    function finish(commit) {
        const drag = gesture;
        if (!drag) return;
        gesture = null;
        drag.element.removeAttribute('data-window-dragging');
        if (drag.capture.hasPointerCapture(drag.pointer)) drag.capture.releasePointerCapture(drag.pointer);
        if (!drag.current) {
            for (const animation of drag.animations) if (animation.playState === 'paused') animation.play();
            drawWindows();
            return;
        }
        lastDragEnd = performance.now();
        if (commit && !Object.keys(drag.start).every(key => drag.start[key] === drag.current[key])) {
            // Save the displayed rectangle and the workspace it belongs to as one snapshot.
            const bounds = { ...drag.current, reference: {width: drag.limits.width, height: drag.limits.height} };
            const earlier = pending.get(drag.app);
            const previous = earlier?.previous ?? new Set();
            previous.add(drag.original);
            if (earlier) previous.add(earlier.saved);
            pending.set(drag.app, { bounds, saved: JSON.stringify(bounds), previous });
            send('.desktop-bounds-input', drag.item, { bounds });
        }
        drawWindows();
    }

    function updateArea() {
        if (!canvas || !layout?.shown) return;
        const current = desktopFrame.workArea(canvas);
        if (observedArea && Object.keys(current).every(key => current[key] === observedArea[key])) return;
        observedArea = current;
        finish(false);
        drawWindows();
    }
    function scheduleAreaUpdate() {
        if (!areaFrame) areaFrame = requestAnimationFrame(() => {
            areaFrame = 0;
            updateArea();
        });
    }
    const observer = new ResizeObserver(scheduleAreaUpdate);
    const dockPosition = new MutationObserver(scheduleAreaUpdate);

    function sync(next) {
        if (next?.epoch !== layout?.epoch) openingAreas.clear();
        const nextCanvas = document.querySelector('.desktop-canvas');
        const reset = next?.epoch !== layout?.epoch || nextCanvas !== canvas;
        const changedFrame = reset || !!next?.overviewCount !== !!layout?.overviewCount || !next?.shown || next.fullscreen !== layout?.fullscreen;
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
                    observer.observe(dock, {box: 'border-box'});
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
        if (event.button !== 0 || gesture || !layout?.shown || layout.overviewCount || !item
            || element.closest('[inert]') || target.closest(controls)) return;
        const handle = target.closest('.desktop-resize-handle');
        const titlebar = target.closest('.desktop-window-titlebar');
        if (titlebar) canvas.focus({preventScroll: true});
        if (item.maximized || (!handle && !titlebar) || !element.offsetParent) return;
        const edge = handle ? [...handle.classList].find(name => name.startsWith('edge-'))?.slice(5) : '';
        const limits = desktopFrame.workArea(canvas);
        const range = reachable(limits, element.querySelector('.desktop-window-titlebar').offsetHeight);
        const animations = element.getAnimations().filter(animation => animation.playState === 'running');
        for (const animation of animations) animation.pause();
        const rendered = element.getBoundingClientRect();
        const layer = element.offsetParent.getBoundingClientRect();
        const start = {
            x: Math.round(rendered.left - layer.left), y: Math.round(rendered.top - layer.top),
            width: Math.round(rendered.width), height: Math.round(rendered.height),
        };
        const capture = handle ? element : titlebar;
        gesture = {
            pointer: event.pointerId, element, capture, edge, limits, range, start, animations,
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
            const position = positionRange(start, range);
            drag.current = {
                ...start,
                x: moveWithin(start.x, dx, position.left, position.right),
                y: moveWithin(start.y, dy, position.top, position.bottom),
            };
        } else {
            const minWidth = Math.min(minimumWidth, limits.width, start.width);
            const minHeight = Math.min(minimumHeight, limits.height, start.height);
            const right = start.x + start.width;
            const bottom = start.y + start.height;
            const farLeft = Math.max(right - limits.width, limits.left ? 0 : -Infinity);
            const left = edge.includes('w') ? moveWithin(start.x, dx, farLeft, Math.min(right - minWidth, limits.width - range.width)) : start.x;
            const top = edge.includes('n') ? moveWithin(start.y, dy, Math.max(bottom - limits.height, range.top), Math.min(bottom - minHeight, range.bottom)) : start.y;
            const farRight = Math.min(left + limits.width, limits.right ? limits.width : Infinity);
            const farBottom = Math.min(top + limits.height, limits.bottom ? limits.height : Infinity);
            const newRight = edge.includes('e') ? moveWithin(right, dx, Math.max(left + minWidth, range.width), farRight) : right;
            const newBottom = edge.includes('s') ? moveWithin(bottom, dy, top + minHeight, farBottom) : bottom;
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
