const arcnixKeys = (() => {
    const desktop = () => document.querySelector('.server-desktop:not(.hidden)');
    function state(root = desktop()) {
        const node = root?.querySelector('.desktop-shortcut-state');
        return node ? JSON.parse(node.textContent) : null;
    }
    function binding(action, fallback) {
        const bindings = state()?.bindings;
        return bindings ? bindings[action] : fallback;
    }
    function matches(event, action, fallback) {
        return window.arcnixChordMatches(event, binding(action, fallback));
    }
    return {desktop, state, binding, matches};
})();

(() => {
    // Window selection has its own focus loop; none of these keys should reach
    // a terminal, document, or the desktop's configurable shortcuts.
    let overviewLayer;
    const overviewInert = new Set();
    function focusOverview() {
        const layer = document.querySelector('.server-desktop:not(.hidden) .window-overview');
        if (layer || overviewInert.size) {
            for (const node of overviewInert) if (!layer?.contains(node) || !node.closest('.overview-tile')) {
                node.inert = false;
                overviewInert.delete(node);
            }
            for (const node of layer?.querySelectorAll('.overview-tile > :is(.desktop-window-titlebar,.desktop-window-content)') || []) {
                if (!node.inert) { node.inert = true; overviewInert.add(node); }
            }
        }
        if (layer === overviewLayer) return;
        const previous = overviewLayer;
        overviewLayer = layer;
        if (layer) (layer.querySelector('.overview-current .overview-select') || layer.querySelector('.overview-select'))?.focus({preventScroll: true});
        else if (previous?.isConnected && document.activeElement === document.body) {
            previous.closest('.desktop-canvas')?.querySelector('.desktop-dock-app.active')?.focus({preventScroll: true});
        }
    }
    new MutationObserver(focusOverview).observe(document, {
        subtree: true, childList: true, attributes: true, attributeFilter: ['class'],
    });
    document.addEventListener('click', event => {
        if (event.target.matches?.('.window-overview,.overview-heading')) {
            event.target.closest('.window-overview').querySelector('.overview-exit')?.click();
        }
    });
    window.addEventListener('keydown', event => {
        const layer = overviewLayer;
        if (!layer || !layer.closest('.desktop-canvas')?.contains(event.target) ||
            layer.closest('[inert]') || event.isComposing) return;
        const tiles = [...layer.querySelectorAll('.overview-select')];
        const index = tiles.indexOf(document.activeElement);
        const columns = Number(getComputedStyle(layer).getPropertyValue('--overview-columns')) || 1;
        const step = {ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns,
            Tab: event.shiftKey ? -1 : 1}[event.key];
        if (event.key === 'Escape') layer.querySelector('.overview-exit')?.click();
        else if (step !== undefined && tiles.length) tiles[(Math.max(index, 0) + step + tiles.length) % tiles.length]?.focus();
        else if ((event.key === 'Enter' || event.key === ' ') && event.target.matches('.overview-select,.overview-exit')) {
            if (!event.repeat) event.target.click();
        } else return;
        event.preventDefault();
        event.stopImmediatePropagation();
    }, true);

    const recordButton = '.desktop-shortcut-record';
    const editableFocus = 'input, textarea, select, [contenteditable]';
    const protectedFocus = [
        '[inert]', '.terminal-module', '.terminal-controls', '.terminal-context-menu',
        '[role=dialog]', '[role=menu]',
    ].join(',');
    let recording;
    let origin;

    function stopRecording() {
        if (!recording) return;
        const root = arcnixKeys.desktop();
        if (root) send(root, arcnixKeys.state(root).epoch, {kind: 'recording', active: false});
        recording?.removeAttribute('data-recording');
        recording?.removeAttribute('aria-pressed');
        recording = null;
    }

    function send(desktop, epoch, intent) {
        const input = desktop.querySelector('.desktop-shortcut-input');
        if (!input) return;
        input.value = JSON.stringify({ epoch, intent });
        input.dispatchEvent(new Event('input', { bubbles: true }));
    }

    document.addEventListener('click', event => {
        const button = event.target.closest?.(recordButton);
        if (!button || button.disabled) return;
        const wasRecording = recording === button;
        stopRecording();
        if (wasRecording) return;
        recording = button;
        const root = arcnixKeys.desktop();
        origin = arcnixKeys.state(root);
        send(root, arcnixKeys.state(root).epoch, {kind: 'recording', active: true});
        button.setAttribute('data-recording', '');
        button.setAttribute('aria-pressed', 'true');
        button.focus({ preventScroll: true });
    }, true);
    document.addEventListener('focusout', event => {
        if (event.target === recording) stopRecording();
    }, true);
    window.addEventListener('blur', stopRecording);

    const focusSelector = 'a[href], button, input, textarea, select, [tabindex], [contenteditable]';
    function moveFocus(canvas, direction) {
        const targets = [...canvas.querySelectorAll(focusSelector)].filter(element =>
            element.tabIndex >= 0 && !element.matches(':disabled') && !element.closest('[inert]')
            && element.getClientRects().length && getComputedStyle(element).visibility === 'visible');
        const index = targets.indexOf(document.activeElement);
        const next = index < 0 ? (direction === 1 ? 0 : targets.length - 1)
            : (index + direction + targets.length) % targets.length;
        targets[next]?.focus();
    }

    document.addEventListener('keydown', event => {
        if (event.defaultPrevented || event.isComposing || event.getModifierState('AltGraph')) return;
        const target = event.target;
        if (!(target instanceof Element)) return;
        const desktop = arcnixKeys.desktop();
        const state = arcnixKeys.state(desktop);
        if (!state?.active) return;
        const chord = window.arcnixShortcutChord(event);

        if (recording && target === recording) {
            if (chord === 'Tab' || chord === 'Shift+Tab') { stopRecording(); return; }
            if (['Control', 'Alt', 'Shift', 'Meta'].includes(event.key)) return;
            event.preventDefault();
            event.stopImmediatePropagation();
            if (event.repeat) return;
            const action = JSON.parse(recording.closest('.desktop-key-controls').querySelector('.desktop-shortcut-id').textContent);
            const add = recording.classList.contains('desktop-shortcut-add');
            const previous = recording.closest('.desktop-shortcut-extra')?.querySelector('.desktop-shortcut-previous').textContent;
            const {epoch, preset} = origin;
            stopRecording();
            if (chord !== 'Escape' && state.epoch === epoch && state.preset === preset)
                send(desktop, epoch, {kind: 'record', action, chord, preset, add, previous});
            return;
        }

        // The native host decides whether to capture these shell shortcuts,
        // including when capture is disabled or a native child owns focus.
        if (state.windowsHost && state.preset === 'windows'
            && (chord === 'Meta+D' || chord === 'Alt+F4'
                || (chord === 'Ctrl+F4' && state.bindings.close_tab.includes(chord)))) return;
        const modifiedTab = chord === 'Ctrl+Tab' || chord === 'Ctrl+Shift+Tab';
        if (target.closest(protectedFocus)
            || (!modifiedTab && chord !== 'Escape' && target.closest(editableFocus))
            || [...document.querySelectorAll('.pane-menu,.context-popover,.terminal-context-menu,[role=dialog]')]
                .some(menu => menu.getClientRects().length)) return;

        const canvas = desktop.querySelector('.desktop-canvas');
        const {bindings, commands} = state;
        if (canvas.contains(target)) {
            const focus = (bindings.focus_next.includes(chord) && chord !== 'Tab') ? 1
                : (bindings.focus_previous.includes(chord) && chord !== 'Shift+Tab') ? -1 : 0;
            if (focus) {
                event.preventDefault();
                event.stopImmediatePropagation();
                moveFocus(canvas, focus);
                return;
            }
            if ((bindings.activate.includes(chord) && chord !== 'Enter')
                || (bindings.activate_space.includes(chord) && chord !== 'Space')) {
                const control = target.closest('button:not(:disabled), a[href], [role=button]');
                if (control) {
                    event.preventDefault();
                    event.stopImmediatePropagation();
                    if (!event.repeat) control.click();
                    return;
                }
            }
            const nativeAction = {Tab: 'focus_next', 'Shift+Tab': 'focus_previous',
                Enter: 'activate', Space: 'activate_space'}[chord];
            const actionable = target.closest('button, a[href], [role=button]');
            if (nativeAction && !bindings[nativeAction].includes(chord)
                && (chord.endsWith('Tab') || actionable)) {
                event.preventDefault();
                event.stopImmediatePropagation();
                return;
            }
        }

        if (target === canvas && chord === 'Enter') {
            const terminal = desktop.querySelector('.desktop-window.active .terminal-module:not(.hidden) .xterm-helper-textarea');
            if (terminal) {
                event.preventDefault();
                event.stopImmediatePropagation();
                terminal.focus({preventScroll: true});
            }
            return;
        }

        const duplicate = 'files_duplicate_window';
        const action = commands.includes(duplicate) && bindings[duplicate].includes(chord)
            ? duplicate : commands.find(id => bindings[id].includes(chord));
        if (!action) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!event.repeat) {
            canvas.focus({preventScroll: true});
            send(desktop, state.epoch, {kind: 'run', action, chord, preset: state.preset});
        }
    }, true);
})();
