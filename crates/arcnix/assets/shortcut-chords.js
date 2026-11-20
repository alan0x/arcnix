window.arcnixShortcutChord = event => {
    const code = event.code || event.key;
    const key = /^(Key[A-Z]|Digit[0-9])$/.test(code) ? code.slice(-1)
        : code === 'NumpadEnter' ? 'Enter' : code === ' ' ? 'Space' : code;
    const parts = [];
    if (event.ctrlKey) parts.push('Ctrl');
    if (event.altKey) parts.push('Alt');
    if (event.shiftKey) parts.push('Shift');
    if (event.metaKey) parts.push('Meta');
    return [...parts, key].join('+');
};

window.arcnixChordMatches = (event, binding) => {
    if (!binding || event.isComposing || event.getModifierState('AltGraph')) return false;
    const chord = window.arcnixShortcutChord(event);
    if (chord === binding) return true;
    // Keep Command equivalents for controls using the portable Ctrl bindings.
    return /Mac|iPhone|iPad/.test(navigator.platform)
        && event.metaKey && !event.ctrlKey && binding.startsWith('Ctrl+')
        && chord.replace(/(^|\+)Meta\+/, '$1') === binding.slice(5);
};
