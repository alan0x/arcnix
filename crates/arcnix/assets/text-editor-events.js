(() => {
    document.addEventListener('keydown', event => {
        if (!(event.ctrlKey || event.metaKey) || event.altKey || event.key.toLowerCase() !== 's') return;
        const editor = event.target.closest?.('.remote-editor-modal, .desktop-window.app-editor');
        if (!editor) return;
        event.preventDefault();
        event.stopPropagation();
        const save = editor.querySelector('.remote-editor-save');
        if (save && !save.disabled) save.click();
    }, true);

    const rendered = new WeakMap();
    function renderPreviews() {
        for (const host of document.querySelectorAll('.editor-preview-host')) {
            const preview = host.querySelector('.markdown-preview');
            const html = host.querySelector('.markdown-preview-source')?.textContent;
            if (!preview || html === undefined || rendered.get(preview) === html) continue;
            rendered.set(preview, html);
            // Rust emits Markdown markup with raw HTML, links and images escaped or stripped.
            const template = document.createElement('template');
            template.innerHTML = html;
            preview.replaceChildren(template.content);
        }
    }
    // The textarea owns editing, selection, undo and IME. Only visible lines are painted.
    const sheets = new Map();
    const checkpointSize = 64;
    const maxTokenLine = 16000;

    function decoration(className, parent) {
        const node = document.createElement('div');
        node.className = 'editor-decoration ' + className;
        node.setAttribute('aria-hidden', 'true');
        parent.append(node);
        return node;
    }

    function fileMode(editor) {
        const name = editor.querySelector('.editor-path')?.textContent.split('/').pop() || '';
        const syntax = CodeMirror.findModeByFileName(name)
            || (/^\.env(?:\.|$)/i.test(name) && CodeMirror.findModeByName('Shell'))
            || (/\.(?:service|socket|timer|ini|conf)$/i.test(name) && CodeMirror.findModeByMIME('text/x-ini'));
        return syntax && syntax.mode !== 'null' && CodeMirror.modes[syntax.mode] ? syntax : null;
    }

    function setText(node, value) {
        if (node && node.textContent !== value) node.textContent = value;
    }

    function lineAt(starts, offset) {
        let low = 0, high = starts.length;
        while (low + 1 < high) {
            const mid = (low + high) >>> 1;
            if (starts[mid] <= offset) low = mid;
            else high = mid;
        }
        return low;
    }

    class SourceView {
        constructor(host, input) {
            this.host = host;
            this.input = input;
            this.editor = host.closest('.remote-editor');
            this.active = decoration('editor-active-line', host);
            this.gutter = decoration('editor-gutter', host);
            this.numbers = decoration('editor-numbers', this.gutter);
            this.clip = decoration('editor-code-clip', host);
            this.code = decoration('editor-code-lines', this.clip);
            input.wrap = 'off';
            input.spellcheck = false;
            input.autocapitalize = 'off';
            input.addEventListener('input', () => this.schedule());
            input.addEventListener('scroll', () => this.schedule(), {passive: true});
            input.addEventListener('selectionchange', () => this.schedule());
            input.addEventListener('keydown', event => {
                if (!this.mode || event.key !== 'Enter' || event.isComposing || event.shiftKey ||
                    event.ctrlKey || event.metaKey || event.altKey || input.selectionStart !== input.selectionEnd) return;
                const caret = input.selectionStart;
                const start = input.value.slice(0, caret).lastIndexOf('\n') + 1;
                const indent = /^[\t ]+/.exec(input.value.slice(start, caret));
                // A native edit keeps auto-indent in the same undo stack as normal typing.
                if (indent && document.execCommand('insertText', false, '\n' + indent[0])) event.preventDefault();
            });
            input.addEventListener('compositionstart', () => host.classList.add('editor-composing'));
            input.addEventListener('compositionend', () => {
                host.classList.remove('editor-composing');
                this.schedule();
            });
            this.resize = new ResizeObserver(() => this.schedule());
            this.resize.observe(input);
        }

        schedule() {
            if (!this.frame) this.frame = requestAnimationFrame(() => {
                this.frame = 0;
                if (this.host.isConnected) this.draw();
            });
        }

        setSyntax(syntax) {
            if (syntax === this.syntax) return;
            this.syntax = syntax;
            this.mode = syntax && CodeMirror.getMode({indentUnit: 4, tabSize: 4}, syntax.mime);
            this.value = undefined;
        }

        index(value) {
            if (this.value !== undefined) {
                let keep = 0;
                const end = Math.min(value.length, this.value.length);
                while (keep < end && value.charCodeAt(keep) === this.value.charCodeAt(keep)) ++keep;
                // Retain only complete, unchanged blocks; some modes look ahead a line.
                const stable = Math.max(0, lineAt(this.starts, keep) - 1);
                this.states.length = Math.min(this.states.length, 1 + Math.floor(stable / checkpointSize));
            } else {
                this.states = this.mode ? [CodeMirror.startState(this.mode)] : [];
            }
            this.starts = [0];
            this.longLine = false;
            let start = 0;
            for (let end = value.indexOf('\n'); end !== -1; end = value.indexOf('\n', start)) {
                if (end - start > maxTokenLine) this.longLine = true;
                start = end + 1;
                this.starts.push(start);
            }
            this.longLine ||= value.length - start > maxTokenLine;
            this.value = value;
            this.range = '';
            this.position = -1;
            this.host.style.setProperty('--editor-gutter-width', `calc(${Math.max(2, String(this.starts.length).length)}ch + 24px)`);
        }

        line(index) {
            return this.value.slice(this.starts[index], (this.starts[index + 1] ?? this.value.length + 1) - 1);
        }

        tokenize(index, state, row) {
            const line = this.line(index);
            if (!line && this.mode.blankLine) this.mode.blankLine(state);
            const stream = new CodeMirror.StringStream(line, 4, {
                lookAhead: n => index + n < this.starts.length ? this.line(index + n) : undefined,
                baseToken: () => {},
            });
            let retries = 0;
            while (!stream.eol()) {
                const style = this.mode.token(stream, state);
                // Mixed-language modes may switch lexers before consuming a character.
                if (stream.pos <= stream.start) {
                    if (++retries < 8) continue;
                    stream.next();
                }
                retries = 0;
                if (row) {
                    if (style) {
                        const token = document.createElement('span');
                        token.className = 'cm-' + style.replace(/ +/g, ' cm-');
                        token.textContent = stream.current();
                        row.append(token);
                    } else row.append(stream.current());
                }
                stream.start = stream.pos;
            }
        }

        paint(first, end) {
            const block = Math.floor(first / checkpointSize);
            const deadline = performance.now() + 6;
            // Jumping down a large file can need more lexer state than fits in one frame.
            while (this.states.length <= block) {
                const from = (this.states.length - 1) * checkpointSize;
                const state = CodeMirror.copyState(this.mode, this.states.at(-1));
                for (let i = from; i < from + checkpointSize; ++i) this.tokenize(i, state);
                this.states.push(state);
                if (performance.now() > deadline) return false;
            }
            const state = CodeMirror.copyState(this.mode, this.states[block]);
            const rows = document.createDocumentFragment();
            for (let i = block * checkpointSize; i < end; ++i) {
                const row = i >= first ? document.createElement('div') : null;
                this.tokenize(i, state, row);
                if (row) rows.append(row);
                if ((i + 1) % checkpointSize === 0 && this.states.length === (i + 1) / checkpointSize)
                    this.states.push(CodeMirror.copyState(this.mode, state));
            }
            this.code.replaceChildren(rows);
            return true;
        }

        draw() {
            const input = this.input;
            if (!input.clientHeight) return;
            if (this.value !== input.value) this.index(input.value);
            const css = getComputedStyle(input);
            const height = parseFloat(css.lineHeight);
            const first = Math.max(0, Math.min(this.starts.length - 1, Math.floor((input.scrollTop - parseFloat(css.paddingTop)) / height)));
            const end = Math.min(this.starts.length, first + Math.ceil(input.clientHeight / height) + 2);
            const y = parseFloat(css.paddingTop) + height * first - input.scrollTop;
            this.numbers.style.transform = `translateY(${y}px)`;
            this.clip.style.width = input.clientWidth + 'px';
            this.clip.style.height = this.gutter.style.height = input.clientHeight + 'px';
            this.code.style.transform = `translate(${parseFloat(css.paddingLeft) - input.scrollLeft}px, ${y}px)`;
            const key = first + '/' + end;
            if (this.range !== key) {
                this.numbers.textContent = Array.from({length: end - first}, (_, i) => first + i + 1).join('\n');
                const colored = this.mode && !this.longLine;
                const ready = !colored || this.paint(first, end);
                this.host.classList.toggle('editor-highlighted', Boolean(colored && ready));
                if (ready) this.range = key;
                else this.schedule();
            }
            const caret = input.selectionDirection === 'backward' ? input.selectionStart : input.selectionEnd;
            const line = lineAt(this.starts, caret);
            this.active.style.top = parseFloat(css.paddingTop) + line * height - input.scrollTop + 'px';
            this.active.hidden = line < first || line >= end || input.disabled || input.selectionStart !== input.selectionEnd;
            if (this.position !== caret || this.lineNode !== this.editor.querySelector('.editor-line-value')) {
                this.lineNode = this.editor.querySelector('.editor-line-value');
                setText(this.lineNode, String(line + 1));
                let col = 0;
                for (const ch of this.value.slice(this.starts[line], caret)) col += ch === '\t' ? 4 - col % 4 : 1;
                setText(this.editor.querySelector('.editor-column-value'), String(col + 1));
                this.position = caret;
            }
        }
    }

    function syncEditors() {
        for (const [host, view] of sheets) {
            if (!host.isConnected) {
                cancelAnimationFrame(view.frame);
                view.resize.disconnect();
                sheets.delete(host);
            }
        }
        for (const editor of document.querySelectorAll('.desktop-editor .remote-editor')) {
            const host = editor.querySelector('.editor-sheet');
            const input = host?.querySelector('textarea');
            let view = sheets.get(host);
            if (input && !view) {
                view = new SourceView(host, input);
                sheets.set(host, view);
            }
            const syntax = fileMode(editor);
            view?.setSyntax(syntax);
            setText(editor.querySelector('.editor-mode-name'), syntax?.name || '');
            view?.schedule();
        }
    }

    const observer = new MutationObserver(records => {
        if (records.every(record => {
            const target = record.target.nodeType === 1 ? record.target : record.target.parentElement;
            return target?.closest('.editor-decoration, .editor-mode-name, .editor-line-value, .editor-column-value, .markdown-preview');
        })) return;
        renderPreviews();
        syncEditors();
    });
    observer.observe(document, {childList: true, characterData: true, attributes: true, attributeFilter: ['disabled'], subtree: true});
    renderPreviews();
    syncEditors();
})();
