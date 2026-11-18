(() => {
    const dragType = 'application/x-arcnix-desktop-icon';
    let drag, marker, focusedMenu;
    let clock, clockLocale, clockTimer, clockSample, clockAnchor;
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
        const after = drag.zone === 'desktop'
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
        const canvas = canvasFor(event.target);
        if (!canvas || event.target.closest('.desktop-window,.desktop-context-menu')) return;
        event.preventDefault();
        const rect = canvas.getBoundingClientRect();
        const target = event.target.closest('.desktop-menu-backdrop')
            ? document.elementsFromPoint(event.clientX,event.clientY).find(element => element !== event.target && canvas.contains(element))
            : event.target;
        if (!target || target.closest('.desktop-window')) { send(canvas, {kind: 'dismiss'}); return; }
        send(canvas, {kind: 'context', zone: zone(target), item: shortcut(target.closest('.desktop-icon-slot')),
            x: Math.round(event.clientX - rect.left), y: Math.round(event.clientY - rect.top)});
    }, true);

    document.addEventListener('keydown', event => {
        const menu = document.querySelector('.server-desktop:not(.hidden) .desktop-context-menu');
        if (!menu) return;
        if (event.key === 'Escape') {
            send(menu.closest('.desktop-canvas'), {kind: 'dismiss'});
        } else if (['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key) && menu.contains(event.target)) {
            const buttons = [...menu.querySelectorAll('button:not(:disabled)')];
            if (!buttons.length) return;
            const index = buttons.indexOf(document.activeElement);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
                : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
            buttons[next].focus();
        } else return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    function updateClock() {
        const panel = document.querySelector('.desktop-panel-clock');
        const current = canvasFor(panel) && !document.hidden ? panel : null;
        const locale = current?.querySelector('.clock-zh') ? 'zh-CN' : 'en-US';
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
        if (!changed && current === clock && locale === clockLocale) return;
        clock = current;
        clockLocale = locale;
        clearTimeout(clockTimer);
        if (!clock) return;

        const time = clock.querySelector('.desktop-panel-time');
        const date = clock.querySelector('.desktop-panel-date');
        if (!time || !date) {
            clock = null;
            return;
        }
        if (!clockAnchor) {
            time.textContent = '—';
            date.textContent = '';
            clock.removeAttribute('title');
            return;
        }

        const {wallTime, offset, received} = clockAnchor;
        const hours = String(Math.floor(Math.abs(offset) / 60)).padStart(2, '0');
        const minutes = String(Math.abs(offset) % 60).padStart(2, '0');
        clock.title = `${clock.getAttribute('aria-label')} (UTC${offset < 0 ? '-' : '+'}${hours}:${minutes})`;
        // The sample is already shifted to the server's civil time.
        const timeFormat = new Intl.DateTimeFormat(locale, {timeZone: 'UTC', hour: 'numeric', minute: '2-digit', second: '2-digit'});
        const dateFormat = new Intl.DateTimeFormat(locale, {timeZone: 'UTC', year: 'numeric', month: 'numeric', day: 'numeric'});
        function tick() {
            const now = new Date(wallTime + (performance.now() - received));
            const serverDate = dateFormat.format(now);
            time.textContent = timeFormat.format(now);
            if (date.textContent !== serverDate) date.textContent = serverDate;
            clockTimer = setTimeout(tick, 1000 - now.getUTCMilliseconds());
        }
        tick();
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
            value.textContent = `${slider.value} px`;
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
            value.textContent = `${control.saved} px`;
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
        const {slider, value} = sizeControl;
        const label = bridge.getAttribute('aria-label') || '';
        if (slider.getAttribute('aria-label') !== label) slider.setAttribute('aria-label', label);
        if (slider.disabled !== bridge.disabled) slider.disabled = bridge.disabled;
        if (sizeControl.saved !== size) {
            sizeControl.saved = size;
            slider.value = size;
            if (value.textContent !== `${size} px`) value.textContent = `${size} px`;
            dock.style.removeProperty('--dock-preview');
        }
    }

    function update() {
        updateClock();
        updateSizeControl();
        for (const slot of document.querySelectorAll('.desktop-icon-slot:not([draggable])')) slot.draggable = true;
        const menu = document.querySelector('.desktop-context-menu');
        const canvas = menu && canvasFor(menu);
        if (!canvas) { focusedMenu = null; return; }
        const [x, y] = menu.querySelector('.desktop-menu-position').textContent.split(',').map(Number);
        menu.style.left = `${Math.max(4, Math.min(x, canvas.clientWidth - menu.offsetWidth - 4))}px`;
        menu.style.top = `${Math.max(4, Math.min(y, canvas.clientHeight - menu.offsetHeight - 4))}px`;
        if (focusedMenu !== menu) {
            focusedMenu = menu;
            menu.querySelector('button')?.focus({preventScroll: true});
        }
    }

    new MutationObserver(update).observe(document, {subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['class', 'hidden', 'inert', 'disabled', 'aria-label']});
    document.addEventListener('visibilitychange', updateClock);
    window.addEventListener('resize', update);
})();
