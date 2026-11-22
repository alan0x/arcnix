// Use the same maximize operation as the rest of the server desktop.
document.addEventListener('loadstart', event => {
    const video = event.target;
    if (video.matches?.('.remote-player-screen video')) {
        video.controls = false;
        video.tabIndex = 0;
        video.disablePictureInPicture = true;
    }
}, true);

// Apply each preference on load and on an explicit preference change. Transport
// controls can still change the volume for just the current video.
(() => {
    const applied = new WeakMap();
    function apply(video) {
        const raw = video.closest('.remote-player')?.querySelector('.player-preferences')?.textContent;
        if (!raw || raw === applied.get(video)?.raw) return;
        const [volume, speed] = JSON.parse(raw);
        const previous = applied.get(video);
        if (volume !== previous?.volume) video.volume = volume / 100;
        if (speed !== previous?.speed) {
            video.defaultPlaybackRate = speed / 100;
            video.playbackRate = speed / 100;
        }
        applied.set(video, {raw, volume, speed});
    }
    document.addEventListener('loadstart', event => {
        if (event.target.matches?.('.remote-player-screen video')) {
            applied.delete(event.target);
            apply(event.target);
        }
    }, true);
    new MutationObserver(records => {
        if (records.some(({target}) => (target.parentElement?.closest('.player-preferences') || target.matches?.('.player-preferences')))) {
            for (const video of document.querySelectorAll('.remote-player-screen video')) apply(video);
        }
    }).observe(document, {subtree:true, childList:true, characterData:true});
})();

// Both renderers reserve the same strip below the picture. The controls read
// HTML media events or libmpv snapshots, and commit seeks only on release.
(() => {
    let active;
    const clock = seconds => {
        const n = Math.floor(Math.max(0, seconds));
        return Math.floor(n / 60) + ':' + String(n % 60).padStart(2, '0');
    };
    function mount(bar) {
        const panel = bar.closest('.remote-player');
        let video, native, labels, lastLabels, lastNative, previousVolume = 100;
        function send(command) {
            if (!bar.isConnected) return;
            const bridge = panel.querySelector('.mpv-command');
            if (native && bridge) {
                bridge.value = JSON.stringify([native.serial, command]);
                bridge.dispatchEvent(new Event('input', {bubbles: true}));
            } else if (video?.isConnected && !video.error) {
                if (command === 'toggle') {
                    if (video.paused || video.ended) video.play().catch(() => {});
                    else video.pause();
                } else if ('seek' in command) video.currentTime = command.seek;
                else if ('volume' in command) video.volume = command.volume / 100;
                else if ('mute' in command) video.muted = command.mute;
            }
        }
        function button(name, click) {
            const button = document.createElement('button');
            button.type = 'button';
            button.className = 'player-' + name;
            button.append(document.createElement('span'));
            button.onclick = click;
            return button;
        }
        function slider(name, step, change) {
            const range = document.createElement('input');
            range.type = 'range';
            range.className = 'player-' + name;
            range.min = '0';
            range.step = step;
            range.onpointerdown = () => { range.dataset.dragging = 'true'; };
            range.onpointerup = range.onpointercancel = range.onblur = () => { delete range.dataset.dragging; };
            range.onchange = () => change(range.valueAsNumber);
            return range;
        }
        function media() {
            if (native) return native.media;
            return {
                loaded: !!video?.readyState && !video.error,
                duration: Number.isFinite(video?.duration) ? video.duration : 0,
                position: video?.currentTime || 0,
                paused: video?.paused ?? true,
                volume: (video?.volume ?? 1) * 100,
                muted: video?.muted ?? false,
            };
        }
        const play = button('play', () => send('toggle'));
        const time = document.createElement('span');
        time.className = 'player-time';
        const seek = slider('seek', '0.1', seek => send({seek}));
        const mute = button('mute', () => {
            const m = media();
            if (m.volume === 0) send({volume: previousVolume});
            send({mute: m.volume > 0 && !m.muted});
        });
        const volume = slider('volume', '1', volume => {
            if (media().muted) send({mute: false});
            send({volume});
        });
        bar.replaceChildren(play, time, seek, mute, volume);

        function render() {
            if (!labels) return;
            const m = media();
            const silent = m.muted || !m.volume;
            if (m.volume > 0) previousVolume = m.volume;
            const [playLabel, pauseLabel, seekLabel, volumeLabel, muteLabel, unmuteLabel] = labels;
            for (const [button, icon, name] of [
                [play, m.paused ? 'play' : 'pause', m.paused ? playLabel : pauseLabel],
                [mute, silent ? 'volume-off' : 'volume', silent ? unmuteLabel : muteLabel],
            ]) {
                button.firstChild.className = 'action-glyph glyph-' + icon;
                button.setAttribute('aria-label', name);
                button.title = name;
                button.disabled = !m.loaded;
            }
            const elapsed = seek.dataset.dragging ? seek.valueAsNumber : m.position;
            const stamp = clock(elapsed) + ' / ' + clock(m.duration);
            if (time.textContent !== stamp) time.textContent = stamp;
            seek.setAttribute('aria-label', seekLabel);
            seek.setAttribute('aria-valuetext', stamp);
            seek.max = m.duration;
            if (!seek.dataset.dragging) seek.value = m.position;
            volume.setAttribute('aria-label', volumeLabel);
            if (!volume.dataset.dragging) volume.value = silent ? 0 : m.volume;
            volume.disabled = !m.loaded;
            seek.disabled = !m.loaded || !m.duration;
        }
        seek.oninput = () => {
            const stamp = clock(seek.valueAsNumber) + ' / ' + clock(media().duration);
            time.textContent = stamp;
            seek.setAttribute('aria-valuetext', stamp);
        };
        return {bar, render, sync() {
            const labelText = panel.querySelector('.player-labels')?.textContent;
            const nativeText = panel.querySelector('.mpv-state')?.textContent;
            const nextVideo = panel.querySelector('.remote-player-screen video');
            if (lastLabels === labelText && lastNative === nativeText && nextVideo === video) return;
            lastLabels = labelText;
            lastNative = nativeText;
            labels = labelText ? JSON.parse(labelText) : null;
            native = nativeText ? JSON.parse(nativeText) : null;
            video = nextVideo;
            render();
        }};
    }
    function sync() {
        const bar = document.querySelector('.remote-player .player-control-bar');
        if (!bar) { active = undefined; return; }
        if (active?.bar !== bar) active = mount(bar);
        active.sync();
    }
    new MutationObserver(sync).observe(document, {subtree: true, childList: true, characterData: true});
    for (const event of ['loadstart','loadedmetadata','loadeddata','durationchange','timeupdate',
        'volumechange','play','pause','ended','seeked','emptied','error']) {
        document.addEventListener(event, e => {
            if (e.target.matches?.('.remote-player-screen video')) active?.render();
        }, true);
    }
})();

