// Display Rust-owned samples only. No network calls, timers or persisted data.
(() => {
    const NS = 'http://www.w3.org/2000/svg';
    const panels = new Map();
    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
    let nextClipId = 0;
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
    function size(value, data, axis = false) {
        if (!valid(value)) return data.labels.missing;
        const base = data.base === 1024 ? 1024 : 1000;
        const units = base === 1000 ? ['B', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB'] : ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB', 'EiB'];
        let index = 0;
        while (value >= base && index < units.length - 1) { value /= base; index++; }
        const decimals = index ? Math.min(data.decimals ?? 1, axis ? 1 : 2) : 0;
        return `${value.toLocaleString(data.locale, { maximumFractionDigits: decimals, useGrouping: !!data.grouping })} ${units[index]}`;
    }
    function makeChart(root, kind, keys) {
        const figure = element('figure', `resource-trend-chart resource-trend-${kind}`, root);
        const heading = element('div', 'resource-trend-chart-heading', figure);
        const title = element('div', 'resource-trend-title', heading);
        const legend = element('div', 'resource-trend-legend', figure);
        const svg = svgNode('svg', { tabindex: '0', role: 'img', 'aria-keyshortcuts': 'ArrowLeft ArrowRight Home End' }, figure);
        const tooltip = element('div', 'resource-trend-tooltip', figure);
        tooltip.hidden = true;
        const state = { figure, title, legend, svg, tooltip, keys, kind, clipId: `resource-trend-live-${++nextClipId}`, activeAt: null, data: null, points: [], width: 0 };
        const hide = () => {
            state.activeAt = null;
            tooltip.hidden = true;
            state.crosshair?.setAttribute('visibility', 'hidden');
            if (state.data) svg.setAttribute('aria-label', `${title.textContent} · ${legend.textContent}`);
        };
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
        if (state.kind === 'network') return size(value, state.data) + '/s';
        if (['dedicated', 'shared'].includes(key)) return size(value, state.data);
        return `${value.toLocaleString(state.data.locale, { maximumFractionDigits: 1 })}%`;
    }
    function inspect(state, point) {
        state.activeAt = point.at;
        const age = Math.max(0, state.end - point.at);
        const time = age < 0.01 ? state.data.labels.latest : `−${age.toLocaleString(state.data.locale, { maximumFractionDigits: 1 })} ${state.data.labels.seconds}`;
        const values = state.keys.map(key => `${state.data.labels[key]}: ${formatted(state, key, point[key])}`);
        state.tooltip.textContent = [time, ...values].join(' · ');
        state.tooltip.hidden = false;
        positionInspection(state);
        state.svg.setAttribute('aria-label', `${state.data.labels[state.kind]} · ${state.tooltip.textContent}`);
        state.crosshair.replaceChildren();
        state.crosshair.setAttribute('visibility', 'visible');
        svgNode('line', { x1: 0, x2: 0, y1: state.top, y2: state.bottom, class: 'resource-trend-crosshair' }, state.crosshair);
        for (const key of state.keys) if (valid(point[key])) svgNode('circle', { cx: 0, cy: state.y(point[key]), r: 4, class: `resource-trend-marker ${key}` }, state.crosshair);
    }
    function positionInspection(state) {
        const x = Math.max(state.left, Math.min(state.left + state.plotWidth, state.x(state.activeAt)));
        state.crosshair.setAttribute('transform', `translate(${x} 0)`);
        const svgRect = state.svg.getBoundingClientRect();
        const cardRect = state.figure.getBoundingClientRect();
        const maxLeft = state.figure.clientWidth - state.tooltip.offsetWidth - 6;
        state.tooltip.style.left = `${Math.max(6, Math.min(svgRect.left - cardRect.left + x + 8, maxLeft))}px`;
        state.tooltip.style.top = `${svgRect.top - cardRect.top + 4}px`;
    }
    // Paths share one transform. Isolated samples use the same projection but
    // keep their radius in screen pixels while a new session's time span grows.
    function drawSeries(layer, plot, domain) {
        const paths = svgNode('g', {}, layer);
        const dots = [];
        for (const mark of plot.marks) {
            if (mark.kind === 'dot') {
                const node = svgNode('circle', { r: 3.5, class: `resource-trend-marker ${mark.series}` }, layer);
                dots.push({ node, mark });
            } else {
                svgNode('path', { d: mark.d, 'vector-effect': 'non-scaling-stroke', class: `resource-trend-${mark.kind} ${mark.series}`, 'data-series': mark.series }, paths);
            }
        }
        return { layer, paths, dots, domain, maximum: plot.maximum };
    }
    function projectSeries(state, series) {
        const x = state.x(series.domain[0]), width = state.x(series.domain[1]) - x;
        const y = state.y(series.maximum), height = state.bottom - y;
        series.paths.setAttribute('transform', `translate(${x} ${y}) scale(${width} ${height})`);
        for (const { node, mark } of series.dots) {
            node.setAttribute('cx', x + mark.x * width);
            node.setAttribute('cy', y + mark.y * height);
        }
    }
    function extendSample(state, previousView, plot, before, latest) {
        if (reducedMotion.matches) return;
        const { layer, domain } = state.series;
        const keys = state.keys.filter(key => !latest.break_before && valid(before[key]) && valid(latest[key]));

        const clip = svgNode('clipPath', { id: state.clipId, clipPathUnits: 'userSpaceOnUse' }, state.svg);
        svgNode('rect', { x: state.left - 4, y: 0, width: state.plotWidth + 4, height: state.svg.viewBox.baseVal.height }, clip);
        layer.setAttribute('clip-path', `url(#${state.clipId})`);
        // Keep the expiring left edge visible while it slides out of the window.
        const first = state.points[0].at;
        const outgoing = previousView.first < first ? previousView.series : null;
        let edgeClip, edgeRect;
        if (outgoing) {
            edgeClip = svgNode('clipPath', { id: `${state.clipId}-edge`, clipPathUnits: 'userSpaceOnUse' }, state.svg);
            edgeRect = svgNode('rect', { x: state.left, y: state.top - 4, height: state.bottom - state.top + 8 }, edgeClip);
            outgoing.layer.setAttribute('clip-path', `url(#${state.clipId}-edge)`);
            outgoing.layer.setAttribute('aria-hidden', 'true');
            layer.before(outgoing.layer);
        }
        const tips = svgNode('g', { class: 'resource-trend-live-tips', 'aria-hidden': 'true' }, state.svg);
        const heads = keys.map(key => {
            const head = svgNode('g', { class: `resource-trend-tip ${key}` }, tips);
            svgNode('circle', { r: 8, opacity: 0.23 }, head);
            svgNode('circle', { r: 3.5, class: 'resource-trend-tip-core' }, head);
            const y0 = state.y(before[key]), y3 = state.y(latest[key]);
            const controls = plot.marks.find(mark => mark.series === key && mark.tip_y)?.tip_y;
            const [y1, y2] = controls
                ? controls.map(y => state.top + y * (state.bottom - state.top))
                : [y0 + (y3 - y0) / 3, y3 - (y3 - y0) / 3];
            return { head, y0, y1, y2, y3 };
        });
        const duration = Math.min(1400, (latest.at - before.at) * 800);
        const start = performance.now();
        function frame(now) {
            // A resize, a newer sample or a page change replaces this layer.
            if (!layer.isConnected || state.series.layer !== layer) return;
            const t = Math.min(1, Math.max(0, (now - start) / duration));
            const progress = reducedMotion.matches ? 1 : t;
            state.start = previousView.start + (domain[0] - previousView.start) * progress;
            state.span = previousView.span + ((domain[1] - domain[0]) - previousView.span) * progress;
            projectSeries(state, state.series);
            if (state.activeAt !== null && state.crosshair?.isConnected) positionInspection(state);
            if (progress === 1) {
                layer.removeAttribute('clip-path');
                clip.remove();
                outgoing?.layer.remove();
                edgeClip?.remove();
                tips.remove();
                return;
            }
            if (outgoing) {
                projectSeries(state, outgoing);
                edgeRect.setAttribute('width', Math.max(0, state.x(first) - state.left));
                outgoing.layer.setAttribute('opacity', 1 - t);
            }
            // The moving time window draws the new segment at its right edge.
            // A manual refresh may arrive before the previous window has settled.
            const shown = (state.start + state.span - before.at) / (latest.at - before.at);
            const tipT = Math.max(0, Math.min(1, shown));
            const x = state.left + state.plotWidth;
            tips.setAttribute('opacity', shown < 0 ? 0 : Math.min(1, (1 - t) * 6));
            const u = 1 - tipT;
            for (const { head, y0, y1, y2, y3 } of heads) {
                const y = u ** 3 * y0 + 3 * u * u * tipT * y1 + 3 * u * tipT * tipT * y2 + tipT ** 3 * y3;
                head.setAttribute('transform', `translate(${x} ${y})`);
            }
            requestAnimationFrame(frame);
        }
        frame(start);
    }
    function draw(state, data) {
        const width = Math.floor(state.svg.clientWidth);
        if (width < 100) return;
        const height = data.adapter ? (['dedicated','shared'].includes(state.kind) ? 96 : 126) : 196;
        const right = 10, top = 12, bottom = height - 30;
        const previous = state.data;
        const previousView = { start: state.start, span: state.span, first: state.points[0]?.at, series: state.series };
        const plot = data.plots[state.kind];
        state.data = data; state.width = width; state.top = top; state.bottom = bottom;
        [state.start, state.end] = data.domain;
        state.span = state.end - state.start;
        state.points = data.points;
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
        if (!plot) {
            svgNode('text', { x: width / 2, y: height / 2, 'text-anchor': 'middle', class: 'resource-trend-empty' }, svg, data.labels.missing);
            state.points = [];
            state.tooltip.hidden = true;
            return;
        }
        state.y = value => bottom - value / plot.maximum * (bottom - top);
        const labels = plot.ticks.map(value => svgNode('text', { y: state.y(value) + 4, 'text-anchor': 'end', class: 'resource-trend-axis' }, svg, state.kind === 'network' ? size(value, data, true) + '/s' : ['dedicated','shared'].includes(state.kind) ? size(value,data,true) : `${value}%`));
        const left = Math.ceil(Math.max(0, ...labels.map(label => label.getComputedTextLength()))) + 10;
        state.left = left;
        state.plotWidth = Math.max(1, width - left - right);
        state.x = at => left + (at - state.start) / state.span * state.plotWidth;
        for (const [i, value] of plot.ticks.entries()) {
            const y = state.y(value);
            labels[i].setAttribute('x', left - 8);
            svgNode('line', { x1: left, x2: width - right, y1: y, y2: y, class: 'resource-trend-grid' }, svg);
        }
        for (let i = 0; i <= 2; i++) {
            if (i === 1 && state.plotWidth < 190) continue;
            const age = state.span * (2 - i) / 2;
            const amount = (age >= 60 ? age / 60 : age).toLocaleString(data.locale, { maximumFractionDigits: 1 });
            svgNode('text', { x: left + state.plotWidth * i / 2, y: height - 7, 'text-anchor': i === 0 ? 'start' : i === 2 ? 'end' : 'middle', class: 'resource-trend-axis' }, svg, age ? `−${amount} ${age >= 60 ? data.labels.minutes : data.labels.seconds}` : data.labels.latest);
        }
        const layer = svgNode('g', { class: 'resource-trend-series' }, svg);
        state.series = drawSeries(layer, plot, data.domain);
        projectSeries(state, state.series);
        if (!plot.marks.length) svgNode('text', { x: left + state.plotWidth / 2, y: top + (bottom - top) / 2, 'text-anchor': 'middle', class: 'resource-trend-empty' }, svg, data.labels.empty);
        else if (previous !== data) {
            const before = state.points.at(-2);
            if (before && previous?.range === data.range && previous.points.at(-1)?.at === before.at && before.at < latest.at) {
                extendSample(state, previousView, plot, before, latest);
            } else if (!previous?.plots[state.kind]?.marks.length || previous.range !== data.range) {
                layer.style.setProperty('--trend-reveal-inset', `${width - left + 4}px`);
                layer.classList.add('resource-trend-reveal');
            }
        }
        state.crosshair = svgNode('g', { visibility: 'hidden' }, svg);
        const selected = state.points.find(p => p.at === state.activeAt);
        if (selected) inspect(state, selected);
        else { state.activeAt = null; state.tooltip.hidden = true; }
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
            const charts = (root.classList.contains('plot-gpu') ? [] : single ? [single] : catalog).map(([kind, keys]) => makeChart(root, kind, keys));
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
            if (root.classList.contains('plot-gpu')) {
                Object.assign(data.labels, data.gpuLabels);
                const keys = ['graphics','utilization','renderer','tiler','copy','encode','decode','dedicated','shared'].filter(key => key in data.plots);
                if (state.data?.adapter !== data.adapter || keys.join() !== state.charts.map(c => c.kind).join()) {
                    root.replaceChildren();
                    state.charts = keys.map(key => makeChart(root, key, [key]));
                }
            }
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
