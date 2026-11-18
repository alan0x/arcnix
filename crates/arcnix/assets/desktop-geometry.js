// Live geometry stays in the WebView until a completed pointer gesture commits it.
const desktopGeometry = (() => {
    const minimumWidth = 480;
    const minimumHeight = 260;
    const controls = 'button, a, input, textarea, select, [contenteditable], [role="button"], [role="tab"]';
    const clamp = (value, minimum, maximum) => Math.min(maximum, Math.max(minimum, value));
    let canvas, windowElement, layout, gesture, liveBounds, observedArea;
    let lastDragEnd = -Infinity;

    function area() {
        const frame = canvas.getBoundingClientRect();
        const dock = canvas.querySelector('.desktop-dock-zone');
        const width = Math.floor(frame.width);
        const height = Math.floor(dock?.offsetHeight ? dock.offsetTop - 8 : frame.height);
        return { width: Math.max(1, width), height: Math.max(1, height) };
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

    function draw(bounds) {
        if (!windowElement) return;
        windowElement.toggleAttribute('data-positioned', Boolean(bounds));
        if (!bounds) return;
        for (const [name, value] of Object.entries(bounds)) {
            windowElement.style.setProperty(`--app-window-${name}`, `${value}px`);
        }
    }

    function drawSaved() {
        if (!layout?.shown || !layout.windowApp || layout.maximized || !canvas) {
            draw(null);
            return;
        }
        draw(liveBounds ? fit(liveBounds, area()) : null);
    }

    function finish(commit) {
        const drag = gesture;
        if (!drag) return;
        gesture = null;
        drag.element.removeAttribute('data-window-dragging');
        if (drag.element.hasPointerCapture(drag.pointer)) drag.element.releasePointerCapture(drag.pointer);
        if (!drag.current) return;
        lastDragEnd = performance.now();
        if (!commit || Object.keys(drag.start).every(key => drag.start[key] === drag.current[key])) {
            drawSaved();
            return;
        }

        const input = document.querySelector('.server-desktop:not(.hidden) .desktop-bounds-input');
        if (!input) {
            drawSaved();
            return;
        }
        liveBounds = drag.current;
        input.value = JSON.stringify({ epoch: drag.epoch, app: drag.app, bounds: liveBounds });
        input.dispatchEvent(new Event('input', { bubbles: true }));
    }

    const observer = new ResizeObserver(() => {
        if (!canvas) return;
        const current = area();
        if (observedArea?.width === current.width && observedArea?.height === current.height) return;
        observedArea = current;
        finish(false);
        drawSaved();
    });

    function sync(next) {
        const nextCanvas = document.querySelector('.desktop-canvas');
        const nextWindow = nextCanvas?.querySelector('.desktop-window');
        const sameWindow = canvas === nextCanvas && windowElement === nextWindow
            && next?.epoch === layout?.epoch && next?.windowApp === layout?.windowApp;
        const sameFrame = sameWindow && next?.shown === layout?.shown && next?.maximized === layout?.maximized
            && next?.fullscreen === layout?.fullscreen;
        if (!sameFrame) finish(false);
        if (nextCanvas !== canvas) {
            observer.disconnect();
            observedArea = null;
            if (nextCanvas) {
                observer.observe(nextCanvas);
                const dock = nextCanvas.querySelector('.desktop-dock-zone');
                if (dock) observer.observe(dock);
            }
        }
        const newBounds = JSON.stringify(next?.bounds) !== JSON.stringify(layout?.bounds);
        if (!sameWindow || newBounds) liveBounds = next?.bounds;
        canvas = nextCanvas;
        windowElement = nextWindow;
        layout = next;
        if (!gesture && (!sameFrame || newBounds)) drawSaved();
    }

    document.addEventListener('pointerdown', event => {
        if (event.button !== 0 || !event.isPrimary || gesture || !layout?.shown || !layout.windowApp || layout.maximized) return;
        const target = event.target;
        if (!(target instanceof Element) || target.closest('.desktop-window') !== windowElement
            || windowElement.classList.contains('hidden') || target.closest(controls)) return;
        const handle = target.closest('.desktop-resize-handle');
        if (!handle && !target.closest('.desktop-window-titlebar')) return;
        const edge = handle ? [...handle.classList].find(name => name.startsWith('edge-'))?.slice(5) : '';
        const limits = area();
        const start = fit({
            x: windowElement.offsetLeft, y: windowElement.offsetTop,
            width: windowElement.offsetWidth, height: windowElement.offsetHeight,
        }, limits);
        gesture = {
            pointer: event.pointerId, element: windowElement, edge, limits, start,
            x: event.clientX, y: event.clientY, epoch: layout.epoch, app: layout.windowApp,
        };
        if (handle) {
            // Edge presses are resize gestures, so the browser must not start a native drag.
            event.preventDefault();
            windowElement.setPointerCapture(event.pointerId);
        }
    });

    document.addEventListener('pointermove', event => {
        const drag = gesture;
        if (!drag || event.pointerId !== drag.pointer) return;
        if (event.buttons === 0) { finish(false); return; }
        const dx = Math.round(event.clientX - drag.x), dy = Math.round(event.clientY - drag.y);
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
            const minWidth = Math.min(minimumWidth, limits.width), minHeight = Math.min(minimumHeight, limits.height);
            const right = start.x + start.width, bottom = start.y + start.height;
            const left = edge.includes('w') ? clamp(start.x + dx, 0, right - minWidth) : start.x;
            const top = edge.includes('n') ? clamp(start.y + dy, 0, bottom - minHeight) : start.y;
            const newRight = edge.includes('e') ? clamp(right + dx, left + minWidth, limits.width) : right;
            const newBottom = edge.includes('s') ? clamp(bottom + dy, top + minHeight, limits.height) : bottom;
            drag.current = { x: left, y: top, width: newRight - left, height: newBottom - top };
        }
        draw(drag.current);
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
