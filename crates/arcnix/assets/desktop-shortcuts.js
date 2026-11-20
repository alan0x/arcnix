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
    const recordButton = '.desktop-shortcut-record';
    const editableFocus = 'input, textarea, select, [contenteditable]';
    const protectedFocus = [
        '[inert]', '.terminal-module', '.terminal-controls', '.terminal-context-menu',
        '[role=dialog]', '[role=menu]',
    ].join(',');
    let recording;

    function stopRecording() {
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
            const action = JSON.parse(recording.parentElement.querySelector('.desktop-shortcut-id').textContent);
            stopRecording();
            if (chord !== 'Escape') send(desktop, state.epoch, {kind: 'record', action, chord});
            return;
        }

        const modifiedTab = chord === 'Ctrl+Tab' || chord === 'Ctrl+Shift+Tab';
        if (target.closest(protectedFocus)
            || (!modifiedTab && chord !== 'Escape' && target.closest(editableFocus))
            || [...document.querySelectorAll('.pane-menu,.context-popover,.terminal-context-menu,[role=dialog]')]
                .some(menu => menu.getClientRects().length)) return;

        const canvas = desktop.querySelector('.desktop-canvas');
        const {bindings, commands} = state;
        if (canvas.contains(target)) {
            const focus = (bindings.focus_next === chord && chord !== 'Tab') ? 1
                : (bindings.focus_previous === chord && chord !== 'Shift+Tab') ? -1 : 0;
            if (focus) {
                event.preventDefault();
                event.stopImmediatePropagation();
                moveFocus(canvas, focus);
                return;
            }
            if ((bindings.activate === chord && chord !== 'Enter')
                || (bindings.activate_space === chord && chord !== 'Space')) {
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
            if (nativeAction && bindings[nativeAction] !== chord
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
        const action = commands.includes(duplicate) && bindings[duplicate] === chord
            ? duplicate : commands.find(id => bindings[id] === chord);
        if (!action) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!event.repeat) {
            canvas.focus({preventScroll: true});
            send(desktop, state.epoch, {kind: 'run', action});
        }
    }, true);
})();
