// Runs in the private Office WebView, after Syntra's document has loaded.
addEventListener('DOMContentLoaded', () => {
    const {kind, width, zh} = officeOptions;
    const label = (en, cn) => zh ? cn : en;
    const make = (tag, className) => {
        const node = document.createElement(tag);
        if (className) node.className = className;
        return node;
    };
    const style = make('style');
    style.textContent = `
        html,body{height:100%;overflow:hidden}
        body{display:flex;flex-direction:column;background:#e8eaee;margin:0}
        .reader-tools{flex:none;display:flex;align-items:center;gap:6px;flex-wrap:wrap;padding:5px 9px;
            background:#f9fafb;border-bottom:1px solid #d4d9df;font:13px system-ui,sans-serif;color:#263447}
        .reader-tools :is(select,button){font:inherit;color:inherit;background:white;border:1px solid #c3cbd5;
            border-radius:4px;min-height:28px;padding:2px 8px}
        .reader-tools select{max-width:min(42vw,280px)}
        .reader-tools button:disabled{opacity:.45}
        .reader-tools output{min-width:4ch;text-align:center}
        .reader-stage{flex:1;min-height:0;overflow:auto;padding:12px;overscroll-behavior:contain}
        .reader-paper{background:white;margin-inline:auto;box-shadow:0 1px 5px #0003}
        .reader-paper[data-kind="writer"]{min-height:85vh;padding:clamp(20px,5vw,72px)}
        .reader-paper[data-kind="calc"]{padding:12px}
        .reader-paper .writer-page-break{margin:18px 0;border-top:1px dashed #bcc3cd}
        .reader-paper [hidden]{display:none!important}
    `;
    document.head.append(style);
    const paper = make('article', 'reader-paper');
    paper.dataset.kind = kind;
    while (document.body.firstChild) paper.append(document.body.firstChild);
    // Reading a hyperlink never replaces the current document.
    for (const a of paper.querySelectorAll('a[href]')) a.removeAttribute('href');
    const stage = make('div', 'reader-stage');
    stage.append(paper);
    const toolbar = make('nav', 'reader-tools');
    toolbar.setAttribute('aria-label', label('Document controls', '文档控制'));
    document.body.append(toolbar, stage);
    const button = (name, caption, action) => {
        const el = make('button');
        el.type = 'button';
        el.setAttribute('aria-label', name);
        el.title = name;
        el.textContent = caption;
        el.addEventListener('click', action);
        toolbar.append(el);
        return el;
    };

    // A sheet may have several print segments followed by separate chart sections.
    const groups = [];
    if (kind === 'slides') {
        for (const node of paper.querySelectorAll(':scope > .canvas-page')) {
            groups.push({name: label('Slide ', '幻灯片 ') + (groups.length + 1), nodes: [node]});
        }
    } else if (kind === 'calc') {
        for (const node of paper.children) {
            const name = node.getAttribute('aria-label') ?? node.querySelector('h1')?.textContent;
            if (node.matches('.sheet') && (!groups.length || groups.at(-1).name !== name)) {
                groups.push({name: name || label('Worksheet', '工作表'), nodes: []});
            }
            groups.at(-1)?.nodes.push(node);
        }
    }
    if (groups.length) {
        let index = 0;
        const previous = button(label('Previous', '上一项'), '‹', () => select(index - 1));
        const picker = make('select');
        picker.setAttribute('aria-label', kind === 'calc' ? label('Worksheet', '工作表') : label('Slide', '幻灯片'));
        groups.forEach((group, i) => picker.add(new Option(group.name, String(i))));
        picker.onchange = () => select(picker.selectedIndex);
        toolbar.append(picker);
        const next = button(label('Next', '下一项'), '›', () => select(index + 1));
        function select(i) {
            if (i < 0 || i >= groups.length) return;
            for (const el of groups[index].nodes) el.hidden = true;
            index = i;
            for (const el of groups[index].nodes) el.hidden = false;
            picker.selectedIndex = index;
            previous.disabled = index === 0;
            next.disabled = index === groups.length - 1;
            stage.scrollTo(0, 0);
        }
        for (const group of groups) for (const el of group.nodes) el.hidden = true;
        select(0);
        addEventListener('keydown', event => {
            if (event.target.closest('select,button') || kind !== 'slides') return;
            if (['PageUp','ArrowLeft','PageDown','ArrowRight'].includes(event.key)) {
                event.preventDefault();
                select(index + (['PageUp','ArrowLeft'].includes(event.key) ? -1 : 1));
            }
        });
    }

    let fit = true;
    let scale = 1;
    const out = button(label('Zoom out', '缩小'), '−', () => adjust(1 / 1.25));
    const status = make('output');
    toolbar.append(status);
    const into = button(label('Zoom in', '放大'), '+', () => adjust(1.25));
    const fitName = label('Fit width', '适应宽度');
    const fitButton = button(fitName, fitName, () => { fit = true; resize(); });
    function adjust(factor) {
        fit = false;
        scale = Math.min(4, Math.max(.05, scale * factor));
        resize();
    }
    function resize() {
        if (fit) scale = Math.min(1, Math.max(.01, (stage.clientWidth - 24) / width));
        paper.style.width = width + 'px';
        paper.style.zoom = String(scale);
        status.value = Math.round(scale * 100) + '%';
        fitButton.setAttribute('aria-pressed', String(fit));
        out.disabled = scale <= .05;
        into.disabled = scale >= 4;
    }
    new ResizeObserver(resize).observe(stage);
    resize();
    addEventListener('pointerdown', () => window.ipc.postMessage('focus'), true);
});
