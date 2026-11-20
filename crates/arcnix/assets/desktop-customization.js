(() => {
    const dragType = 'application/x-arcnix-desktop-icon';
    let drag, marker, focusedMenu, dockMenuAnchor;
    let clock, clockLocale, clockCompact, clockTimer, clockSample, clockAnchor;
    let sizeControl;

    function canvasFor(target) {
        if (!(target instanceof Element) || target.closest('[inert],[hidden],.hidden')) return null;
        return target.closest('.desktop-canvas');
    }

    function epoch(canvas) {
        return JSON.parse(canvas.closest('.server-desktop').querySelector('.desktop-motion-state').textContent).epoch;
    }

    function send(canvas, payload) {
        const bridge = canvas.closest('.server-desktop')?.querySelector('.desktop-customization-input');
        if (!bridge) return;
        bridge.value = JSON.stringify({epoch: epoch(canvas), ...payload});
        bridge.dispatchEvent(new Event('input', {bubbles: true}));
    }

    function zone(target) {
        return target.closest('.desktop-dock') ? 'dock' : 'desktop';
    }

    function shortcut(slot) {
        const key = slot?.querySelector('.desktop-item-key');
        return key ? JSON.parse(key.textContent) : null;
    }

    function openContextMenu(canvas, target, x, y) {
        const slot = target.closest('.desktop-icon-slot');
        const menuZone = zone(target);
        dockMenuAnchor = menuZone === 'dock' ? slot?.querySelector('.desktop-dock-app') : null;
        const frame = canvas.getBoundingClientRect();
        send(canvas, {kind: 'context', zone: menuZone, item: shortcut(slot),
            x: Math.round(x - frame.left), y: Math.round(y - frame.top)});
    }

    function clearMarker() {
        marker?.removeAttribute('data-desktop-drop');
        marker = null;
    }

    function clearDrag() {
        clearMarker();
        drag?.slot.removeAttribute('data-desktop-dragging');
        drag = null;
    }

    function dropTarget(event) {
        const canvas = canvasFor(event.target);
        const slot = event.target.closest?.('.desktop-icon-slot');
        if (canvas !== drag.canvas || epoch(canvas) !== drag.epoch || !slot || slot === drag.slot || zone(slot) !== drag.zone) return null;
        const rect = slot.getBoundingClientRect();
        const after = drag.zone === 'desktop' || slot.closest('.dock-side-zone')
            ? event.clientY >= rect.top + rect.height / 2
            : event.clientX >= rect.left + rect.width / 2;
        return {slot, after};
    }

    document.addEventListener('dragstart', event => {
        const slot = event.target.closest?.('.desktop-icon-slot');
        const canvas = canvasFor(slot);
        if (!canvas || !event.dataTransfer) return;
        clearDrag();
        drag = {slot, canvas, epoch: epoch(canvas), zone: zone(slot), item: shortcut(slot)};
        event.dataTransfer.effectAllowed = 'move';
        event.dataTransfer.setData(dragType, String(drag.epoch));
        slot.setAttribute('data-desktop-dragging', '');
    }, true);

    document.addEventListener('dragover', event => {
        if (!drag) return;
        clearMarker();
        event.preventDefault();
        const target = dropTarget(event);
        event.dataTransfer.dropEffect = target ? 'move' : 'none';
        if (!target) return;
        marker = target.slot;
        marker.setAttribute('data-desktop-drop', target.after ? 'after' : 'before');
    }, true);

    document.addEventListener('drop', event => {
        if (!drag) return;
        event.preventDefault();
        const target = dropTarget(event);
        if (target && event.dataTransfer.getData(dragType) === String(drag.epoch)) {
            send(drag.canvas, {kind: 'reorder', zone: drag.zone, item: drag.item, anchor: shortcut(target.slot), after: target.after});
        }
        clearDrag();
    }, true);
    document.addEventListener('dragend', clearDrag, true);
    window.addEventListener('blur', clearDrag);

    document.addEventListener('wheel', event => {
        const grid = event.target.closest?.('.desktop-shortcuts');
        if (!grid || event.ctrlKey || event.deltaX || !event.deltaY || grid.scrollWidth <= grid.clientWidth) return;
        const scale = event.deltaMode === 1 ? 20 : event.deltaMode === 2 ? grid.clientWidth : 1;
        grid.scrollLeft += event.deltaY * scale;
        event.preventDefault();
    }, {passive: false});

    document.addEventListener('contextmenu', event => {
        if (!(event.target instanceof Element) || !event.target.closest('.server-desktop,.desktop-modal-backdrop')) return;
        event.preventDefault();
        const canvas = canvasFor(event.target);
        if (!canvas || event.target.closest('.desktop-window,.desktop-context-menu')) return;
        const target = event.target.closest('.desktop-menu-backdrop')
            ? document.elementsFromPoint(event.clientX,event.clientY).find(element => element !== event.target && canvas.contains(element))
            : event.target;
        if (!target || target.closest('.desktop-window')) { send(canvas, {kind: 'dismiss'}); return; }
        openContextMenu(canvas, target, event.clientX, event.clientY);
    }, true);

    document.addEventListener('keydown', event => {
        const desktopFile = event.target.closest?.('.desktop-shortcut.desktop-file');
        if (desktopFile && event.key === 'Delete' && !event.ctrlKey && !event.metaKey && !event.altKey) {
            const canvas = canvasFor(desktopFile);
            const item = shortcut(desktopFile.closest('.desktop-icon-slot'));
            if (canvas && item?.kind === 'file') {
                event.preventDefault(); event.stopImmediatePropagation();
                if (!event.repeat) send(canvas, {kind:'remove_file', name:item.id, permanent:event.shiftKey});
                return;
            }
        }
        const menu = document.querySelector('.server-desktop:not(.hidden) .desktop-context-menu');
        if (!menu) {
            const slot = event.target.closest?.('.desktop-icon-slot');
            const canvas = canvasFor(slot);
            if (!canvas || event.ctrlKey || event.altKey || event.metaKey
                || (event.key !== 'ContextMenu' && !(event.shiftKey && event.key === 'F10'))) return;
            event.preventDefault();
            event.stopImmediatePropagation();
            if (event.repeat) return;
            const anchor = event.target.getBoundingClientRect();
            openContextMenu(canvas, event.target, anchor.left, anchor.bottom);
            return;
        }
        const palette = event.target.closest?.('.file-color-menu');
        const horizontal = palette && ['ArrowLeft', 'ArrowRight'].includes(event.key);
        if (event.key === 'Escape') {
            send(menu.closest('.desktop-canvas'), {kind: 'dismiss'});
        } else if (menu.contains(event.target) && (horizontal || ['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key))) {
            const buttons = [...(horizontal ? palette : menu).querySelectorAll('button:not(:disabled)')];
            if (!buttons.length) return;
            const index = buttons.indexOf(document.activeElement);
            const back = event.key === 'ArrowUp' || event.key === 'ArrowLeft';
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
                : (index + (back ? -1 : 1) + buttons.length) % buttons.length;
            buttons[next].focus();
        } else return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    function updateClock() {
        const panel = document.querySelector('.desktop-panel-clock');
        const current = canvasFor(panel) && !document.hidden ? panel : null;
        const locale = current?.querySelector('.clock-zh') ? 'zh-CN' : 'en-US';
        const compact = !!current?.closest('.dock-side-zone');
        const sample = document.querySelector('.desktop-clock-sample')?.textContent;
        const changed = sample !== clockSample;
        if (changed) {
            clockSample = sample;
            const [, seconds, offset] = (sample || '').split(',').map(Number);
            const wallTime = seconds * 1000 + offset * 60_000;
            clockAnchor = Number.isFinite(new Date(wallTime).getTime())
                ? {wallTime, offset, received: performance.now()}
                : null;
        }
        if (!changed && current === clock && locale === clockLocale && compact === clockCompact) return;
        clock = current;
        clockLocale = locale;
        clockCompact = compact;
        clearTimeout(clockTimer);
        if (!clock) return;

        const time = clock.querySelector('.desktop-panel-time');
        const date = clock.querySelector('.desktop-panel-date');
        const tooltip = clock.querySelector('.desktop-clock-tooltip');
        if (!time || !date || !tooltip) {
            clock = null;
            return;
        }
        if (!clockAnchor) {
            time.textContent = '—';
            date.textContent = '';
            tooltip.textContent = '';
            clock.removeAttribute('title');
            return;
        }

        const {wallTime, offset, received} = clockAnchor;
        const hours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, '0');
        const minutes = String(Math.abs(offset) % 60).padStart(2, '0');
        const zone = `UTC${offset < 0 ? '-' : '+'}${hours}:${minutes}`;
        if (compact) clock.removeAttribute('title');
        else clock.title = `${clock.getAttribute('aria-label')} (${zone})`;
        // The sample is already shifted to the server's civil time.
        const fullTime = new Intl.DateTimeFormat(locale, {timeZone: 'UTC', hour: 'numeric', minute: '2-digit', second: '2-digit'});
        const fullDate = new Intl.DateTimeFormat(locale, {timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric'});
        const timeFormat = compact
            ? new Intl.DateTimeFormat(locale, {timeZone: 'UTC', hour: '2-digit', minute: '2-digit', hourCycle: 'h23'})
            : fullTime;
        const dateFormat = compact
            ? new Intl.DateTimeFormat(locale, {timeZone: 'UTC', month: '2-digit', day: '2-digit'})
            : fullDate;
        function tick() {
            const now = new Date(wallTime + (performance.now() - received));
            const serverDate = dateFormat.format(now);
            const serverTime = timeFormat.format(now);
            if (time.textContent !== serverTime) time.textContent = serverTime;
            if (date.textContent !== serverDate) date.textContent = serverDate;
            if (compact) tooltip.textContent = `${fullTime.format(now)} · ${fullDate.format(now)} (${zone})`;
            clockTimer = setTimeout(tick, 1000 - now.getUTCMilliseconds());
        }
        tick();
    }

    function showSizeValue({slider, value}) {
        const size = slider.valueAsNumber;
        const min = Number(slider.min);
        const max = Number(slider.max);
        value.textContent = `${size} px`;
        slider.style.setProperty('--dock-size-progress', `${(size - min) / (max - min) * 100}%`);
    }

    function mountSizeControl(host, dock, bridge, min, max) {
        const slider = document.createElement('input');
        slider.className = 'dock-size-slider';
        slider.type = 'range';
        slider.min = min;
        slider.max = max;
        slider.step = 1;
        const value = document.createElement('span');
        value.className = 'dock-size-value';
        host.append(slider, value);

        const control = {host, dock, slider, value};
        slider.addEventListener('input', () => {
            dock.style.setProperty('--dock-preview', `${slider.value}px`);
            showSizeValue(control);
        });
        slider.addEventListener('change', () => {
            if (slider.valueAsNumber === control.saved) {
                dock.style.removeProperty('--dock-preview');
                return;
            }
            bridge.value = slider.value;
            bridge.dispatchEvent(new Event('input', {bubbles: true}));
        });
        slider.addEventListener('pointercancel', () => {
            slider.value = control.saved;
            showSizeValue(control);
            dock.style.removeProperty('--dock-preview');
        });
        return control;
    }

    function updateSizeControl() {
        const host = document.querySelector('.desktop-window.app-settings.active .dock-size-control');
        if (sizeControl && (sizeControl.host !== host || !canvasFor(host))) {
            sizeControl.dock.style.removeProperty('--dock-preview');
            sizeControl.slider.remove();
            sizeControl.value.remove();
            sizeControl = null;
        }
        const dock = canvasFor(host)?.querySelector('.desktop-dock');
        if (!dock) return;
        const bridge = host.querySelector('.dock-size-bridge');
        const state = host.querySelector('.dock-size-state');
        if (!bridge || !state) return;
        const [min, max, size] = state.textContent.split(',').map(Number);
        if (!sizeControl) sizeControl = mountSizeControl(host, dock, bridge, min, max);
        const {slider} = sizeControl;
        const label = bridge.getAttribute('aria-label') || '';
        if (slider.getAttribute('aria-label') !== label) slider.setAttribute('aria-label', label);
        if (slider.disabled !== bridge.disabled) slider.disabled = bridge.disabled;
        if (sizeControl.saved !== size) {
            sizeControl.saved = size;
            slider.value = size;
            showSizeValue(sizeControl);
            dock.style.removeProperty('--dock-preview');
        }
    }

    function placeDockMenu(menu, canvas, anchor) {
        const frame = canvas.getBoundingClientRect();
        const dock = anchor.closest('.desktop-dock').getBoundingClientRect();
        const icon = anchor.getBoundingClientRect();
        const margin = 8;
        const gap = 8;
        const limit = (value, low, high) => Math.max(low, Math.min(value, high));
        const above = dock.top - frame.top;
        const below = frame.bottom - dock.bottom;
        const left = dock.left - frame.left;
        const right = frame.right - dock.right;
        const horizontalDock = !anchor.closest('.dock-side-zone');
        const side = horizontalDock ? (above >= below ? 'above' : 'below') : (right >= left ? 'right' : 'left');
        menu.dataset.dockMenuSide = side;
        menu.style.setProperty('--dock-menu-max-height', `${Math.max(0, (horizontalDock ? Math.max(above, below) - gap : frame.height) - margin)}px`);
        menu.style.setProperty('--dock-menu-max-width', `${Math.max(0, (horizontalDock ? frame.width - margin : Math.max(left, right) - gap) - margin)}px`);
        const width = menu.offsetWidth;
        const height = menu.offsetHeight;
        const centerX = (icon.left + icon.right) / 2 - frame.left;
        const centerY = (icon.top + icon.bottom) / 2 - frame.top;
        const x = horizontalDock ? limit(centerX - width / 2, margin, frame.width - width - margin)
            : side === 'right' ? dock.right - frame.left + gap : left - gap - width;
        const y = horizontalDock ? (side === 'above' ? above - gap - height : dock.bottom - frame.top + gap)
            : limit(centerY - height / 2, margin, frame.height - height - margin);
        const arrow = horizontalDock ? limit(centerX - x, 14, width - 14) : limit(centerY - y, 14, height - 14);
        menu.style.left = `${x}px`;
        menu.style.top = `${y}px`;
        menu.style.setProperty('--dock-menu-arrow', `${arrow}px`);
    }

    function update() {
        updateClock();
        updateSizeControl();
        for (const slot of document.querySelectorAll('.desktop-icon-slot:not([draggable])')) slot.draggable = true;
        const menu = document.querySelector('.desktop-context-menu');
        const canvas = menu && canvasFor(menu);
        if (!canvas) { focusedMenu = null; return; }
        if (dockMenuAnchor?.isConnected && canvas.contains(dockMenuAnchor) && dockMenuAnchor.getClientRects().length) {
            placeDockMenu(menu, canvas, dockMenuAnchor);
        } else {
            delete menu.dataset.dockMenuSide;
            const [x, y] = menu.querySelector('.desktop-menu-position').textContent.split(',').map(Number);
            menu.style.left = `${Math.max(4, Math.min(x, canvas.clientWidth - menu.offsetWidth - 4))}px`;
            menu.style.top = `${Math.max(4, Math.min(y, canvas.clientHeight - menu.offsetHeight - 4))}px`;
        }
        if (focusedMenu !== menu) {
            focusedMenu = menu;
            menu.querySelector('button:not(:disabled)')?.focus({preventScroll: true});
        }
    }

    new MutationObserver(update).observe(document, {subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['class', 'hidden', 'inert', 'disabled', 'aria-label']});
    document.addEventListener('visibilitychange', updateClock);
    window.addEventListener('resize', update);
})();
