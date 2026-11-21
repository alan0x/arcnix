// Display Rust-owned samples only. No network calls, timers or persisted data.
(() => {
    const NS = 'http://www.w3.org/2000/svg';
    const panels = new Map();
    const element = (tag, className, parent) => {
        const node = document.createElement(tag);
        node.className = className;
        parent?.append(node);
        return node;
    };
    const svgNode = (tag, attributes, parent, text) => {
        const node = document.createElementNS(NS, tag);
        for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
        if (text !== undefined) node.textContent = text;
        parent?.append(node);
        return node;
    };
    const valid = value => typeof value === 'number' && Number.isFinite(value) && value >= 0;
    const niceTop = maximum => {
        if (!(maximum > 0)) return 1;
        const raw = maximum / 4;
        const power = 10 ** Math.floor(Math.log10(raw));
        const step = [1, 2, 2.5, 5, 10].find(n => n * power >= raw) * power;
        const result = Math.ceil(maximum / step) * step;
        return Number.isFinite(result) && result >= maximum ? result : maximum;
    };
    function rate(value, data, axis = false) {
        if (!valid(value)) return data.labels.missing;
        const base = data.base === 1024 ? 1024 : 1000;
        const units = base === 1000 ? ['B', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB'] : ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB', 'EiB'];
        let index = 0;
        while (value >= base && index < units.length - 1) { value /= base; index++; }
        const decimals = index ? Math.min(data.decimals ?? 1, axis ? 1 : 2) : 0;
        return `${value.toLocaleString(data.locale, { maximumFractionDigits: decimals, useGrouping: !!data.grouping })} ${units[index]}/s`;
    }
    function makeChart(root, kind, keys) {
        const figure = element('figure', `resource-trend-chart resource-trend-${kind}`, root);
        const heading = element('div', 'resource-trend-chart-heading', figure);
        const title = element('div', 'resource-trend-title', heading);
        const legend = element('div', 'resource-trend-legend', figure);
        const svg = svgNode('svg', { tabindex: '0', role: 'img', 'aria-keyshortcuts': 'ArrowLeft ArrowRight Home End' }, figure);
        const tooltip = element('div', 'resource-trend-tooltip', figure);
        tooltip.hidden = true;
        const state = { figure, title, legend, svg, tooltip, keys, kind, activeAt: null, data: null, points: [], width: 0 };
        const hide = () => { state.activeAt = null; tooltip.hidden = true; state.crosshair?.setAttribute('visibility', 'hidden'); };
        svg.addEventListener('pointermove', event => {
            if (!state.points.length) return;
            const rect = svg.getBoundingClientRect();
            const time = state.start + Math.max(0, Math.min(1, (event.clientX - rect.left - state.left) / state.plotWidth)) * state.span;
            inspect(state, state.points.reduce((best, p) => Math.abs(p.at - time) < Math.abs(best.at - time) ? p : best));
        });
        svg.addEventListener('pointerleave', () => { if (document.activeElement !== svg) hide(); });
        svg.addEventListener('focus', () => { if (state.points.length) inspect(state, state.points.at(-1)); });
        svg.addEventListener('blur', hide);
        svg.addEventListener('keydown', event => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End', 'Escape'].includes(event.key)) return;
            event.preventDefault();
            if (event.key === 'Escape') { hide(); return; }
            if (!state.points.length) return;
            let index = state.points.findIndex(p => p.at === state.activeAt);
            if (index < 0) index = state.points.length - 1;
            index = event.key === 'Home' ? 0 : event.key === 'End' ? state.points.length - 1 : Math.max(0, Math.min(state.points.length - 1, index + (event.key === 'ArrowLeft' ? -1 : 1)));
            inspect(state, state.points[index]);
        });
        return state;
    }
    function formatted(state, key, value) {
        if (!valid(value)) return state.data.labels.missing;
        return state.kind === 'network' ? rate(value, state.data) : `${value.toLocaleString(state.data.locale, { maximumFractionDigits: 1 })}%`;
    }
    function inspect(state, point) {
        state.activeAt = point.at;
        const x = state.x(point.at);
        const age = Math.max(0, state.end - point.at);
        const time = age < 0.01 ? state.data.labels.latest : `−${age.toLocaleString(state.data.locale, { maximumFractionDigits: 1 })} ${state.data.labels.seconds}`;
        const values = state.keys.map(key => `${state.data.labels[key]}: ${formatted(state, key, point[key])}`);
        state.tooltip.textContent = [time, ...values].join(' · ');
        state.tooltip.hidden = false;
        state.tooltip.style.left = `${Math.max(6, Math.min(x, state.width - 250))}px`;
        state.svg.setAttribute('aria-label', `${state.data.labels[state.kind]} · ${state.tooltip.textContent}`);
        state.crosshair.replaceChildren();
        state.crosshair.setAttribute('visibility', 'visible');
        svgNode('line', { x1: x, x2: x, y1: state.top, y2: state.bottom, class: 'resource-trend-crosshair' }, state.crosshair);
        for (const key of state.keys) if (valid(point[key])) svgNode('circle', { cx: x, cy: state.y(point[key]), r: 4, class: `resource-trend-marker ${key}` }, state.crosshair);
    }
    function draw(state, data) {
        const width = Math.floor(state.svg.clientWidth);
        if (width < 100) return;
        const height = 196, left = 72, right = 14, top = 14, bottom = 166;
        state.data = data; state.width = width; state.left = left; state.top = top; state.bottom = bottom;
        state.plotWidth = Math.max(1, width - left - right);
        state.end = data.points.at(-1)?.at ?? 0;
        state.span = Math.min(data.range, Math.max(1, state.end));
        state.start = state.end - state.span;
        state.points = data.points.filter(p => p.at >= state.start && p.at <= state.end);
        const maximum = state.kind === 'network' ? niceTop(Math.max(0, ...state.points.flatMap(p => state.keys.map(key => valid(p[key]) ? p[key] : 0)))) : 100;
        state.x = at => left + (at - state.start) / state.span * state.plotWidth;
        state.y = value => bottom - Math.min(1, value / maximum) * (bottom - top);
        state.title.textContent = data.labels[state.kind];
        state.legend.replaceChildren();
        const latest = state.points.at(-1);
        for (const key of state.keys) {
            const entry = element('span', `resource-trend-legend-entry ${key}`, state.legend);
            if (state.keys.length > 1) element('span', 'resource-trend-swatch', entry);
            entry.append(document.createTextNode(`${state.keys.length > 1 ? data.labels[key] + ' ' : ''}${formatted(state, key, latest?.[key])}`));
        }
        const svg = state.svg;
        svg.setAttribute('viewBox', `0 0 ${width} ${height}`);
        svg.setAttribute('aria-label', `${data.labels[state.kind]} · ${state.legend.textContent}`);
        svg.replaceChildren();
        for (let i = 0; i <= 4; i++) {
            const value = maximum * i / 4, y = state.y(value);
            svgNode('line', { x1: left, x2: width - right, y1: y, y2: y, class: 'resource-trend-grid' }, svg);
            svgNode('text', { x: left - 8, y: y + 4, 'text-anchor': 'end', class: 'resource-trend-axis' }, svg, state.kind === 'network' ? rate(value, data, true) : `${value}%`);
        }
        for (let i = 0; i <= 2; i++) {
            const age = state.span * (2 - i) / 2;
            const amount = (age >= 60 ? age / 60 : age).toLocaleString(data.locale, { maximumFractionDigits: 1 });
            svgNode('text', { x: left + state.plotWidth * i / 2, y: height - 7, 'text-anchor': i === 0 ? 'start' : i === 2 ? 'end' : 'middle', class: 'resource-trend-axis' }, svg, age ? `−${amount} ${age >= 60 ? data.labels.minutes : data.labels.seconds}` : data.labels.latest);
        }
        let hasValues = false;
        for (const key of state.keys) {
            let segment = [];
            const flush = () => {
                if (!segment.length) return;
                hasValues = true;
                if (segment.length === 1) svgNode('circle', { cx: state.x(segment[0].at), cy: state.y(segment[0][key]), r: 3, class: `resource-trend-marker ${key}` }, svg);
                else svgNode('path', { d: segment.map((p, i) => `${i ? 'L' : 'M'}${state.x(p.at).toFixed(2)},${state.y(p[key]).toFixed(2)}`).join(' '), class: `resource-trend-line ${key}`, 'data-series': key }, svg);
                segment = [];
            };
            for (const point of state.points) {
                if (point.break_before || !valid(point[key])) flush();
                if (valid(point[key])) segment.push(point);
            }
            flush();
        }
        if (!hasValues) svgNode('text', { x: left + state.plotWidth / 2, y: top + (bottom - top) / 2, 'text-anchor': 'middle', class: 'resource-trend-empty' }, svg, data.labels.empty);
        state.crosshair = svgNode('g', { visibility: 'hidden' }, svg);
        const selected = state.points.find(p => p.at === state.activeAt);
        if (selected) inspect(state, selected); else state.tooltip.hidden = true;
    }
    function update(panel) {
        const source = panel.querySelector('.resource-trend-data');
        const root = panel.querySelector('.resource-trend-plots');
        if (!source || !root) return;
        let state = panels.get(panel);
        if (state && (state.root !== root || state.charts.some(chart => chart.figure.parentElement !== root))) {
            state.resize.disconnect();
            panels.delete(panel);
            root.replaceChildren();
            state = null;
        }
        if (!state) {
            const catalog = [['cpu', ['cpu']], ['memory', ['memory']], ['network', ['receive', 'transmit']]];
            const single = catalog.find(([kind]) => root.classList.contains(`plot-${kind}`));
            const charts = (single ? [single] : catalog).map(([kind, keys]) => makeChart(root, kind, keys));
            state = { root, text: '', data: null, charts };
            state.resize = new ResizeObserver(() => {
                if (!state.data) return;
                for (const chart of state.charts) {
                    if (chart.data !== state.data || Math.floor(chart.svg.clientWidth) !== chart.width) draw(chart, state.data);
                }
            });
            state.resize.observe(root);
            panels.set(panel, state);
        }
        if (state.text === source.textContent) return;
        state.text = source.textContent;
        try {
            const data = JSON.parse(state.text);
            if (![60, 300, 900].includes(data.range) || !Array.isArray(data.points) || !data.labels) return;
            data.points = data.points.slice(-1024).filter(p => p && valid(p.at));
            state.data = data;
            for (const chart of state.charts) draw(chart, data);
        } catch { /* The Rust payload is replaced atomically on the next render. */ }
    }
    let scheduled = false;
    function scan() {
        scheduled = false;
        for (const [panel, state] of panels) if (!panel.isConnected) { state.resize.disconnect(); panels.delete(panel); }
        document.querySelectorAll('.resource-trends').forEach(update);
    }
    new MutationObserver(records => {
        if (scheduled) return;
        const selector = '.resource-trends,.resource-trend-data,.resource-trend-plots,.resource-trend-chart';
        const relevant = records.some(record => (record.target.nodeType === Node.ELEMENT_NODE ? record.target : record.target.parentElement)?.closest('.resource-trend-data') || [...record.addedNodes, ...record.removedNodes].some(n => n.nodeType === Node.ELEMENT_NODE && (n.matches(selector) || n.querySelector(selector))));
        if (relevant) { scheduled = true; queueMicrotask(scan); }
    }).observe(document, { childList: true, subtree: true, characterData: true });
    scan();
})();