(() => {
    const playbackTimers = new WeakMap();
    const pictureClicks = new WeakMap();
    function feedback(video, symbol, settled = '') {
        const indicator = video.closest('.remote-player-screen')?.querySelector('.player-feedback');
        if (!indicator) return;
        clearTimeout(playbackTimers.get(video));
        indicator.dataset.symbol = symbol;
        if (symbol === settled) return;
        playbackTimers.set(video, setTimeout(() => {
            if (video.isConnected) indicator.dataset.symbol = settled;
        }, 420));
    }

    document.addEventListener('click', event => {
        const video = event.target;
        if (!video.matches?.('.remote-player-screen video') || event.button !== 0 || !event.detail || !video.readyState) return;
        // Leave a short interval for a double-click to maximize the window.
        clearTimeout(pictureClicks.get(video));
        if (event.detail !== 1) return;
        pictureClicks.set(video, setTimeout(() => {
            if (!video.isConnected) return;
            if (video.paused) video.play().catch(() => feedback(video, 'play', 'play'));
            else video.pause();
        }, 220));
    }, true);

    document.addEventListener('play', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, 'play');
    }, true);
    document.addEventListener('pause', event => {
        if (event.target.matches?.('.remote-player-screen video') && !event.target.ended) feedback(event.target, 'pause', 'play');
    }, true);
    document.addEventListener('ended', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, 'play', 'play');
    }, true);
    document.addEventListener('loadstart', event => {
        if (event.target.matches?.('.remote-player-screen video')) feedback(event.target, '');
    }, true);
})();

function maximizeVideoWindow(event) {
    if (!event.target.matches?.('.remote-player-screen video')) return;
    event.preventDefault();
    event.stopPropagation();
    event.target.closest('.desktop-window')?.querySelector('.window-maximize')?.click();
}
document.addEventListener('dblclick', maximizeVideoWindow, true);
document.addEventListener('keydown', event => {
    const video = event.target;
    if (!video.matches?.('.remote-player-screen video') || event.repeat || event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.key.toLowerCase() === 'f') maximizeVideoWindow(event);
    else if (event.key === ' ' || event.key.toLowerCase() === 'k') {
        event.preventDefault();
        if (video.paused) video.play().catch(() => {});
        else video.pause();
    }
}, true);
document.addEventListener('contextmenu', event => {
    if (event.target.matches?.('.remote-player-screen video')) event.preventDefault();
}, true);
document.addEventListener('loadedmetadata', event => {
    const video = event.target;
    if (video.matches?.('.remote-player-screen video')) {
        const bridge = video.closest('.remote-player')?.querySelector('.player-opened-input');
        if (bridge) {
            bridge.value = new URL(video.currentSrc).pathname;
            bridge.dispatchEvent(new Event('input', { bubbles: true }));
        }
        video.play().catch(() => {});
    }
}, true);
document.addEventListener('error', event => {
    if (event.target.matches?.('.remote-player-screen video[src]')) {
        event.target.classList.add('playback-failed');
    }
}, true);
