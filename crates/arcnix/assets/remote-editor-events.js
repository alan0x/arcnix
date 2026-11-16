(() => {
    document.addEventListener('keydown', event => {
        if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return;
        const editor = event.target.closest?.('.remote-editor-modal');
        if (!editor) return;
        event.preventDefault();
        event.stopPropagation();
        const save = editor.querySelector('.remote-editor-save');
        if (save && !save.disabled) save.click();
    }, true);
})();
