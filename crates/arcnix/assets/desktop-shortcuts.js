(() => {
    const recordButton = '.desktop-shortcut-record';
    const editableFocus = 'input, textarea, select, [contenteditable]';
    const protectedFocus = [
        '[inert]',
        '.terminal-module', '.terminal-controls', '.terminal-context-menu',
        '[role=dialog]', '[role=menu]',
    ].join(',');
    let recording;

    function stopRecording() {
        recording?.removeAttribute('data-recording');
        recording?.removeAttribute('aria-pressed');
        recording = null;
    }

    function shortcutState(desktop) {
        try {
            return JSON.parse(desktop?.querySelector('.desktop-shortcut-state')?.textContent || 'null');
        } catch (_) {
            return null;
        }
    }

    function send(desktop, epoch, intent) {
        const input = desktop.querySelector('.desktop-shortcut-input');
        if (!input) return;
        input.value = JSON.stringify({ epoch, intent });
        input.dispatchEvent(new Event('input', { bubbles: true }));
    }

    function chordFor(event) {
        const code = event.code || event.key;
        const key = /^(Key[A-Z]|Digit[0-9])$/.test(code) ? code.slice(-1) : code;
        const parts = [];
        if (event.ctrlKey) parts.push('Ctrl');
        if (event.altKey) parts.push('Alt');
        if (event.shiftKey) parts.push('Shift');
        if (event.metaKey) parts.push('Meta');
        return [...parts, key].join('+');
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

    document.addEventListener('keydown', event => {
        if (event.defaultPrevented || event.isComposing || event.getModifierState('AltGraph')) return;
        const target = event.target;
        if (!(target instanceof Element)) return;
        const desktop = document.querySelector('.server-desktop:not(.hidden)');
        const state = shortcutState(desktop);
        if (!state?.active) return;

        if (recording && target === recording) {
            if (event.key === 'Tab' && !event.ctrlKey && !event.altKey && !event.metaKey) {
                stopRecording();
                return;
            }
            if (['Control', 'Alt', 'Shift', 'Meta'].includes(event.key)) return;
            event.preventDefault();
            event.stopImmediatePropagation();
            if (event.repeat) return;
            const action = JSON.parse(recording.parentElement.querySelector('.desktop-shortcut-id').textContent);
            stopRecording();
            if (event.key !== 'Escape') {
                send(desktop, state.epoch, { kind: 'record', action, chord: chordFor(event) });
            }
            return;
        }

        const chord = chordFor(event);
        const modifiedTab = chord === 'Ctrl+Tab' || chord === 'Ctrl+Shift+Tab';
        if (target.closest(protectedFocus)
            || (!modifiedTab && target.closest(editableFocus))
            || [...document.querySelectorAll('.pane-menu,.context-popover,.terminal-context-menu,[role=dialog]')]
                .some(menu => menu.getClientRects().length)) return;

        const canvas = desktop.querySelector('.desktop-canvas');
        if (target === canvas && event.key === 'Enter'
            && !event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) {
            const terminal = desktop.querySelector('.desktop-window.active .terminal-module:not(.hidden) .xterm-helper-textarea');
            if (terminal) {
                event.preventDefault();
                event.stopImmediatePropagation();
                terminal.focus({ preventScroll: true });
            }
            return;
        }

        if (!state.bindings.includes(chord)) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!event.repeat) {
            canvas.focus({ preventScroll: true });
            send(desktop, state.epoch, { kind: 'run', chord });
        }
    }, true);
})();
